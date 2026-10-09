//! The text area: line numbers, text, selection, caret and the IME composition. Only the lines
//! in view are shaped and painted, as Phase 1 will do.

use std::ops::Range;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use unicode_width::UnicodeWidthChar;

use crate::document::{Document, LineEnding, char_to_byte};
use crate::editor::Editor;
use crate::settings::Settings;
use gpui::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler,
    Entity, Font, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement,
    LayoutId, LineLayout, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, Rgba, ScrollWheelEvent, ShapedLine, SharedString, Style, TextAlign, TextRun,
    UnderlineStyle, Window, fill, point, px, relative, rgb, size,
};

/// Lines are cut here for display. Long lines are a Phase 1 and 2 topic (wrapping, ranges).
pub const MAX_SHAPED_CHARS: usize = 10_000;
/// Sakura's default tab width.
pub const TAB_WIDTH: usize = 4;
/// The space between the gutter and the first column.
pub const TEXT_PADDING: Pixels = px(4.);

const TEXT: u32 = 0x000000;
const BACKGROUND: u32 = 0xffffff;
const GUTTER_BACKGROUND: u32 = 0xf4f4f4;
const GUTTER_BORDER: u32 = 0xc8c8c8;
const LINE_NUMBER: u32 = 0x8a8a8a;
const SELECTION: u32 = 0xb4d5fe;
const CURSOR_LINE: u32 = 0x5a8cd8;
const MARKS: u32 = 0x3fa4a4;
const SCROLLBAR: u32 = 0xb0b0b0;

pub static STARTED: OnceLock<Instant> = OnceLock::new();
pub static FIRST_FRAME: OnceLock<Duration> = OnceLock::new();

/// Font metrics and layout rules for one frame.
pub struct LineStyle {
    font: Font,
    font_size: Pixels,
    pub line_height: Pixels,
    /// The width of a half-width cell: the advance of "0".
    pub cell_width: Pixels,
    grid: bool,
}

impl LineStyle {
    pub fn new(settings: &Settings, window: &Window) -> Self {
        let font = settings.font();
        let font_size = settings.font_size;
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let cell_width = text_system
            .advance(font_id, font_size, '0')
            .map_or(font_size * 0.6, |advance| advance.width);
        Self {
            font,
            font_size,
            line_height: settings.line_height(),
            cell_width,
            grid: settings.grid_layout,
        }
    }

    /// Shapes one line of text. `marked` is the byte range of the IME composition to underline.
    pub fn shape(&self, text: &str, marked: Option<Range<usize>>, window: &Window) -> ShapedLine {
        self.shape_colored(text, rgb(TEXT), marked, window)
    }

