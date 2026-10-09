//! Text model of the prototype: a rope with a selection, an IME composition, and undo history.
//!
//! Positions are char indices into the rope. The IME speaks UTF-16 offsets, which the rope
//! converts in O(log n), so a keystroke costs the same in a short note and in a file with a
//! million lines. The real buffer will be a piece tree in `ume-core` (ADR-0005); this model only
//! has to be good enough to evaluate the UI framework.

use std::io::{self, Read, Write};
use std::ops::Range;

use ropey::{Rope, RopeBuilder, RopeSlice};
use unicode_segmentation::UnicodeSegmentation;

/// A line break kind. New lines use the kind of the first line break in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
            LineEnding::Cr => "CR",
        }
    }

    fn detect(rope: &Rope) -> Self {
        let mut chars = rope.chars();
        while let Some(c) = chars.next() {
            match c {
                '\n' => return LineEnding::Lf,
                '\r' if chars.next() == Some('\n') => return LineEnding::CrLf,
                '\r' => return LineEnding::Cr,
                _ => {}
            }
        }
        LineEnding::Lf
    }
}

/// A selection in char indices. `head` is where the caret is; an empty selection is a caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn reversed(&self) -> bool {
        self.head < self.anchor
    }
}

#[derive(Debug)]
struct Edit {
    start: usize,
    removed: String,
    inserted: String,
    before: Selection,
    after: Selection,
}

/// What an IME composition replaced. The composed text is edited many times while the user
/// converts it; the whole composition becomes one undo step when it ends.
#[derive(Debug)]
struct Composition {
    start: usize,
    replaced: String,
    before: Selection,
}

pub struct Document {
    rope: Rope,
    selection: Selection,
    /// The text the IME is composing. It always starts at `composition.start`.
    marked: Option<Range<usize>>,
    composition: Option<Composition>,
    undo_stack: Vec<Edit>,
    redo_stack: Vec<Edit>,
    line_ending: LineEnding,
    modified: bool,
}

impl Default for Document {
    fn default() -> Self {
        Self::from_rope(Rope::new())
    }
}

impl Document {
    #[cfg(test)]
    pub fn new(text: &str) -> Self {
        Self::from_rope(Rope::from_str(text))
    }

    pub fn from_rope(rope: Rope) -> Self {
        let line_ending = LineEnding::detect(&rope);
        Self {
            rope,
            selection: Selection::default(),
            marked: None,
            composition: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            line_ending,
            modified: false,
        }
    }

    /// Reads UTF-8 text in chunks. Returns the document and whether the text started with a BOM.
    pub fn read_from(reader: impl Read) -> io::Result<(Self, bool)> {
        let mut rope = Rope::from_reader(reader)?;
        let has_bom = rope.len_chars() > 0 && rope.char(0) == '\u{feff}';
        if has_bom {
            rope.remove(0..1);
        }
        Ok((Self::from_rope(rope), has_bom))
    }

