//! Throwaway PoC for ADR-0004 (see docs/poc/ui-framework.md).
//!
//! A self-drawn text view that shapes and paints only the visible lines,
//! with IME support, a Sakura-style menu bar, and file opening.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, ExternalPaths, FocusHandle, Focusable, GlobalElementId, KeyBinding,
    KeyDownEvent, LayoutId, Menu, MenuItem, ModifiersChangedEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathPromptOptions, Pixels, Point, ScrollWheelEvent, ShapedLine,
    SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, WindowBounds,
    WindowOptions, actions, div, fill, point, prelude::*, px, relative, rgb, size,
};
use gpui_component::Root;
use gpui_platform::application;

mod memstat;
mod menu_bar;
use menu_bar::MenuBar;

#[global_allocator]
static ALLOC: memstat::CountingAlloc = memstat::CountingAlloc;

static START: OnceLock<Instant> = OnceLock::new();

actions!(
    editor,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        Home,
        End,
        PageUp,
        PageDown,
        DocStart,
        DocEnd,
        Enter,
        Tab,
        Paste,
        CopyLine,
        OpenFile,
        GenerateLargeText,
        CycleFont,
        FontBigger,
        FontSmaller,
        Quit,
        NoOp,
    ]
);

const FONTS: &[&str] = &["BIZ UDGothic", "MS Gothic", "Consolas", "Yu Gothic UI"];

struct Editor {
    focus_handle: FocusHandle,
    lines: Vec<String>,
    /// Cursor as (line index, byte offset within the line).
    cursor: (usize, usize),
    /// Desired x position kept across vertical moves.
    goal_x: Option<Pixels>,
    /// IME composition range within the cursor line (byte offsets).
    marked: Option<Range<usize>>,
    scroll_y: Pixels,
    font_index: usize,
    font_size: f32,
    file_name: Option<String>,
    first_frame_ms: Option<f64>,
    layout: Option<LayoutCache>,
    /// Scrollbar thumb drag: (mouse y at start, scroll_y at start).
    scroll_drag: Option<(Pixels, Pixels)>,
}

/// What the last paint produced, used for hit testing, IME placement, and scrolling.
struct LayoutCache {
    text_origin: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    first_line: usize,
    shaped: Vec<ShapedLine>,
}

