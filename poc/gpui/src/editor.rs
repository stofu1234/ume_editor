//! The editor view: one document in one window, as Sakura opens one window per file.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, ExternalPaths, FocusHandle, Focusable,
    MouseDownEvent, Pixels, Point, PromptLevel, Render, ScrollWheelEvent, SharedString, Task,
    TitlebarOptions, UTF16Selection, Window, WindowBounds, WindowHandle, WindowOptions, div, point,
    prelude::*, px, rgb, size,
};
use unicode_width::UnicodeWidthChar;

use crate::actions::*;
use crate::document::{Document, Selection, byte_to_char, char_to_byte};
use crate::element::{
    EditorElement, LayoutSnapshot, LineStyle, MAX_SHAPED_CHARS, TAB_WIDTH, TEXT_PADDING,
};
use crate::settings::Settings;

const APP_TITLE: &str = "ume_editor UI 試作";

pub struct Editor {
    pub focus_handle: FocusHandle,
    pub doc: Document,
    path: Option<PathBuf>,
    has_bom: bool,
    scroll: Point<Pixels>,
    /// Scroll the caret into view on the next frame.
    autoscroll: bool,
    /// The x that Up and Down keep, so that the caret returns to its column after a short line.
    goal_x: Option<Pixels>,
    selecting: bool,
    /// Where the lines were drawn in the last frame.
    pub layout: Option<LayoutSnapshot>,
    message: Option<SharedString>,
    title: String,
    close_confirmed: bool,
}

/// Opens an empty editor window. Windows cascade like Sakura's.
pub fn open_window(cx: &mut App) -> Option<WindowHandle<Editor>> {
    let offset = px(24.) * (cx.windows().len() % 8);
    let mut bounds = Bounds::centered(None, size(px(960.), px(720.)), cx);
    bounds.origin.x += offset;
    bounds.origin.y += offset;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(format!("(無題) - {APP_TITLE}").into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let result = cx.open_window(options, |window, cx| {
        let editor = cx.new(Editor::new);
        let focus_handle = editor.read(cx).focus_handle.clone();
        window.focus(&focus_handle, cx);
        let weak = editor.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |editor, cx| editor.should_close(window, cx))
                .unwrap_or(true)
        });
        editor
    });
    match result {
        Ok(handle) => Some(handle),
        Err(err) => {
            eprintln!("ume-poc-gpui: cannot open a window: {err:#}");
            None
        }
    }
}

impl Editor {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            doc: Document::default(),
            path: None,
            has_bom: false,
            scroll: Point::default(),
            autoscroll: false,
            goal_x: None,
            selecting: false,
            layout: None,
            message: None,
            title: String::new(),
            close_confirmed: false,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// An untitled, untouched window, which opening a file reuses.
    pub fn is_pristine(&self) -> bool {
        self.path.is_none() && !self.doc.is_modified() && self.doc.is_empty()
    }

    fn display_name(&self) -> String {
        self.path.as_deref().and_then(Path::file_name).map_or_else(
            || "(無題)".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    pub fn set_document(&mut self, doc: Document, cx: &mut Context<Self>) {
        self.doc = doc;
        self.path = None;
        self.has_bom = false;
        self.scroll = Point::default();
        self.goal_x = None;
        self.layout = None;
        self.message = None;
        cx.notify();
    }

    pub fn set_message(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.message = Some(message.into());
        cx.notify();
    }

    /// Moves the caret to the start of `line` (0-based) and scrolls it into view.
    #[cfg(feature = "self-check")]
    pub fn go_to_line(&mut self, line: usize, cx: &mut Context<Self>) {
        self.move_to(self.doc.offset(line, 0), false, cx);
    }

    /// What the status bar says.
    #[cfg(feature = "self-check")]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn open_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        match File::open(path).and_then(|file| Document::read_from(BufReader::new(file))) {
            Ok((doc, has_bom)) => {
                self.set_document(doc, cx);
                self.path = Some(path.to_path_buf());
                self.has_bom = has_bom;
            }
            // Sakura opens a file that does not exist as an empty document with its name, and
            // saving creates the file (CLoadAgent::OnLoad in sakura_core/agent/CLoadAgent.cpp).
            // Sakura says so only when asked to; here the status bar always says it, in
            // Sakura's words (STR_NOT_EXSIST_SAVE in sakura_core/sakura_rc.rc).
            Err(err)
                if err.kind() == io::ErrorKind::NotFound
                    && path.parent().is_some_and(Path::is_dir) =>
            {
                self.set_document(Document::default(), cx);
                self.path = Some(path.to_path_buf());
                let message = format!(
                    "{} というファイルは存在しません。保存したときに作成されます",
                    self.display_name()
                );
                self.set_message(message, cx);
            }
            Err(err) if err.kind() == io::ErrorKind::InvalidData => self.set_message(
                format!(
                    "{} は UTF-8 として読めません（試作は UTF-8 だけに対応しています）",
                    path.display()
                ),
                cx,
            ),
            Err(err) => self.set_message(format!("{} を開けません: {err}", path.display()), cx),
        }
    }