    /// Writes the text in chunks, without building one string of the whole document.
    pub fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        for chunk in self.rope.chunks() {
            writer.write_all(chunk.as_bytes())?;
        }
        Ok(())
    }

    /// Builds a document of `count` lines for the scrolling check (docs/poc/ui-framework.md #4).
    pub fn sample(count: usize) -> Self {
        let mut builder = RopeBuilder::new();
        for i in 1..=count {
            builder.append(&format!(
                "{i:>7} 行目　日本語と ASCII が混ざった行です。The quick brown fox jumps over the lazy dog.\n"
            ));
        }
        Self::from_rope(builder.finish())
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_utf16(&self) -> usize {
        self.rope.len_utf16_cu()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn mark_saved(&mut self) {
        self.modified = false;
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    pub fn set_selection(&mut self, selection: Selection) {
        let len = self.len_chars();
        self.selection = Selection {
            anchor: selection.anchor.min(len),
            head: selection.head.min(len),
        };
    }

    pub fn cursor(&self) -> usize {
        self.selection.head
    }

    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    /// The number of chars in `line`, not counting its line break.
    pub fn line_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        slice.len_chars() - eol_len(&slice)
    }

    /// The position after the last char of `line`, before its line break.
    pub fn line_end(&self, line: usize) -> usize {
        self.line_start(line) + self.line_len(line)
    }

    /// Up to `max_chars` chars of `line`, without its line break.
    pub fn line_text(&self, line: usize, max_chars: usize) -> String {
        let slice = self.rope.line(line);
        let len = (slice.len_chars() - eol_len(&slice)).min(max_chars);
        slice.slice(..len).to_string()
    }

    pub fn line_ending_of(&self, line: usize) -> Option<LineEnding> {
        let slice = self.rope.line(line);
        match eol_len(&slice) {
            0 => None,
            2 => Some(LineEnding::CrLf),
            _ if slice.char(slice.len_chars() - 1) == '\r' => Some(LineEnding::Cr),
            _ => Some(LineEnding::Lf),
        }
    }

    pub fn line_of(&self, pos: usize) -> usize {
        self.rope.char_to_line(pos.min(self.len_chars()))
    }

    /// The line and the column (in chars) of `pos`. A position inside a CRLF counts as the end of
    /// its line.
    pub fn point(&self, pos: usize) -> (usize, usize) {
        let line = self.line_of(pos);
        let col = (pos.min(self.len_chars()) - self.line_start(line)).min(self.line_len(line));
        (line, col)
    }

    pub fn offset(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.line_count() - 1);
        self.line_start(line) + col.min(self.line_len(line))
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        self.rope.slice(range).to_string()
    }

    pub fn char_to_utf16(&self, pos: usize) -> usize {
        self.rope.char_to_utf16_cu(pos.min(self.len_chars()))
    }

    pub fn utf16_to_char(&self, offset: usize) -> usize {
        self.rope.utf16_cu_to_char(offset.min(self.len_utf16()))
    }

    /// The caret position one grapheme before `pos`. A line break counts as one step.
    pub fn prev_boundary(&self, pos: usize) -> usize {
        let (line, col) = self.point(pos);
        if col == 0 {
            return if line == 0 {
                0
            } else {
                self.line_end(line - 1)
            };
        }
        let text = self.line_text(line, col);
        let prev = text
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i);
        self.line_start(line) + text[..prev].chars().count()
    }

    /// The caret position one grapheme after `pos`. A line break counts as one step.
    pub fn next_boundary(&self, pos: usize) -> usize {
        let (line, col) = self.point(pos);
        let len = self.line_len(line);
        if col >= len {
            return if line + 1 < self.line_count() {
                self.line_start(line + 1)
            } else {
                self.line_end(line)
            };
        }
        let text = self.line_text(line, usize::MAX);
        let byte = char_to_byte(&text, col);
        let next = text[byte..]
            .grapheme_indices(true)
            .nth(1)
            .map_or(text.len(), |(i, _)| byte + i);
        self.line_start(line) + text[..next].chars().count()
    }

    /// The run of same-kind characters around `pos`, for double-click selection.
    ///
    /// Sakura groups characters by kind (`CWordParse::WhatKindOfChar` in sakura_core/parse); this
    /// is a coarse approximation by script.
    pub fn word_range(&self, pos: usize) -> Range<usize> {
        let (line, col) = self.point(pos);
        let chars: Vec<char> = self.line_text(line, usize::MAX).chars().collect();
        if chars.is_empty() {
            return pos..pos;
        }
        let at = col.min(chars.len() - 1);
        let kind = char_kind(chars[at]);
        let start = chars[..at]
            .iter()
            .rposition(|&c| char_kind(c) != kind)
            .map_or(0, |i| i + 1);
        let end = chars[at..]
            .iter()
            .position(|&c| char_kind(c) != kind)
            .map_or(chars.len(), |i| at + i);
        let line_start = self.line_start(line);
        line_start + start..line_start + end
    }

    /// The whole line including its line break, for triple-click and line-number selection.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let start = self.line_start(line);
        let end = if line + 1 < self.line_count() {
            self.line_start(line + 1)
        } else {
            self.line_end(line)
        };
        start..end
    }

    pub fn select_all(&mut self) {
        self.unmark_text();
        self.selection = Selection {
            anchor: 0,
            head: self.len_chars(),
        };
    }

    /// Replaces `range` with `text` as one undo step and puts the caret after the new text.
    pub fn edit(&mut self, range: Range<usize>, text: &str) {
        self.unmark_text();
        let before = self.selection;
        let removed = self.slice(range.clone());
        if removed.is_empty() && text.is_empty() {
            return;
        }
        let inserted = self.splice(range.clone(), text);
        self.selection = Selection::caret(inserted.end);
        self.push_undo(Edit {
            start: range.start,
            removed,
            inserted: text.to_owned(),
            before,
            after: self.selection,
        });
    }

    pub fn delete_backward(&mut self) -> bool {
        let range = if self.selection.is_empty() {
            self.prev_boundary(self.selection.head)..self.selection.head
        } else {
            self.selection.range()
        };
        if range.is_empty() {
            return false;
        }
        self.edit(range, "");
        true
    }

    pub fn delete_forward(&mut self) -> bool {
        let range = if self.selection.is_empty() {
            self.selection.head..self.next_boundary(self.selection.head)
        } else {
            self.selection.range()
        };
        if range.is_empty() {
            return false;
        }
        self.edit(range, "");
        true
    }

    pub fn insert_newline(&mut self) {
        self.edit(self.selection.range(), self.line_ending.as_str());
    }

    /// Converts the selected text to upper or lower case and keeps it selected, as Sakura does
    /// (`CEditView::ConvSelectedArea` in sakura_core/view/CEditView.cpp).
    pub fn convert_case(&mut self, upper: bool) -> bool {
        let range = self.selection.range();
        if range.is_empty() {
            return false;
        }
        let text = self.slice(range.clone());
        let converted = if upper {
            text.to_uppercase()
        } else {
            text.to_lowercase()
        };
        if converted == text {
            return false;
        }
        self.edit(range.clone(), &converted);
        self.selection = Selection {
            anchor: range.start,
            head: range.start + converted.chars().count(),
        };
        if let Some(edit) = self.undo_stack.last_mut() {
            edit.after = self.selection;
        }
        true
    }

    /// Text from the keyboard or the IME (`insertText` on macOS). Without a range, it replaces
    /// the composed text, or the selection when nothing is composed.
    pub fn insert_text(&mut self, range: Option<Range<usize>>, text: &str) {
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.range());
        if let Some(composition) = self.composition.take() {
            let marked = self.marked.take();
            if marked.as_ref() == Some(&range) {
                let inserted = self.splice(range, text);
                self.selection = Selection::caret(inserted.end);
                self.push_composition(composition, inserted);
                return;
            }
            // The IME replaced other text: keep what was composed so far as typed.
            let start = composition.start;
            self.push_composition(composition, marked.unwrap_or(start..start));
        }
        self.edit(range, text);
    }

    /// Text that the IME is composing (`setMarkedText` on macOS). `selected` is relative to
    /// `text`, in chars.
    pub fn set_marked_text(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) {
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.range());
        if self.composition.is_some() && self.marked.as_ref() != Some(&range) {
            self.unmark_text();
        }
        if self.composition.is_none() {
            self.composition = Some(Composition {
                start: range.start,
                replaced: self.slice(range.clone()),
                before: self.selection,
            });
        }
        let inserted = self.splice(range, text);
        self.marked = (!text.is_empty()).then(|| inserted.clone());
        let len = inserted.len();
        let selected = selected.unwrap_or(len..len);
        self.selection = Selection {
            anchor: inserted.start + selected.start.min(len),
            head: inserted.start + selected.end.min(len),
        };
    }

    /// Ends the composition and keeps the composed text as it is (`unmarkText` on macOS).
    pub fn unmark_text(&mut self) {
        if let Some(composition) = self.composition.take() {
            let start = composition.start;
            let marked = self.marked.take().unwrap_or(start..start);
            self.push_composition(composition, marked);
        }
        self.marked = None;
    }

    pub fn undo(&mut self) -> bool {
        self.unmark_text();
        let Some(edit) = self.undo_stack.pop() else {
            return false;
        };
        let inserted_len = edit.inserted.chars().count();
        self.splice(edit.start..edit.start + inserted_len, &edit.removed);
        self.selection = edit.before;
        self.redo_stack.push(edit);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.unmark_text();
        let Some(edit) = self.redo_stack.pop() else {
            return false;
        };
        let removed_len = edit.removed.chars().count();
        self.splice(edit.start..edit.start + removed_len, &edit.inserted);
        self.selection = edit.after;
        self.undo_stack.push(edit);
        true
    }

    /// Replaces `range` with `text` without recording it. Returns the range of the new text.
    fn splice(&mut self, range: Range<usize>, text: &str) -> Range<usize> {
        if !range.is_empty() {
            self.rope.remove(range.clone());
        }
        if !text.is_empty() {
            self.rope.insert(range.start, text);
        }
        self.modified = true;
        range.start..range.start + text.chars().count()
    }

    fn push_composition(&mut self, composition: Composition, composed: Range<usize>) {
        let inserted = self.slice(composed);
        if composition.replaced == inserted {
            return;
        }
        self.push_undo(Edit {
            start: composition.start,
            removed: composition.replaced,
            inserted,
            before: composition.before,
            after: self.selection,
        });
    }

    fn push_undo(&mut self, edit: Edit) {
        self.undo_stack.push(edit);
        self.redo_stack.clear();
    }
}