    fn shape_colored(
        &self,
        text: &str,
        color: Rgba,
        marked: Option<Range<usize>>,
        window: &Window,
    ) -> ShapedLine {
        let run = |len: usize, underline: bool| TextRun {
            len,
            font: self.font.clone(),
            color: color.into(),
            background_color: None,
            underline: underline.then(|| UnderlineStyle {
                color: Some(color.into()),
                thickness: px(1.),
                wavy: false,
            }),
            strikethrough: None,
        };
        let runs: Vec<TextRun> = match marked {
            Some(marked) if !marked.is_empty() => [
                run(marked.start, false),
                run(marked.len(), true),
                run(text.len() - marked.end, false),
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            _ => vec![run(text.len(), false)],
        };
        let mut line = window.text_system().shape_line(
            SharedString::from(text.to_owned()),
            self.font_size,
            &runs,
            None,
        );
        if self.grid {
            align_to_grid(&mut line, text, self.cell_width, self.font_size);
        }
        line
    }
}

/// Puts every glyph on a grid of half-width cells, as Sakura draws text, so that columns line
/// up even when the full-width glyphs of a font are not exactly twice as wide as its half-width
/// ones. Wide characters (East Asian Width W and F) take two cells, ambiguous ones take two when
/// their glyph is wide, a tab extends to the next tab stop, and the rest take one.
fn align_to_grid(line: &mut ShapedLine, text: &str, cell: Pixels, font_size: Pixels) {
    struct Glyph {
        index: usize,
        x: Pixels,
        /// False for a glyph that belongs to the previous one: a combining mark, or the second
        /// glyph of one character.
        base: bool,
    }

    let char_at = |index: usize| text.get(index..).and_then(|rest| rest.chars().next());
    let mut glyphs: Vec<Glyph> = Vec::new();
    for glyph in line.runs.iter().flat_map(|run| &run.glyphs) {
        let zero_width = char_at(glyph.index).is_some_and(|c| c.width() == Some(0));
        let same_char = glyphs.last().is_some_and(|last| last.index == glyph.index);
        glyphs.push(Glyph {
            index: glyph.index,
            x: glyph.position.x,
            base: !zero_width && !same_char,
        });
    }

    // A character without a base glyph of its own (a combining mark, the rest of an emoji
    // sequence, one that the shaper merged into the previous glyph) takes no cells.
    let mut cells_at = vec![0u8; text.len()];
    let bases: Vec<&Glyph> = glyphs.iter().filter(|glyph| glyph.base).collect();
    for (i, base) in bases.iter().enumerate() {
        let (Some(c), Some(cells)) = (char_at(base.index), cells_at.get_mut(base.index)) else {
            continue;
        };
        let advance = bases.get(i + 1).map_or(line.width, |next| next.x) - base.x;
        *cells = match c.width() {
            Some(2) => 2,
            _ if c.width_cjk() == Some(2) && advance > font_size * 0.75 => 2,
            _ => 1,
        };
    }

    let mut column_at = vec![0usize; text.len() + 1];
    let mut column = 0;
    for (i, c) in text.char_indices() {
        column_at[i] = column;
        column += match c {
            '\t' => TAB_WIDTH - column % TAB_WIDTH,
            _ => cells_at[i] as usize,
        };
    }
    column_at[text.len()] = column;

    let mut runs = line.runs.clone();
    let mut base_shaped_x = px(0.);
    let mut base_x = px(0.);
    let shaped = runs.iter_mut().flat_map(|run| run.glyphs.iter_mut());
    for (glyph, info) in shaped.zip(&glyphs) {
        if info.base {
            base_shaped_x = glyph.position.x;
            base_x = cell * column_at[glyph.index.min(text.len())] as f32;
            glyph.position.x = base_x;
        } else {
            glyph.position.x = base_x + (glyph.position.x - base_shaped_x);
        }
    }
    let layout = LineLayout {
        font_size: line.font_size,
        width: cell * column as f32,
        ascent: line.ascent,
        descent: line.descent,
        runs,
        len: line.len,
    };
    **line = Arc::new(layout);
}

/// Where the lines were drawn in the last frame, for mouse and IME queries.
#[derive(Clone)]
pub struct LayoutSnapshot {
    pub text_bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub scroll: Point<Pixels>,
    pub first_line: usize,
    pub lines: Vec<(ShapedLine, String)>,
}

impl LayoutSnapshot {
    pub fn line_top(&self, line: usize) -> Pixels {
        self.text_bounds.top() + self.line_height * line as f32 - self.scroll.y
    }

    /// The x of column 0.
    pub fn text_left(&self) -> Pixels {
        self.text_bounds.left() + TEXT_PADDING - self.scroll.x
    }

    pub fn shaped(&self, line: usize) -> Option<&(ShapedLine, String)> {
        self.lines.get(line.checked_sub(self.first_line)?)
    }