    /// Saves to the current path, or asks for one. Resolves to whether the file was written.
    fn save(&mut self, ask_path: bool, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        if !ask_path && let Some(path) = self.path.clone() {
            return Task::ready(self.write_file(&path, cx));
        }
        let directory = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let name = self.path.as_deref().and_then(Path::file_name).map_or_else(
            || "無題.txt".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let receiver = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = receiver.await else {
                return false;
            };
            this.update(cx, |this, cx| this.write_file(&path, cx))
                .unwrap_or(false)
        })
    }

    fn write_file(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        match write_atomically(path, &self.doc, self.has_bom) {
            Ok(()) => {
                self.path = Some(path.to_path_buf());
                self.doc.mark_saved();
                self.set_message("保存しました", cx);
                true
            }
            Err(err) => {
                self.set_message(format!("保存できませんでした: {err}"), cx);
                false
            }
        }
    }

    /// Asks whether to save a modified document. Returns whether the window can close now; if
    /// not, the window closes itself once the user has answered.
    pub fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.close_confirmed || !self.doc.is_modified() {
            return true;
        }
        // Sakura asks the same (STR_ERR_DLGEDITDOC31 in sakura_core/sakura_rc.rc).
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("{} は変更されています。", self.display_name()),
            Some("閉じる前に保存しますか？"),
            &["保存", "保存しない", "キャンセル"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let save = match answer.await {
                Ok(0) => true,
                Ok(1) => false,
                _ => return,
            };
            if save {
                let Ok(saved) = this.update_in(cx, |this, window, cx| this.save(false, window, cx))
                else {
                    return;
                };
                if !saved.await {
                    return;
                }
            }
            this.update_in(cx, |this, window, _| {
                this.close_confirmed = true;
                window.remove_window();
            })
            .ok();
        })
        .detach();
        false
    }

    fn sync_title(&mut self, window: &mut Window) {
        // Sakura's caption: "$f(更新) - sakura $V" (sakura_core/env/CShareData.cpp).
        let modified = self.doc.is_modified();
        let title = format!(
            "{}{} - {APP_TITLE}",
            self.display_name(),
            if modified { "(更新)" } else { "" }
        );
        if title != self.title {
            window.set_window_title(&title);
            window.set_window_edited(modified);
            window.set_document_path(self.path.as_deref());
            self.title = title;
        }
    }

    /// Clamps the scroll offset and, after the caret moved, scrolls it into view. Called while
    /// the frame is laid out, when the size of the view is known.
    pub fn update_scroll(
        &mut self,
        style: &LineStyle,
        text_bounds: Bounds<Pixels>,
        window: &Window,
    ) -> Point<Pixels> {
        let lh = style.line_height;
        let view = text_bounds.size;
        let mut max_x = self
            .layout
            .as_ref()
            .and_then(|layout| layout.lines.iter().map(|(line, _)| line.width()).max())
            .unwrap_or_default();
        if self.autoscroll {
            self.autoscroll = false;
            let (line, col) = self.doc.point(self.doc.cursor());
            let top = lh * line;
            if top < self.scroll.y {
                self.scroll.y = top;
            } else if top + lh > self.scroll.y + view.height {
                self.scroll.y = top + lh - view.height;
            }
            let text = self.doc.line_text(line, MAX_SHAPED_CHARS);
            let x = style
                .shape(&text, None, window)
                .x_for_index(char_to_byte(&text, col))
                + TEXT_PADDING;
            let margin = style.cell_width * 4.;
            if x < self.scroll.x + margin {
                self.scroll.x = x - margin;
            } else if x > self.scroll.x + view.width - margin {
                self.scroll.x = x - view.width + margin;
            }
            max_x = max_x.max(x);
        }
        // The widest line comes from the last frame; the view can scroll a little past it.
        let max_scroll_x = (max_x + style.cell_width * 4. - view.width).max(px(0.));
        // The last line can scroll up to the top, as in Sakura.
        let max_scroll_y = lh * (self.doc.line_count() - 1);
        self.scroll.x = self.scroll.x.clamp(px(0.), max_scroll_x);
        self.scroll.y = self.scroll.y.clamp(px(0.), max_scroll_y);
        self.scroll
    }

    fn position_for_point(&self, position: Point<Pixels>, window: &Window, cx: &App) -> usize {
        let Some(layout) = &self.layout else {
            return self.doc.cursor();
        };
        let y = position.y - layout.text_bounds.top() + layout.scroll.y;
        let line = if y < px(0.) {
            0
        } else {
            ((y / layout.line_height) as usize).min(self.doc.line_count() - 1)
        };
        let x = position.x - layout.text_left();
        let byte = match layout.shaped(line) {
            Some((shaped, text)) => byte_to_char(text, shaped.closest_index_for_x(x)),
            None => {
                let style = LineStyle::new(cx.global::<Settings>(), window);
                let text = self.doc.line_text(line, MAX_SHAPED_CHARS);
                byte_to_char(
                    &text,
                    style.shape(&text, None, window).closest_index_for_x(x),
                )
            }
        };
        self.doc.offset(line, byte)
    }

    pub fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.doc.unmark_text();
        let in_gutter = self
            .layout
            .as_ref()
            .is_some_and(|layout| event.position.x < layout.text_bounds.left());
        let pos = self.position_for_point(event.position, window, cx);
        let anchor = self.doc.selection().anchor;
        let selection = if in_gutter || event.click_count >= 3 {
            let range = self.doc.line_range(self.doc.line_of(pos));
            Selection {
                anchor: range.start,
                head: range.end,
            }
        } else if event.click_count == 2 {
            let range = self.doc.word_range(pos);
            Selection {
                anchor: range.start,
                head: range.end,
            }
        } else if event.modifiers.shift {
            Selection { anchor, head: pos }
        } else {
            Selection::caret(pos)
        };
        self.doc.set_selection(selection);
        self.selecting = true;
        self.goal_x = None;
        self.autoscroll = true;
        cx.notify();
    }

    pub fn mouse_drag(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selecting {
            return;
        }
        let head = self.position_for_point(position, window, cx);
        let selection = self.doc.selection();
        if selection.head != head {
            self.doc.set_selection(Selection {
                anchor: selection.anchor,
                head,
            });
            self.autoscroll = true;
            cx.notify();
        }
    }

    pub fn mouse_up(&mut self) {
        self.selecting = false;
    }

    pub fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let line_height = self
            .layout
            .as_ref()
            .map_or(px(20.), |layout| layout.line_height);
        let delta = event.delta.pixel_delta(line_height);
        self.scroll = point(self.scroll.x - delta.x, self.scroll.y - delta.y);
        cx.notify();
    }

    fn move_to(&mut self, head: usize, extend: bool, cx: &mut Context<Self>) {
        let anchor = self.doc.selection().anchor;
        self.doc.set_selection(if extend {
            Selection { anchor, head }
        } else {
            Selection::caret(head)
        });
        self.goal_x = None;
        self.autoscroll = true;
        cx.notify();
    }

    fn move_vertically(
        &mut self,
        lines: isize,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let style = LineStyle::new(cx.global::<Settings>(), window);
        let selection = self.doc.selection();
        let (line, col) = self.doc.point(selection.head);
        let goal_x = self.goal_x.unwrap_or_else(|| {
            let text = self.doc.line_text(line, MAX_SHAPED_CHARS);
            style
                .shape(&text, None, window)
                .x_for_index(char_to_byte(&text, col))
        });
        let last_line = self.doc.line_count() as isize - 1;
        let target = (line as isize + lines).clamp(0, last_line) as usize;
        let text = self.doc.line_text(target, MAX_SHAPED_CHARS);
        let col = byte_to_char(
            &text,
            style.shape(&text, None, window).closest_index_for_x(goal_x),
        );
        self.move_to(self.doc.offset(target, col), extend, cx);
        self.goal_x = Some(goal_x);
    }

    fn move_page(&mut self, down: bool, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else {
            return;
        };
        // Sakura scrolls by a page and moves the caret by as many lines (F_1PageDown).
        let lines = layout.page_lines().saturating_sub(1).max(1);
        let delta = layout.line_height * lines;
        self.scroll.y = if down {
            self.scroll.y + delta
        } else {
            self.scroll.y - delta
        };
        let lines = lines as isize;
        self.move_vertically(if down { lines } else { -lines }, extend, window, cx);
    }

    /// Left and Right only cancel a selection, as in Sakura (`Command_LEFT` in
    /// sakura_core/cmd/CViewCommander_Cursor.cpp).
    fn move_horizontally(&mut self, forward: bool, extend: bool, cx: &mut Context<Self>) {
        let selection = self.doc.selection();
        if !extend && !selection.is_empty() {
            self.move_to(selection.head, false, cx);
            return;
        }
        let head = if forward {
            self.doc.next_boundary(selection.head)
        } else {
            self.doc.prev_boundary(selection.head)
        };
        self.move_to(head, extend, cx);
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.autoscroll = true;
        self.message = None;
        cx.notify();
    }

    fn render_status_bar(&self, cx: &App) -> impl IntoElement {
        let settings = cx.global::<Settings>();
        let (line, col) = self.doc.point(self.doc.cursor());
        // Sakura counts columns in half-width cells: "%5d 行 %4d 桁" (STR_STATUS_ROW_COL).
        let mut column = 0;
        for c in self.doc.line_text(line, col).chars() {
            column += match c {
                '\t' => TAB_WIDTH - column % TAB_WIDTH,
                c => c.width().unwrap_or(1),
            };
        }
        let line_ending = self.doc.line_ending().label();
        let encoding = if self.has_bom {
            "UTF-8 BOM付"
        } else {
            "UTF-8"
        };
        div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .h(px(22.))
            .px_2()
            .border_t_1()
            .border_color(rgb(0xc8c8c8))
            .bg(rgb(0xf0f0f0))
            .text_size(px(12.))
            .text_color(rgb(0x333333))
            .child(div().child(self.message.clone().unwrap_or_default()))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_4()
                    .child(format!("{:>5} 行 {:>4} 桁", line + 1, column + 1))
                    .child(line_ending)
                    .child(encoding)
                    .child(format!(
                        "{} {}pt",
                        settings.font_label(),
                        settings.font_size.as_f32()
                    )),
            )
    }

    fn on_drop(&mut self, paths: &ExternalPaths, _: &mut Window, cx: &mut Context<Self>) {
        let paths = paths.paths().to_vec();
        cx.defer(move |cx| crate::open_paths(paths, cx));
    }

    fn file_save(&mut self, _: &FileSave, window: &mut Window, cx: &mut Context<Self>) {
        self.save(false, window, cx).detach();
    }

    fn file_save_as(&mut self, _: &FileSaveAs, window: &mut Window, cx: &mut Context<Self>) {
        self.save(true, window, cx).detach();
    }

    fn win_close(&mut self, _: &WinClose, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close(window, cx) {
            window.remove_window();
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.doc.undo() {
            self.after_edit(cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.doc.redo() {
            self.after_edit(cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.doc.selection().range();
        if !range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.doc.slice(range)));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.doc.selection().range();
        if !range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.doc.slice(range.clone())));
            self.doc.edit(range, "");
            self.after_edit(cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        // Pasted line breaks become the document's, as Sakura converts them on paste.
        let text = normalize_line_endings(&text, self.doc.line_ending().as_str());
        self.doc.edit(self.doc.selection().range(), &text);
        self.after_edit(cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.doc.delete_forward() {
            self.after_edit(cx);
        }
    }

    fn delete_back(&mut self, _: &DeleteBack, _: &mut Window, cx: &mut Context<Self>) {
        if self.doc.delete_backward() {
            self.after_edit(cx);
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.doc.select_all();
        cx.notify();
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.doc.insert_newline();
        self.after_edit(cx);
    }

    fn indent_tab(&mut self, _: &IndentTab, _: &mut Window, cx: &mut Context<Self>) {
        self.doc.edit(self.doc.selection().range(), "\t");
        self.after_edit(cx);
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn tolower(&mut self, _: &ToLower, _: &mut Window, cx: &mut Context<Self>) {
        if self.doc.convert_case(false) {
            self.after_edit(cx);
        }
    }

    fn toupper(&mut self, _: &ToUpper, _: &mut Window, cx: &mut Context<Self>) {
        if self.doc.convert_case(true) {
            self.after_edit(cx);
        }
    }

    fn minimize(&mut self, _: &Minimize, window: &mut Window, _: &mut Context<Self>) {
        window.minimize_window();
    }

    fn zoom(&mut self, _: &Zoom, window: &mut Window, _: &mut Context<Self>) {
        window.zoom_window();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontally(false, false, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontally(true, false, cx);
    }

    fn left_sel(&mut self, _: &LeftSel, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontally(false, true, cx);
    }

    fn right_sel(&mut self, _: &RightSel, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontally(true, true, cx);
    }

    fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, false, window, cx);
    }

    fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, false, window, cx);
    }

    fn up_sel(&mut self, _: &UpSel, window: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, true, window, cx);
    }

    fn down_sel(&mut self, _: &DownSel, window: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, true, window, cx);
    }

    fn line_top(&self) -> usize {
        self.doc.line_start(self.doc.line_of(self.doc.cursor()))
    }

    fn line_end(&self) -> usize {
        self.doc.line_end(self.doc.line_of(self.doc.cursor()))
    }

    fn go_line_top(&mut self, _: &GoLineTop, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_top(), false, cx);
    }

    fn go_line_end(&mut self, _: &GoLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_end(), false, cx);
    }

    fn go_line_top_sel(&mut self, _: &GoLineTopSel, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_top(), true, cx);
    }

    fn go_line_end_sel(&mut self, _: &GoLineEndSel, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_end(), true, cx);
    }

    fn go_file_top(&mut self, _: &GoFileTop, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, false, cx);
    }

    fn go_file_end(&mut self, _: &GoFileEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.doc.len_chars(), false, cx);
    }

    fn go_file_top_sel(&mut self, _: &GoFileTopSel, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, true, cx);
    }

    fn go_file_end_sel(&mut self, _: &GoFileEndSel, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.doc.len_chars(), true, cx);
    }

    fn page_up(&mut self, _: &PageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.move_page(false, false, window, cx);
    }

    fn page_down(&mut self, _: &PageDown, window: &mut Window, cx: &mut Context<Self>) {
        self.move_page(true, false, window, cx);
    }

    fn page_up_sel(&mut self, _: &PageUpSel, window: &mut Window, cx: &mut Context<Self>) {
        self.move_page(false, true, window, cx);
    }

    fn page_down_sel(&mut self, _: &PageDownSel, window: &mut Window, cx: &mut Context<Self>) {
        self.move_page(true, true, window, cx);
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_title(window);
        div()
            .id("editor")
            .key_context(EDITOR_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::file_save))
            .on_action(cx.listener(Self::file_save_as))
            .on_action(cx.listener(Self::win_close))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_back))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::indent_tab))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::tolower))
            .on_action(cx.listener(Self::toupper))
            .on_action(cx.listener(Self::minimize))
            .on_action(cx.listener(Self::zoom))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::left_sel))
            .on_action(cx.listener(Self::right_sel))
            .on_action(cx.listener(Self::up_sel))
            .on_action(cx.listener(Self::down_sel))
            .on_action(cx.listener(Self::go_line_top))
            .on_action(cx.listener(Self::go_line_end))
            .on_action(cx.listener(Self::go_line_top_sel))
            .on_action(cx.listener(Self::go_line_end_sel))
            .on_action(cx.listener(Self::go_file_top))
            .on_action(cx.listener(Self::go_file_end))
            .on_action(cx.listener(Self::go_file_top_sel))
            .on_action(cx.listener(Self::go_file_end_sel))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::page_up_sel))
            .on_action(cx.listener(Self::page_down_sel))
            .on_drop(cx.listener(Self::on_drop))
            .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(0.7))
            .child(EditorElement::new(cx.entity()))
            .child(self.render_status_bar(cx))
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The IME and the keyboard talk to the editor through this, in UTF-16 offsets
/// (NSTextInputClient on macOS).
impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.doc.slice(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let selection = self.doc.selection();
        Some(UTF16Selection {
            range: self.range_to_utf16(&selection.range()),
            reversed: selection.reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.doc
            .marked_range()
            .map(|range| self.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.doc.unmark_text();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.range_from_utf16(&range));
        self.doc.insert_text(range, text);
        self.after_edit(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.range_from_utf16(&range));
        let selected = new_selected_range_utf16.map(|range| {
            utf16_to_char_in(new_text, range.start)..utf16_to_char_in(new_text, range.end)
        });
        self.doc.set_marked_text(range, new_text, selected);
        self.after_edit(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let (line, start_col) = self.doc.point(range.start);
        let (end_line, end_col) = self.doc.point(range.end);
        let shaped;
        let (line_layout, text) = match layout.shaped(line) {
            Some((shaped, text)) => (shaped, text.as_str()),
            None => {
                let style = LineStyle::new(cx.global::<Settings>(), window);
                let text = self.doc.line_text(line, MAX_SHAPED_CHARS);
                shaped = (style.shape(&text, None, window), text);
                (&shaped.0, shaped.1.as_str())
            }
        };
        let x0 = line_layout.x_for_index(char_to_byte(text, start_col));
        let x1 = if end_line == line {
            line_layout.x_for_index(char_to_byte(text, end_col))
        } else {
            line_layout.width()
        };
        let top = layout.line_top(line);
        let left = layout.text_left();
        Some(Bounds::from_corners(
            point(left + x0, top),
            point(left + x1, top + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        self.layout.as_ref()?;
        let pos = self.position_for_point(point, window, cx);
        Some(self.doc.char_to_utf16(pos))
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.doc.len_utf16())
    }
}

impl Editor {
    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.doc.utf16_to_char(range.start)..self.doc.utf16_to_char(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.doc.char_to_utf16(range.start)..self.doc.char_to_utf16(range.end)
    }
}

fn utf16_to_char_in(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.chars().enumerate() {
        if units >= offset {
            return i;
        }
        units += c.len_utf16();
    }
    text.chars().count()
}

fn normalize_line_endings(text: &str, line_ending: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                chars.next_if_eq(&'\n');
                out.push_str(line_ending);
            }
            '\n' => out.push_str(line_ending),
            c => out.push(c),
        }
    }
    out
}

/// Writes next to the file and renames it over the old one, so that a failed write keeps the
/// old file (ADR-0005).
fn write_atomically(path: &Path, doc: &Document, has_bom: bool) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ファイル名がありません"))?;
    let temp = path.with_file_name(format!(".{}.ume-tmp", name.to_string_lossy()));
    let result = (|| {
        let mut writer = BufWriter::new(File::create(&temp)?);
        if has_bom {
            writer.write_all("\u{feff}".as_bytes())?;
        }
        doc.write_to(&mut writer)?;
        writer
            .into_inner()
            .map_err(|err| err.into_error())?
            .sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        std::fs::remove_file(&temp).ok();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_replaces_the_file_and_keeps_the_bom() {
        let dir = std::env::temp_dir().join(format!("ume-poc-gpui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        std::fs::write(&path, "old").unwrap();
        let (doc, has_bom) = Document::read_from("\u{feff}new\r\n".as_bytes()).unwrap();
        write_atomically(&path, &doc, has_bom).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "\u{feff}new\r\n");
        // The temporary file is gone.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pasted_line_breaks_become_the_documents() {
        assert_eq!(
            normalize_line_endings("a\r\nb\nc\rd", "\r\n"),
            "a\r\nb\r\nc\r\nd"
        );
    }

    #[test]
    fn utf16_offsets_within_composed_text() {
        assert_eq!(utf16_to_char_in("a𠮷b", 3), 2);
        assert_eq!(utf16_to_char_in("にほんご", 4), 4);
    }
}