fn eol_len(line: &RopeSlice) -> usize {
    let len = line.len_chars();
    match (
        len.checked_sub(2).map(|i| line.char(i)),
        len.checked_sub(1).map(|i| line.char(i)),
    ) {
        (Some('\r'), Some('\n')) => 2,
        (_, Some('\n' | '\r')) => 1,
        _ => 0,
    }
}

pub fn char_to_byte(text: &str, col: usize) -> usize {
    text.char_indices().nth(col).map_or(text.len(), |(i, _)| i)
}

pub fn byte_to_char(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharKind {
    Space,
    Word,
    Hiragana,
    Katakana,
    Ideograph,
    Other,
}

fn char_kind(c: char) -> CharKind {
    match c {
        ' ' | '\t' | '\u{3000}' => CharKind::Space,
        '_' => CharKind::Word,
        c if c.is_alphanumeric() && (c as u32) < 0x3000 => CharKind::Word,
        '\u{3041}'..='\u{309f}' => CharKind::Hiragana,
        '\u{30a0}'..='\u{30ff}' | '\u{ff66}'..='\u{ff9f}' => CharKind::Katakana,
        '\u{3005}' | '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{20000}'.. => {
            CharKind::Ideograph
        }
        c if c.is_alphanumeric() => CharKind::Word,
        _ => CharKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(doc: &Document) -> String {
        doc.slice(0..doc.len_chars())
    }

    #[test]
    fn lines_exclude_their_line_breaks() {
        let doc = Document::new("ab\r\ncd\nef\rgh");
        assert_eq!(doc.line_count(), 4);
        assert_eq!(doc.line_text(0, usize::MAX), "ab");
        assert_eq!(doc.line_len(0), 2);
        assert_eq!(doc.line_ending_of(0), Some(LineEnding::CrLf));
        assert_eq!(doc.line_ending_of(1), Some(LineEnding::Lf));
        assert_eq!(doc.line_ending_of(2), Some(LineEnding::Cr));
        assert_eq!(doc.line_ending_of(3), None);
        assert_eq!(doc.line_ending(), LineEnding::CrLf);
    }

    #[test]
    fn caret_steps_over_crlf_at_once() {
        let doc = Document::new("ab\r\ncd");
        assert_eq!(doc.next_boundary(2), 4);
        assert_eq!(doc.prev_boundary(4), 2);
        // A position between CR and LF counts as the end of the line.
        assert_eq!(doc.point(3), (0, 2));
    }

    #[test]
    fn caret_steps_over_graphemes() {
        let doc = Document::new("a👍🏽b");
        assert_eq!(doc.next_boundary(1), 3);
        assert_eq!(doc.prev_boundary(3), 1);
    }

    #[test]
    fn backspace_at_line_start_joins_lines() {
        let mut doc = Document::new("ab\r\ncd");
        doc.set_selection(Selection::caret(4));
        assert!(doc.delete_backward());
        assert_eq!(text(&doc), "abcd");
        assert_eq!(doc.cursor(), 2);
    }

    #[test]
    fn newline_uses_the_files_line_ending() {
        let mut doc = Document::new("a\r\nb");
        doc.set_selection(Selection::caret(1));
        doc.insert_newline();
        assert_eq!(text(&doc), "a\r\n\r\nb");
    }

    #[test]
    fn utf16_offsets_count_surrogate_pairs() {
        let doc = Document::new("a𠮷b");
        assert_eq!(doc.char_to_utf16(2), 3);
        assert_eq!(doc.utf16_to_char(3), 2);
        assert_eq!(doc.len_utf16(), 4);
    }

    #[test]
    fn composition_is_one_undo_step() {
        let mut doc = Document::new("x");
        doc.set_selection(Selection::caret(1));
        doc.set_marked_text(None, "に", None);
        doc.set_marked_text(None, "にほ", None);
        doc.set_marked_text(None, "日本", Some(0..2));
        assert_eq!(doc.marked_range(), Some(1..3));
        doc.insert_text(None, "日本");
        assert_eq!(text(&doc), "x日本");
        assert_eq!(doc.marked_range(), None);
        assert_eq!(doc.cursor(), 3);
        assert!(doc.undo());
        assert_eq!(text(&doc), "x");
        assert_eq!(doc.cursor(), 1);
        assert!(doc.redo());
        assert_eq!(text(&doc), "x日本");
    }

    #[test]
    fn composition_replaces_the_selection() {
        let mut doc = Document::new("abc");
        doc.set_selection(Selection { anchor: 0, head: 3 });
        doc.set_marked_text(None, "か", None);
        doc.insert_text(None, "火");
        assert_eq!(text(&doc), "火");
        assert!(doc.undo());
        assert_eq!(text(&doc), "abc");
    }

    #[test]
    fn cancelled_composition_leaves_nothing_to_undo() {
        let mut doc = Document::new("ab");
        doc.set_selection(Selection::caret(2));
        doc.set_marked_text(None, "あ", None);
        doc.set_marked_text(None, "", None);
        doc.unmark_text();
        assert_eq!(text(&doc), "ab");
        assert!(!doc.undo());
    }

    #[test]
    fn unmark_keeps_the_composed_text() {
        let mut doc = Document::new("");
        doc.set_marked_text(None, "かな", None);
        doc.unmark_text();
        assert_eq!(text(&doc), "かな");
        assert_eq!(doc.marked_range(), None);
        assert!(doc.undo());
        assert_eq!(text(&doc), "");
    }

    #[test]
    fn case_conversion_keeps_the_selection() {
        let mut doc = Document::new("abc def");
        doc.set_selection(Selection { anchor: 4, head: 0 });
        assert!(doc.convert_case(true));
        assert_eq!(text(&doc), "ABC def");
        assert_eq!(doc.selection(), Selection { anchor: 0, head: 4 });
    }

    #[test]
    fn word_range_groups_by_script() {
        let doc = Document::new("foo_bar 日本語のテキスト");
        assert_eq!(doc.word_range(2), 0..7);
        assert_eq!(doc.word_range(9), 8..11);
        assert_eq!(doc.word_range(11), 11..12);
        assert_eq!(doc.word_range(13), 12..16);
    }

    #[test]
    fn writes_what_it_reads() {
        let source = "\u{feff}a\r\nb";
        let (doc, has_bom) = Document::read_from(source.as_bytes()).unwrap();
        assert!(has_bom);
        let mut out = Vec::new();
        doc.write_to(&mut out).unwrap();
        assert_eq!(out, b"a\r\nb");
    }

    #[test]
    fn invalid_utf8_is_an_error() {
        assert!(Document::read_from(&b"\x82\xa0"[..]).is_err());
    }
}