    /// The number of whole lines in view.
    pub fn page_lines(&self) -> usize {
        ((self.text_bounds.size.height / self.line_height).floor() as usize).max(1)
    }
}

pub struct EditorElement {
    editor: Entity<Editor>,
}

impl EditorElement {
    pub fn new(editor: Entity<Editor>) -> Self {
        Self { editor }
    }
}

pub struct PrepaintState {
    hitbox: Hitbox,
    text_hitbox: Hitbox,
    gutter_bounds: Bounds<Pixels>,
    snapshot: LayoutSnapshot,
    line_origins: Vec<Point<Pixels>>,
    line_numbers: Vec<(Point<Pixels>, ShapedLine)>,
    marks: Vec<(Point<Pixels>, ShapedLine)>,
    selections: Vec<PaintQuad>,
    cursor_line: Option<PaintQuad>,
    caret: Option<PaintQuad>,
    scrollbar: Option<PaintQuad>,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.flex_grow = 1.;
        style.flex_shrink = 1.;
        style.min_size.height = px(0.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let settings = cx.global::<Settings>();
        let style = LineStyle::new(settings, window);
        let show_eol = settings.show_eol;
        let lh = style.line_height;

        let line_count = self.editor.read(cx).doc.line_count();
        let digits = line_count.to_string().len().max(3);
        let gutter_width = style.cell_width * (digits + 2) as f32;
        let gutter_bounds = Bounds::new(bounds.origin, size(gutter_width, bounds.size.height));
        let text_bounds = Bounds::from_corners(
            point(bounds.left() + gutter_width, bounds.top()),
            bounds.bottom_right(),
        );

        let scroll = self.editor.update(cx, |editor, _| {
            editor.update_scroll(&style, text_bounds, window)
        });

        let editor = self.editor.read(cx);
        let doc = &editor.doc;
        let focused = editor.focus_handle.is_focused(window);
        let first_line = ((scroll.y / lh).floor() as usize).min(line_count - 1);
        let last_line =
            (first_line + (text_bounds.size.height / lh).ceil() as usize + 1).min(line_count);
        let text_left = text_bounds.left() + TEXT_PADDING - scroll.x;
        let selection = doc.selection().range();
        let marked = doc.marked_range();
        let (cursor_line, cursor_col) = doc.point(doc.cursor());

        let mut snapshot = LayoutSnapshot {
            text_bounds,
            line_height: lh,
            scroll,
            first_line,
            lines: Vec::with_capacity(last_line - first_line),
        };
        let mut line_origins = Vec::new();
        let mut line_numbers = Vec::new();
        let mut marks = Vec::new();
        let mut selections = Vec::new();
        let mut cursor_line_quad = None;
        let mut caret = None;

        for line in first_line..last_line {
            let text = doc.line_text(line, MAX_SHAPED_CHARS);
            let start = doc.line_start(line);
            let end = start + doc.line_len(line);
            let marked_bytes = marked
                .as_ref()
                .filter(|marked| marked.start < end && marked.end > start)
                .map(|marked| {
                    char_to_byte(&text, marked.start.max(start) - start)
                        ..char_to_byte(&text, marked.end.min(end) - start)
                });
            let shaped = style.shape(&text, marked_bytes, window);
            let top = text_bounds.top() + lh * line as f32 - scroll.y;
            let x_at = |col: usize| text_left + shaped.x_for_index(char_to_byte(&text, col));

            if selection.start <= end && selection.end > start {
                let x0 = x_at(selection.start.max(start) - start);
                let mut x1 = x_at(selection.end.min(end) - start);
                // A selected line break shows as one more cell.
                if selection.end > end {
                    x1 += style.cell_width;
                }
                if x1 > x0 {
                    selections.push(fill(
                        Bounds::from_corners(point(x0, top), point(x1, top + lh)),
                        rgb(SELECTION),
                    ));
                }
            }

            if line == cursor_line {
                cursor_line_quad = Some(fill(
                    Bounds::new(
                        point(text_bounds.left(), top + lh - px(1.)),
                        size(text_bounds.size.width, px(1.)),
                    ),
                    rgb(CURSOR_LINE),
                ));
                if focused {
                    caret = Some(fill(
                        Bounds::new(point(x_at(cursor_col), top), size(px(2.), lh)),
                        rgb(TEXT),
                    ));
                }
            }

            // End-of-line marks as Sakura draws them (sakura_core/view/figures/CFigure_Eol.cpp).
            if show_eol {
                let mark = match doc.line_ending_of(line) {
                    Some(LineEnding::CrLf) => Some("↵"),
                    Some(LineEnding::Cr) => Some("←"),
                    Some(LineEnding::Lf) => Some("↓"),
                    None if line + 1 == line_count => Some("[EOF]"),
                    None => None,
                };
                if let Some(mark) = mark {
                    let shaped_mark = style.shape_colored(mark, rgb(MARKS), None, window);
                    marks.push((point(text_left + shaped.width(), top), shaped_mark));
                }
            }

            let number =
                style.shape_colored(&(line + 1).to_string(), rgb(LINE_NUMBER), None, window);
            let number_x = gutter_bounds.right() - style.cell_width - number.width();
            line_numbers.push((point(number_x, top), number));

            line_origins.push(point(text_left, top));
            snapshot.lines.push((shaped, text));
        }

        let scrollbar = scrollbar_thumb(doc, &style, text_bounds, scroll);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);
        PrepaintState {
            hitbox,
            text_hitbox,
            gutter_bounds,
            snapshot,
            line_origins,
            line_numbers,
            marks,
            selections,
            cursor_line: cursor_line_quad,
            caret,
            scrollbar,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        let text_bounds = state.snapshot.text_bounds;
        let lh = state.snapshot.line_height;
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(text_bounds, self.editor.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &state.text_hitbox);

        window.paint_quad(fill(bounds, rgb(BACKGROUND)));
        window.paint_quad(fill(state.gutter_bounds, rgb(GUTTER_BACKGROUND)));
        window.paint_quad(fill(
            Bounds::new(
                point(state.gutter_bounds.right() - px(1.), bounds.top()),
                size(px(1.), bounds.size.height),
            ),
            rgb(GUTTER_BORDER),
        ));

        window.with_content_mask(
            Some(ContentMask {
                bounds: text_bounds,
            }),
            |window| {
                if let Some(quad) = state.cursor_line.take() {
                    window.paint_quad(quad);
                }
                for quad in state.selections.drain(..) {
                    window.paint_quad(quad);
                }
                for ((line, _), origin) in state.snapshot.lines.iter().zip(&state.line_origins) {
                    line.paint(*origin, lh, TextAlign::Left, None, window, cx)
                        .ok();
                }
                for (origin, mark) in &state.marks {
                    mark.paint(*origin, lh, TextAlign::Left, None, window, cx)
                        .ok();
                }
                if let Some(caret) = state.caret.take() {
                    window.paint_quad(caret);
                }
                if let Some(thumb) = state.scrollbar.take() {
                    window.paint_quad(thumb);
                }
            },
        );
        window.with_content_mask(
            Some(ContentMask {
                bounds: state.gutter_bounds,
            }),
            |window| {
                for (origin, number) in &state.line_numbers {
                    number
                        .paint(*origin, lh, TextAlign::Left, None, window, cx)
                        .ok();
                }
            },
        );

        self.register_mouse_listeners(&state.hitbox, window);
        let snapshot = state.snapshot.clone();
        self.editor
            .update(cx, |editor, _| editor.layout = Some(snapshot));

        if let Some(started) = STARTED.get() {
            FIRST_FRAME.get_or_init(|| {
                let elapsed = started.elapsed();
                eprintln!(
                    "ume-poc-gpui: first frame painted {:.1} ms after start",
                    elapsed.as_secs_f64() * 1000.
                );
                elapsed
            });
        }
    }
}

impl EditorElement {
    fn register_mouse_listeners(&self, hitbox: &Hitbox, window: &mut Window) {
        window.on_mouse_event({
            let editor = self.editor.clone();
            let hitbox = hitbox.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == MouseButton::Left
                    && hitbox.is_hovered(window)
                {
                    editor.update(cx, |editor, cx| editor.mouse_down(event, window, cx));
                    cx.stop_propagation();
                }
            }
        });
        window.on_mouse_event({
            let editor = self.editor.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && event.pressed_button == Some(MouseButton::Left)
                {
                    editor.update(cx, |editor, cx| {
                        editor.mouse_drag(event.position, window, cx)
                    });
                }
            }
        });
        window.on_mouse_event({
            let editor = self.editor.clone();
            move |event: &MouseUpEvent, phase, _window, cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                    editor.update(cx, |editor, _| editor.mouse_up());
                }
            }
        });
        window.on_mouse_event({
            let editor = self.editor.clone();
            let hitbox = hitbox.clone();
            move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                    editor.update(cx, |editor, cx| editor.scroll_wheel(event, cx));
                    cx.stop_propagation();
                }
            }
        });
    }
}

/// A thumb that shows where the view is in the document. It does not take the mouse.
fn scrollbar_thumb(
    doc: &Document,
    style: &LineStyle,
    text_bounds: Bounds<Pixels>,
    scroll: Point<Pixels>,
) -> Option<PaintQuad> {
    let track = text_bounds.size.height;
    let max_scroll = style.line_height * (doc.line_count() - 1) as f32;
    if max_scroll <= px(0.) {
        return None;
    }
    let content = max_scroll + track;
    let thumb = (track * (track / content)).max(px(24.)).min(track);
    let top = text_bounds.top() + (track - thumb) * (scroll.y / max_scroll);
    Some(
        fill(
            Bounds::new(
                point(text_bounds.right() - px(8.), top),
                size(px(6.), thumb),
            ),
            rgb(SCROLLBAR),
        )
        .corner_radii(px(3.)),
    )
}