impl Editor {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            lines: vec![
                "GPUI テキスト表示の試作です。日本語を入力してみてください。".into(),
                "ファイル > 開く、またはファイルをウィンドウにドロップすると開けます。".into(),
                "ファイル > 100万行のテキストを作る で、スクロールを試せます。".into(),
                String::new(),
            ],
            cursor: (3, 0),
            goal_x: None,
            marked: None,
            scroll_y: px(0.),
            font_index: 0,
            font_size: 14.,
            file_name: None,
            first_frame_ms: None,
            layout: None,
            scroll_drag: None,
        }
    }

    fn line(&self) -> &str {
        &self.lines[self.cursor.0]
    }

    fn set_text(&mut self, text: &str, name: Option<String>, cx: &mut Context<Self>) {
        self.lines = text
            .split('\n')
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect();
        self.cursor = (0, 0);
        self.marked = None;
        self.scroll_y = px(0.);
        self.file_name = name;
        cx.notify();
    }

    fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                self.set_text(&text, name, cx);
            }
            Err(err) => self.set_text(&format!("開けませんでした: {err}"), None, cx),
        }
    }

    fn prev_boundary(&self) -> Option<(usize, usize)> {
        let (row, col) = self.cursor;
        if col > 0 {
            let prev = self.line()[..col]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
            Some((row, prev))
        } else if row > 0 {
            Some((row - 1, self.lines[row - 1].len()))
        } else {
            None
        }
    }

    fn next_boundary(&self) -> Option<(usize, usize)> {
        let (row, col) = self.cursor;
        if let Some(ch) = self.line()[col..].chars().next() {
            Some((row, col + ch.len_utf8()))
        } else if row + 1 < self.lines.len() {
            Some((row + 1, 0))
        } else {
            None
        }
    }

    fn move_to(&mut self, pos: (usize, usize), cx: &mut Context<Self>) {
        self.cursor = pos;
        self.goal_x = None;
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let (row, col) = self.cursor;
        let mut parts = text.split('\n');
        let first = parts.next().unwrap_or_default().trim_end_matches('\r');
        let rest: Vec<String> = parts
            .map(|p| p.trim_end_matches('\r').to_string())
            .collect();
        if rest.is_empty() {
            self.lines[row].insert_str(col, first);
            self.cursor = (row, col + first.len());
        } else {
            let tail = self.lines[row].split_off(col);
            self.lines[row].push_str(first);
            let last_row = row + rest.len();
            let last_len = rest.last().map_or(0, String::len);
            for (i, l) in rest.into_iter().enumerate() {
                self.lines.insert(row + 1 + i, l);
            }
            self.lines[last_row].push_str(&tail);
            self.cursor = (last_row, last_len);
        }
        self.goal_x = None;
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn delete_range(&mut self, from: (usize, usize), to: (usize, usize)) {
        if from.0 == to.0 {
            self.lines[from.0].replace_range(from.1..to.1, "");
        } else {
            let tail = self.lines[to.0][to.1..].to_string();
            self.lines[from.0].truncate(from.1);
            self.lines[from.0].push_str(&tail);
            self.lines.drain(from.0 + 1..=to.0);
        }
        self.cursor = from;
    }

    fn visible_rows(&self) -> usize {
        self.layout
            .as_ref()
            .map_or(30, |l| {
                (l.bounds.size.height / l.line_height).floor() as usize
            })
            .max(1)
    }

    fn line_height(&self) -> Pixels {
        self.layout.as_ref().map_or(px(20.), |l| l.line_height)
    }

    fn ensure_cursor_visible(&mut self) {
        let lh = self.line_height();
        let top = lh * self.cursor.0 as f32;
        let rows = self.visible_rows() as f32;
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if top + lh > self.scroll_y + lh * rows {
            self.scroll_y = top + lh - lh * rows;
        }
    }

    fn clamp_scroll(&mut self) {
        let max = match self.layout.as_ref() {
            Some(l) => max_scroll(l.bounds.size.height, l.line_height, self.lines.len()),
            None => px(0.),
        };
        self.scroll_y = self.scroll_y.clamp(px(0.), max);
    }

    fn x_for(&self, row: usize, col: usize) -> Option<Pixels> {
        let l = self.layout.as_ref()?;
        let shaped = l.shaped.get(row.checked_sub(l.first_line)?)?;
        Some(shaped.x_for_index(col))
    }

    fn col_for_x(&self, row: usize, x: Pixels) -> usize {
        let fallback = self.lines[row].len().min(self.cursor.1);
        let Some(l) = self.layout.as_ref() else {
            return fallback;
        };
        match row.checked_sub(l.first_line).and_then(|i| l.shaped.get(i)) {
            Some(shaped) => shaped.closest_index_for_x(x).min(self.lines[row].len()),
            None => fallback,
        }
    }

    fn vertical(&mut self, delta: isize, cx: &mut Context<Self>) {
        let goal = self
            .goal_x
            .or_else(|| self.x_for(self.cursor.0, self.cursor.1));
        let row = (self.cursor.0 as isize + delta).clamp(0, self.lines.len() as isize - 1) as usize;
        self.scroll_y += self.line_height() * (row as f32 - self.cursor.0 as f32);
        self.clamp_scroll();
        self.cursor.0 = row;
        // The target row may not be shaped yet; fall back to the byte column.
        let col = match goal {
            Some(x) if self.x_for(row, 0).is_some() => self.col_for_x(row, x),
            _ => self.cursor.1.min(self.lines[row].len()),
        };
        let col = floor_char_boundary(&self.lines[row], col);
        self.cursor.1 = col;
        self.goal_x = goal;
        self.ensure_cursor_visible();
        cx.notify();
    }

    // --- actions -------------------------------------------------------

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.prev_boundary() {
            self.move_to(p, cx);
        }
    }
    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.next_boundary() {
            self.move_to(p, cx);
        }
    }
    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, cx);
    }
    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, cx);
    }
    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-(self.visible_rows() as isize), cx);
    }
    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(self.visible_rows() as isize, cx);
    }
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to((self.cursor.0, 0), cx);
    }
    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to((self.cursor.0, self.line().len()), cx);
    }
    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to((0, 0), cx);
    }
    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.lines.len() - 1;
        self.move_to((last, self.lines[last].len()), cx);
    }
    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.prev_boundary() {
            self.delete_range(p, self.cursor);
            self.goal_x = None;
            self.ensure_cursor_visible();
            cx.notify();
        }
    }
    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.next_boundary() {
            self.delete_range(self.cursor, p);
            cx.notify();
        }
    }
    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.insert("\n", cx);
    }
    fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        self.insert("\t", cx);
    }
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(&text, cx);
        }
    }
    fn copy_line(&mut self, _: &CopyLine, _: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.line().to_string()));
    }
    fn open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("開く".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await
                && let Some(path) = paths.pop()
            {
                this.update(cx, |this, cx| this.open_path(path, cx)).ok();
            }
        })
        .detach();
    }
    fn generate(&mut self, _: &GenerateLargeText, _: &mut Window, cx: &mut Context<Self>) {
        let started = Instant::now();
        self.lines = (1..=1_000_000)
            .map(|i| format!("{i:07}: いろはにほへと ちりぬるを わかよたれそ The quick brown fox jumps over the lazy dog."))
            .collect();
        self.cursor = (0, 0);
        self.marked = None;
        self.scroll_y = px(0.);
        self.file_name = Some(format!(
            "100万行 (生成 {} ms)",
            started.elapsed().as_millis()
        ));
        cx.notify();
    }
    fn cycle_font(&mut self, _: &CycleFont, _: &mut Window, cx: &mut Context<Self>) {
        self.font_index = (self.font_index + 1) % FONTS.len();
        cx.notify();
    }
    fn font_bigger(&mut self, _: &FontBigger, _: &mut Window, cx: &mut Context<Self>) {
        self.font_size = (self.font_size + 1.).min(48.);
        cx.notify();
    }
    fn font_smaller(&mut self, _: &FontSmaller, _: &mut Window, cx: &mut Context<Self>) {
        self.font_size = (self.font_size - 1.).max(8.);
        cx.notify();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(l) = self.layout.as_ref() else {
            return;
        };
        let bar = ScrollbarGeometry::new(l.bounds, l.line_height, self.lines.len(), self.scroll_y);
        if bar.track.contains(&event.position) {
            if bar.thumb.contains(&event.position) {
                self.scroll_drag = Some((event.position.y, self.scroll_y));
            } else {
                // Clicking the track pages up or down, as on Windows.
                let page = l.bounds.size.height - l.line_height;
                if event.position.y < bar.thumb.top() {
                    self.scroll_y -= page;
                } else {
                    self.scroll_y += page;
                }
                self.clamp_scroll();
            }
            cx.notify();
            return;
        }
        let rel_y = event.position.y - l.text_origin.y;
        let row = ((rel_y + self.scroll_y) / l.line_height).floor().max(0.) as usize;
        let row = row.min(self.lines.len() - 1);
        let col = self.col_for_x(row, event.position.x - l.text_origin.x);
        self.move_to((row, col), cx);
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some((start_y, start_scroll)), Some(l)) = (self.scroll_drag, self.layout.as_ref())
        else {
            return;
        };
        let bar = ScrollbarGeometry::new(l.bounds, l.line_height, self.lines.len(), self.scroll_y);
        let travel = bar.track.size.height - bar.thumb.size.height;
        if travel > px(0.) {
            let max = max_scroll(l.bounds.size.height, l.line_height, self.lines.len());
            self.scroll_y = start_scroll + max * ((event.position.y - start_y) / travel);
            self.clamp_scroll();
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll_drag.take().is_some() {
            cx.notify();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(self.line_height());
        self.scroll_y -= delta.y;
        self.clamp_scroll();
        cx.notify();
    }

    fn on_drop(&mut self, paths: &ExternalPaths, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = paths.paths().first() {
            self.open_path(path.clone(), cx);
        }
    }

    // --- UTF-16 helpers for the IME (offsets are within the cursor line) --

    fn to_utf16(&self, byte: usize) -> usize {
        self.line()[..byte].encode_utf16().count()
    }

    fn byte_from_utf16(&self, units: usize) -> usize {
        let mut count = 0;
        for (i, ch) in self.line().char_indices() {
            if count >= units {
                return i;
            }
            count += ch.len_utf16();
        }
        self.line().len()
    }

    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.byte_from_utf16(r.start)..self.byte_from_utf16(r.end)
    }
}

fn scrollbar_width() -> Pixels {
    px(14.)
}

/// The largest scroll offset: the last line may scroll up to the top of the view.
fn max_scroll(view_height: Pixels, line_height: Pixels, lines: usize) -> Pixels {
    let rows = (view_height / line_height).floor().max(1.);
    (line_height * lines as f32 - line_height * (rows - 1.)).max(px(0.))
}

/// Vertical scrollbar placement, shared by painting and hit testing.
struct ScrollbarGeometry {
    track: Bounds<Pixels>,
    thumb: Bounds<Pixels>,
}

impl ScrollbarGeometry {
    fn new(bounds: Bounds<Pixels>, line_height: Pixels, lines: usize, scroll_y: Pixels) -> Self {
        let track = Bounds::new(
            point(bounds.right() - scrollbar_width(), bounds.top()),
            size(scrollbar_width(), bounds.size.height),
        );
        let view = bounds.size.height;
        let max = max_scroll(view, line_height, lines);
        let thumb_h = (track.size.height * (view / (max + view)))
            .max(px(24.))
            .min(track.size.height);
        let ratio = if max > px(0.) { scroll_y / max } else { 0. };
        let thumb_top = track.top() + (track.size.height - thumb_h) * ratio;
        let thumb = Bounds::new(
            point(track.left() + px(3.), thumb_top),
            size(scrollbar_width() - px(6.), thumb_h),
        );
        Self { track, thumb }
    }
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.to_utf16(range.start)..self.to_utf16(range.end));
        Some(self.line()[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let c = self.to_utf16(self.cursor.1);
        Some(UTF16Selection {
            range: c..c,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|r| self.to_utf16(r.start)..self.to_utf16(r.end))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked.clone())
            .unwrap_or(self.cursor.1..self.cursor.1);
        let row = self.cursor.0;
        self.lines[row].replace_range(range.clone(), "");
        self.cursor.1 = range.start;
        self.marked = None;
        self.insert(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked.clone())
            .unwrap_or(self.cursor.1..self.cursor.1);
        // Composition text never contains a newline, so it stays within the line.
        let text = text.replace(['\r', '\n'], "");
        let row = self.cursor.0;
        self.lines[row].replace_range(range.clone(), &text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        let caret = match new_selected_range_utf16 {
            Some(sel) => {
                let units: usize = text.encode_utf16().count().min(sel.end);
                let mut byte = text.len();
                let mut count = 0;
                for (i, ch) in text.char_indices() {
                    if count >= units {
                        byte = i;
                        break;
                    }
                    count += ch.len_utf16();
                }
                range.start + byte
            }
            None => range.start + text.len(),
        };
        self.cursor.1 = caret;
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let l = self.layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let row = self.cursor.0;
        let shaped = l.shaped.get(row.checked_sub(l.first_line)?)?;
        let y = l.text_origin.y + l.line_height * row as f32 - self.scroll_y;
        Some(Bounds::from_corners(
            point(l.text_origin.x + shaped.x_for_index(range.start), y),
            point(
                l.text_origin.x + shaped.x_for_index(range.end),
                y + l.line_height,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let l = self.layout.as_ref()?;
        let shaped = l.shaped.get(self.cursor.0.checked_sub(l.first_line)?)?;
        let byte = shaped.closest_index_for_x(p.x - l.text_origin.x);
        Some(self.to_utf16(byte.min(self.line().len())))
    }
}

/// The custom element: shapes and paints only the lines in view.
struct EditorElement {
    editor: Entity<Editor>,
}

struct Prepaint {
    gutter: Vec<ShapedLine>,
    text_origin: Point<Pixels>,
    line_height: Pixels,
    first_line: usize,
    y_offset: Pixels,
    shaped: Vec<ShapedLine>,
    gutter_width: Pixels,
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let editor = self.editor.read(cx);
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let font = style.font();

        let first_line = (editor.scroll_y / line_height).floor().max(0.) as usize;
        let y_offset = -(editor.scroll_y - line_height * first_line as f32);
        let rows = (bounds.size.height / line_height).ceil() as usize + 1;
        let last_line = (first_line + rows).min(editor.lines.len());

        let digits = editor.lines.len().to_string().len().max(3);
        let gutter_width = window
            .text_system()
            .shape_line(
                "0".repeat(digits + 1).into(),
                font_size,
                &[run(&font, digits + 1, rgb(0))],
                None,
            )
            .width();

        let mut gutter = Vec::new();
        let mut shaped = Vec::new();
        for row in first_line..last_line {
            let num: SharedString = format!("{:>width$}", row + 1, width = digits).into();
            gutter.push(window.text_system().shape_line(
                num.clone(),
                font_size,
                &[run(&font, num.len(), rgb(0x8a8a8a))],
                None,
            ));
            let text = &editor.lines[row];
            let display: SharedString = text.replace('\t', "    ").into();
            // Keep byte offsets aligned with the buffer: only tabs are expanded,
            // so shape the raw text when it has no tab.
            let display: SharedString = if text.contains('\t') {
                display
            } else {
                text.clone().into()
            };
            let mut runs = vec![run(&font, display.len(), rgb(0x1e1e1e))];
            if row == editor.cursor.0
                && let Some(m) = editor.marked.clone()
                && !text.contains('\t')
            {
                runs = vec![
                    run(&font, m.start, rgb(0x1e1e1e)),
                    TextRun {
                        underline: Some(UnderlineStyle {
                            color: Some(rgb(0x1e1e1e).into()),
                            thickness: px(1.),
                            wavy: false,
                        }),
                        ..run(&font, m.end - m.start, rgb(0x1e1e1e))
                    },
                    run(&font, display.len() - m.end, rgb(0x1e1e1e)),
                ]
                .into_iter()
                .filter(|r| r.len > 0)
                .collect();
            }
            shaped.push(
                window
                    .text_system()
                    .shape_line(display, font_size, &runs, None),
            );
        }

        let text_origin = point(bounds.left() + gutter_width + px(8.), bounds.top());
        Prepaint {
            gutter,
            text_origin,
            line_height,
            first_line,
            y_offset,
            shaped,
            gutter_width,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        p: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        window.paint_quad(fill(bounds, rgb(0xffffff)));
        window.paint_quad(fill(
            Bounds::new(
                bounds.origin,
                size(p.gutter_width + px(4.), bounds.size.height),
            ),
            rgb(0xf0f0f0),
        ));

        let editor = self.editor.read(cx);
        let (cursor_row, cursor_col) = editor.cursor;
        let bar =
            ScrollbarGeometry::new(bounds, p.line_height, editor.lines.len(), editor.scroll_y);
        let dragging = editor.scroll_drag.is_some();
        let text_bounds = Bounds::new(
            bounds.origin,
            size(bounds.size.width - scrollbar_width(), bounds.size.height),
        );
        window.with_content_mask(
            Some(gpui::ContentMask {
                bounds: text_bounds,
            }),
            |window| {
                for (i, (num, line)) in p.gutter.iter().zip(&p.shaped).enumerate() {
                    let y = bounds.top() + p.y_offset + p.line_height * i as f32;
                    num.paint(
                        point(bounds.left(), y),
                        p.line_height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
                    line.paint(
                        point(p.text_origin.x, y),
                        p.line_height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
                    if p.first_line + i == cursor_row && focus_handle.is_focused(window) {
                        let x = p.text_origin.x + line.x_for_index(cursor_col);
                        window.paint_quad(fill(
                            Bounds::new(point(x, y), size(px(2.), p.line_height)),
                            rgb(0x0050c8),
                        ));
                    }
                }
            },
        );

        window.paint_quad(fill(bar.track, rgb(0xf3f3f3)));
        window.paint_quad(
            fill(
                bar.thumb,
                if dragging {
                    rgb(0x8c8c8c)
                } else {
                    rgb(0xc2c2c2)
                },
            )
            .corner_radii(px(4.)),
        );

        let shaped = std::mem::take(&mut p.shaped);
        let text_origin = point(p.text_origin.x, p.text_origin.y);
        let (line_height, first_line) = (p.line_height, p.first_line);
        self.editor.update(cx, |editor, _| {
            if editor.first_frame_ms.is_none()
                && let Some(start) = START.get()
            {
                let ms = start.elapsed().as_secs_f64() * 1000.;
                eprintln!("first frame: {ms:.1} ms");
                editor.first_frame_ms = Some(ms);
            }
            editor.layout = Some(LayoutCache {
                text_origin,
                bounds,
                line_height,
                first_line,
                shaped,
            });
        });
    }
}

fn run(font: &gpui::Font, len: usize, color: gpui::Rgba) -> TextRun {
    TextRun {
        len,
        font: font.clone(),
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = format!(
            "{} 行 {} 桁 | 全 {} 行 | {} {}pt | 最初の描画まで {} | {}",
            self.cursor.0 + 1,
            self.line()[..self.cursor.1].chars().count() + 1,
            self.lines.len(),
            FONTS[self.font_index],
            self.font_size,
            self.first_frame_ms
                .map_or("-".into(), |ms| format!("{ms:.0} ms")),
            self.file_name.as_deref().unwrap_or("(無題)"),
        );
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .key_context("Editor")
                    .track_focus(&self.focus_handle)
                    .cursor(CursorStyle::IBeam)
                    .font_family(FONTS[self.font_index])
                    .text_size(px(self.font_size))
                    .line_height(px((self.font_size * 1.5).round()))
                    .on_action(cx.listener(Self::left))
                    .on_action(cx.listener(Self::right))
                    .on_action(cx.listener(Self::up))
                    .on_action(cx.listener(Self::down))
                    .on_action(cx.listener(Self::page_up))
                    .on_action(cx.listener(Self::page_down))
                    .on_action(cx.listener(Self::home))
                    .on_action(cx.listener(Self::end))
                    .on_action(cx.listener(Self::doc_start))
                    .on_action(cx.listener(Self::doc_end))
                    .on_action(cx.listener(Self::backspace))
                    .on_action(cx.listener(Self::delete))
                    .on_action(cx.listener(Self::enter))
                    .on_action(cx.listener(Self::tab))
                    .on_action(cx.listener(Self::paste))
                    .on_action(cx.listener(Self::copy_line))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_scroll_wheel(cx.listener(Self::on_scroll))
                    .on_drop(cx.listener(Self::on_drop))
                    .children((!flag("POC_NO_TEXT")).then(|| EditorElement {
                        editor: cx.entity(),
                    })),
            )
            .child(
                div()
                    .px_2()
                    .py_0p5()
                    .text_size(px(12.))
                    .bg(rgb(0xe8e8e8))
                    .text_color(rgb(0x333333))
                    .child(status),
            )
    }
}

/// Window content: menu bar on top, editor below. Window-level actions live here
/// so that they work from the menu bar as well as from the editor.
struct Workspace {
    menu_bar: Entity<MenuBar>,
    editor: Entity<Editor>,
    /// Alt went down with no other key or modifier; releasing it toggles the menu bar.
    alt_pending: bool,
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_font = gpui_component::Theme::global(cx).font_family.clone();
        let editor = self.editor.clone();
        let forward = move |f: fn(&mut Editor, &mut Window, &mut Context<Editor>)| {
            let editor = editor.clone();
            move |window: &mut Window, cx: &mut App| editor.update(cx, |e, cx| f(e, window, cx))
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x1e1e1e))
            .font_family(ui_font)
            .on_modifiers_changed(
                cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                    let m = event.modifiers;
                    let alt_only = m.alt && !m.control && !m.shift && !m.platform && !m.function;
                    if alt_only {
                        this.alt_pending = true;
                    } else if !m.modified() && this.alt_pending {
                        this.alt_pending = false;
                        this.menu_bar
                            .update(cx, |bar, cx| bar.toggle_armed(window, cx));
                    } else {
                        this.alt_pending = false;
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.alt_pending = false;
                let m = event.keystroke.modifiers;
                if m.alt && !m.control && !m.shift && !m.platform {
                    let key = event.keystroke.key.clone();
                    let opened = this
                        .menu_bar
                        .update(cx, |bar, cx| bar.open_by_mnemonic(&key, window, cx));
                    if opened {
                        cx.stop_propagation();
                    }
                }
            }))
            .on_action({
                let f = forward(|e, w, cx| e.open_file(&OpenFile, w, cx));
                move |_: &OpenFile, w, cx| f(w, cx)
            })
            .on_action({
                let f = forward(|e, w, cx| e.generate(&GenerateLargeText, w, cx));
                move |_: &GenerateLargeText, w, cx| f(w, cx)
            })
            .on_action({
                let f = forward(|e, w, cx| e.cycle_font(&CycleFont, w, cx));
                move |_: &CycleFont, w, cx| f(w, cx)
            })
            .on_action({
                let f = forward(|e, w, cx| e.font_bigger(&FontBigger, w, cx));
                move |_: &FontBigger, w, cx| f(w, cx)
            })
            .on_action({
                let f = forward(|e, w, cx| e.font_smaller(&FontSmaller, w, cx));
                move |_: &FontSmaller, w, cx| f(w, cx)
            })
            .on_action({
                let f = forward(|e, w, cx| e.paste(&Paste, w, cx));
                move |_: &Paste, w, cx| f(w, cx)
            })
            .on_action({
                let f = forward(|e, w, cx| e.copy_line(&CopyLine, w, cx));
                move |_: &CopyLine, w, cx| f(w, cx)
            })
            .children((!flag("POC_NO_MENU")).then(|| {
                div()
                    .border_b_1()
                    .border_color(rgb(0xd0d0d0))
                    .child(self.menu_bar.clone())
            }))
            .child(div().flex_1().overflow_hidden().child(self.editor.clone()))
    }
}

/// Measurement toggles for the PoC (set the environment variable to disable a part).
fn flag(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

fn set_menus(cx: &mut App) {
    // Sakura's top-level menus: File / Edit / Convert / Search / Tools / Settings / Window / Help.
    let todo = || MenuItem::action("(試作では未実装)", NoOp).disabled(true);
    cx.set_menus([
        Menu::new("ファイル(F)").items([
            MenuItem::action("開く(O)...", OpenFile),
            MenuItem::action("100万行のテキストを作る", GenerateLargeText),
            MenuItem::separator(),
            MenuItem::action("終了(X)", Quit),
        ]),
        Menu::new("編集(E)").items([
            MenuItem::action("貼り付け(P)", Paste),
            MenuItem::action("行をコピー", CopyLine),
        ]),
        Menu::new("変換(C)").items([todo()]),
        Menu::new("検索(S)").items([todo()]),
        Menu::new("ツール(T)").items([todo()]),
        Menu::new("設定(O)").items([
            MenuItem::action("フォントを切り替える", CycleFont),
            MenuItem::action("文字を大きく", FontBigger),
            MenuItem::action("文字を小さく", FontSmaller),
        ]),
        Menu::new("ウィンドウ(W)").items([todo()]),
        Menu::new("ヘルプ(H)").items([todo()]),
    ]);
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", Left, Some("Editor")),
        KeyBinding::new("right", Right, Some("Editor")),
        KeyBinding::new("up", Up, Some("Editor")),
        KeyBinding::new("down", Down, Some("Editor")),
        KeyBinding::new("pageup", PageUp, Some("Editor")),
        KeyBinding::new("pagedown", PageDown, Some("Editor")),
        KeyBinding::new("home", Home, Some("Editor")),
        KeyBinding::new("end", End, Some("Editor")),
        KeyBinding::new("secondary-home", DocStart, Some("Editor")),
        KeyBinding::new("secondary-end", DocEnd, Some("Editor")),
        KeyBinding::new("backspace", Backspace, Some("Editor")),
        KeyBinding::new("delete", Delete, Some("Editor")),
        KeyBinding::new("enter", Enter, Some("Editor")),
        KeyBinding::new("tab", Tab, Some("Editor")),
        KeyBinding::new("secondary-v", Paste, None),
        KeyBinding::new("secondary-c", CopyLine, None),
        KeyBinding::new("secondary-o", OpenFile, None),
        KeyBinding::new("secondary-=", FontBigger, None),
        KeyBinding::new("secondary--", FontSmaller, None),
        KeyBinding::new("f7", CycleFont, None),
        KeyBinding::new("f8", GenerateLargeText, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
}

fn main() {
    START.get_or_init(Instant::now);
    memstat::start_from_env();
    // RUST_LOG=info (or ZED_LOG) prints GPUI's logs to stderr.
    if std::env::var_os("RUST_LOG").is_some() || std::env::var_os("ZED_LOG").is_some() {
        zlog::init();
        zlog::init_output_stderr();
    }
    application().run(|cx: &mut App| {
        gpui_component::init(cx);
        if std::env::var("POC_TEXT_MODE").as_deref() == Ok("grayscale") {
            cx.set_text_rendering_mode(gpui::TextRenderingMode::Grayscale);
        }
        menu_bar::init(cx);
        if let Ok(font) = std::env::var("POC_UI_FONT") {
            let theme = gpui_component::Theme::global_mut(cx);
            theme.font_family = font.clone().into();
            theme.mono_font_family = font.into();
        }
        bind_keys(cx);
        set_menus(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &NoOp, _| {});
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("ume_editor GPUI PoC".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let build = |window: &mut Window, cx: &mut App| {
            let editor = cx.new(Editor::new);
            window.focus(&editor.focus_handle(cx), cx);
            let menu_bar = MenuBar::new(cx.get_menus().unwrap_or_default(), cx);
            cx.new(|_| Workspace {
                menu_bar,
                editor,
                alt_pending: false,
            })
        };
        if flag("POC_NO_ROOT") {
            cx.open_window(options, build)
                .expect("failed to open window");
        } else {
            cx.open_window(options, |window, cx| {
                let workspace = build(window, cx);
                cx.new(|cx| Root::new(workspace, window, cx))
            })
            .expect("failed to open window");
        }
        cx.activate(true);
    });
}
