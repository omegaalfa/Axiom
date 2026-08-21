//! Axiom headless editor.
//!
//! Internal positions are UTF-8 byte offsets. Public cursor and selection APIs
//! clamp offsets and normalize backward to Unicode scalar boundaries. Grapheme
//! navigation is future UI work; LSP will require an explicit UTF-16 mapping.

use floem_editor_core::{
    buffer::{Buffer, rope_text::RopeText},
    cursor::{Cursor, CursorMode},
    editor::EditType,
    line_ending::LineEnding as BackendLineEnding,
    mode::Mode,
    selection::Selection,
    xi_rope::Rope,
};
use std::{
    borrow::Cow,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }

    const fn backend(self) -> BackendLineEnding {
        match self {
            Self::Lf => BackendLineEnding::Lf,
            Self::CrLf => BackendLineEnding::CrLf,
        }
    }

    fn detect(content: &str) -> Self {
        let bytes = content.as_bytes();
        let (mut crlf, mut lf, mut i) = (0, 0, 0);
        while i < bytes.len() {
            if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                crlf += 1;
                i += 2;
            } else {
                if bytes[i] == b'\n' {
                    lf += 1;
                }
                i += 1;
            }
        }
        if crlf > lf { Self::CrLf } else { Self::Lf }
    }
}

/// Buffer is authoritative for text/history/pristine state. Cursor is
/// authoritative for caret and selection; no duplicate selection is stored.
pub struct Document {
    buffer: Buffer,
    cursor: Cursor,
    file_path: Option<PathBuf>,
    line_ending: LineEnding,
}

impl Document {
    pub fn new() -> Self {
        Self::from_content("")
    }

    pub fn from_content(content: &str) -> Self {
        let line_ending = LineEnding::detect(content);
        let mut buffer = Buffer::new(Rope::from(content));
        buffer.set_line_ending(line_ending.backend());
        Self {
            buffer,
            cursor: Cursor::new(CursorMode::Normal(0), None, None),
            file_path: None,
            line_ending,
        }
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let content = fs::read_to_string(path.as_ref())?;
        let mut document = Self::from_content(&content);
        document.file_path = Some(path.as_ref().to_path_buf());
        Ok(document)
    }

    pub fn save(&mut self) -> io::Result<()> {
        let path = self.file_path.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "document has no associated file",
            )
        })?;
        self.write_to(&path)?;
        self.buffer.set_pristine();
        Ok(())
    }

    pub fn save_as<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        self.write_to(path.as_ref())?;
        self.file_path = Some(path.as_ref().to_path_buf());
        self.buffer.set_pristine();
        Ok(())
    }

    fn write_to(&self, path: &Path) -> io::Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(self.content().as_bytes())?;
        temporary.as_file_mut().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        !self.buffer.is_pristine()
    }
    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }
    pub fn set_file_path(&mut self, path: PathBuf) {
        self.file_path = Some(path);
    }
    pub fn content(&self) -> String {
        self.buffer.text().to_string()
    }
    pub fn line_count(&self) -> usize {
        self.buffer.num_lines()
    }
    pub fn line_content(&self, index: usize) -> Cow<'_, str> {
        self.buffer.line_content(index)
    }
    pub fn len(&self) -> usize {
        self.buffer.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buffer.len() == 0
    }
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }
    pub fn mode(&self) -> Mode {
        self.cursor.get_mode()
    }
    pub fn cursor_offset(&self) -> usize {
        self.cursor.mode.offset()
    }
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn selection(&self) -> Selection {
        match &self.cursor.mode {
            CursorMode::Insert(selection) => selection.clone(),
            _ => Selection::new(),
        }
    }

    fn edit_selection(&self) -> Selection {
        match &self.cursor.mode {
            CursorMode::Insert(selection) => selection.clone(),
            _ => Selection::region(self.cursor_offset(), self.cursor_offset()),
        }
    }

    /// Clamp to the buffer and normalize backward to a UTF-8 scalar boundary.
    pub fn move_cursor(&mut self, offset: usize) {
        let offset = self.normalize_offset(offset);
        self.cursor = Cursor::new(CursorMode::Normal(offset), None, None);
    }

    pub fn set_selection(&mut self, anchor: usize, active: usize) {
        let selection =
            Selection::region(self.normalize_offset(anchor), self.normalize_offset(active));
        self.cursor = Cursor::new(CursorMode::Insert(selection), None, None);
    }

    pub fn select_all(&mut self) {
        self.set_selection(0, self.len());
    }

    pub fn clear_selection(&mut self) {
        let offset = self.cursor_offset();
        self.move_cursor(offset);
    }

    pub fn insert_text(&mut self, text: &str) {
        let selection = self.edit_selection();
        let start = selection.min_offset();
        let end = start + normalized_text_len(text, self.line_ending);
        self.apply_edit(selection, text, EditType::InsertChars, end);
    }

    pub fn delete_backward(&mut self) {
        let selection = self.edit_selection();
        if selection.min_offset() != selection.max_offset() {
            let start = selection.min_offset();
            self.apply_edit(selection, "", EditType::Delete, start);
            return;
        }
        let offset = self.cursor_offset();
        let previous = self.previous_codepoint_boundary(offset);
        if previous != offset {
            self.apply_edit(
                Selection::region(previous, offset),
                "",
                EditType::Delete,
                previous,
            );
        }
    }

    pub fn delete_forward(&mut self) {
        let selection = self.edit_selection();
        if selection.min_offset() != selection.max_offset() {
            let start = selection.min_offset();
            self.apply_edit(selection, "", EditType::Delete, start);
            return;
        }
        let offset = self.cursor_offset();
        let next = self.next_codepoint_boundary(offset);
        if next != offset {
            self.apply_edit(
                Selection::region(offset, next),
                "",
                EditType::Delete,
                offset,
            );
        }
    }

    pub fn insert_newline(&mut self) {
        let selection = self.edit_selection();
        let start = selection.min_offset();
        self.apply_edit(
            selection,
            "\n",
            EditType::InsertNewline,
            start + self.line_ending.as_str().len(),
        );
    }

    pub fn undo(&mut self) -> bool {
        let Some((_, _, _, before)) = self.buffer.do_undo() else {
            return false;
        };
        if let Some(mode) = before {
            self.cursor = Cursor::new(mode, None, None);
        } else {
            self.move_cursor(self.cursor_offset());
        }
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((_, _, _, after)) = self.buffer.do_redo() else {
            return false;
        };
        if let Some(mode) = after {
            self.cursor = Cursor::new(mode, None, None);
        } else {
            self.move_cursor(self.cursor_offset());
        }
        true
    }

    fn apply_edit(&mut self, selection: Selection, text: &str, kind: EditType, offset: usize) {
        let before = self.cursor.mode.clone();
        self.buffer.edit([(selection, text)], kind);
        let after = CursorMode::Normal(offset);
        self.buffer.set_cursor_before(before);
        self.buffer.set_cursor_after(after.clone());
        self.cursor = Cursor::new(after, None, None);
    }

    pub fn line_of_offset(&self, offset: usize) -> usize {
        self.buffer.line_of_offset(self.normalize_offset(offset))
    }

    pub fn offset_of_line(&self, line: usize) -> usize {
        self.buffer.offset_of_line(line.min(self.line_count()))
    }

    pub fn previous_codepoint_offset(&self, offset: usize) -> usize {
        self.previous_codepoint_boundary(offset)
    }

    pub fn next_codepoint_offset(&self, offset: usize) -> usize {
        self.next_codepoint_boundary(offset)
    }

    pub fn selection_offsets(&self) -> Option<(usize, usize)> {
        let selection = self.selection();
        if selection.is_empty() {
            return None;
        }
        let start = selection.min_offset();
        let end = selection.max_offset();
        (start != end).then_some((start, end))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection_offsets()
            .map(|(start, end)| self.text_in_range(start, end))
    }

    pub fn cut_selection(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.insert_text("");
        Some(text)
    }

    pub fn paste_text(&mut self, text: &str) {
        self.insert_text(text);
    }

    pub fn text_in_range(&self, start: usize, end: usize) -> String {
        let start = self.normalize_offset(start);
        let end = self.normalize_offset(end).max(start);
        self.buffer.text().slice_to_cow(start..end).into_owned()
    }

    fn normalize_offset(&self, offset: usize) -> usize {
        let content = self.content();
        let mut offset = offset.min(content.len());
        while !content.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn previous_codepoint_boundary(&self, offset: usize) -> usize {
        let content = self.content();
        let offset = self.normalize_offset(offset);
        content[..offset]
            .char_indices()
            .next_back()
            .map_or(offset, |(i, _)| i)
    }

    fn next_codepoint_boundary(&self, offset: usize) -> usize {
        let content = self.content();
        let offset = self.normalize_offset(offset);
        content[offset..]
            .chars()
            .next()
            .map_or(offset, |c| offset + c.len_utf8())
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

fn normalized_text_len(text: &str, eol: LineEnding) -> usize {
    let bytes = text.as_bytes();
    let (mut length, mut i) = (0, 0);
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            length += eol.as_str().len();
            i += 2;
        } else if bytes[i] == b'\n' || bytes[i] == b'\r' {
            length += eol.as_str().len();
            i += 1;
        } else {
            length += 1;
            i += 1;
        }
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn new_document_is_empty_and_pristine() {
        let document = Document::new();
        assert_eq!(document.content(), "");
        assert_eq!(document.line_count(), 1);
        assert_eq!(document.selection_offsets(), None);
        assert!(!document.is_dirty());
    }

    #[test]
    fn insertion_and_cursor_use_byte_offsets() {
        let mut document = Document::new();
        document.insert_text("Olá 👋");
        assert_eq!(document.content(), "Olá 👋");
        assert_eq!(document.cursor_offset(), "Olá 👋".len());
    }

    #[test]
    fn invalid_offsets_clamp_and_normalize_backward() {
        let mut document = Document::from_content("Olá 👋");
        document.move_cursor(usize::MAX);
        assert_eq!(document.cursor_offset(), document.len());
        document.move_cursor(3);
        assert_eq!(document.cursor_offset(), 2);
        document.insert_text("X");
        assert_eq!(document.content(), "OlXá 👋");
    }

    #[test]
    fn selection_is_authoritative_in_cursor() {
        let mut document = Document::from_content("Olá mundo");
        document.set_selection(0, 4);
        assert_eq!(document.selection().max_offset(), 4);
        document.insert_text("Oi");
        assert_eq!(document.content(), "Oi mundo");
        assert!(document.selection().is_empty());
    }

    #[test]
    fn unicode_backspace_required_texts() {
        for text in [
            "João",
            "ação",
            "informação",
            "São Paulo",
            "Olá 👋",
            "こんにちは",
            "你好",
        ] {
            let mut document = Document::from_content(text);
            let expected = text
                .char_indices()
                .next_back()
                .map_or("", |(i, _)| &text[..i]);
            document.move_cursor(document.len());
            document.delete_backward();
            assert_eq!(document.content(), expected, "input: {text}");
            assert!(document.undo());
            assert_eq!(document.content(), text);
            assert!(document.redo());
            assert_eq!(document.content(), expected);
        }
    }

    #[test]
    fn unicode_delete_forward_required_texts() {
        for text in [
            "João",
            "ação",
            "informação",
            "São Paulo",
            "Olá 👋",
            "こんにちは",
            "你好",
        ] {
            let mut document = Document::from_content(text);
            let first = text.chars().next().unwrap().len_utf8();
            document.delete_forward();
            assert_eq!(document.content(), &text[first..], "input: {text}");
        }
    }

    #[test]
    fn unicode_insert_and_selection_are_safe() {
        let mut document = Document::from_content("こんにちは你好");
        document.set_selection(1, 8);
        document.insert_text("Olá 👋");
        assert_eq!(document.content(), "Olá 👋にちは你好");
    }

    #[test]
    fn single_insertion_undo_redo_restores_cursor() {
        let mut document = Document::new();
        document.insert_text("Hello");
        assert!(document.undo());
        assert_eq!(document.content(), "");
        assert_eq!(document.cursor_offset(), 0);
        assert!(document.redo());
        assert_eq!(document.content(), "Hello");
        assert_eq!(document.cursor_offset(), 5);
    }

    #[test]
    fn consecutive_insertions_are_one_undo_group() {
        let mut document = Document::new();
        document.insert_text("Hello");
        document.insert_text(" World");
        assert!(document.undo());
        assert_eq!(document.content(), "");
        assert!(document.redo());
        assert_eq!(document.content(), "Hello World");
    }

    #[test]
    fn edit_type_breaks_undo_group() {
        let mut document = Document::new();
        document.insert_text("Hello");
        document.delete_backward();
        assert!(document.undo());
        assert_eq!(document.content(), "Hello");
        assert!(document.undo());
        assert_eq!(document.content(), "");
    }

    #[test]
    fn new_edit_invalidates_redo() {
        let mut document = Document::new();
        document.insert_text("AB");
        assert!(document.undo());
        document.insert_text("C");
        assert!(!document.redo());
    }

    #[test]
    fn undo_restores_selection_and_redo_restores_cursor() {
        let mut document = Document::from_content("Hello");
        document.set_selection(1, 4);
        document.insert_text("i");
        assert!(document.undo());
        assert_eq!(document.content(), "Hello");
        assert_eq!(document.selection().min_offset(), 1);
        assert_eq!(document.selection().max_offset(), 4);
        assert!(document.redo());
        assert_eq!(document.content(), "Hio");
        assert_eq!(document.cursor_offset(), 2);
    }

    #[test]
    fn dirty_returns_to_pristine_after_undo() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.txt");
        let mut document = Document::new();
        document.save_as(&path).unwrap();
        document.insert_text("dirty");
        assert!(document.is_dirty());
        assert!(document.undo());
        assert!(!document.is_dirty());
    }

    #[test]
    fn save_without_path_is_rejected() {
        let mut document = Document::new();
        assert_eq!(
            document.save().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn real_lf_open_edit_save_reopen() {
        round_trip(LineEnding::Lf);
    }

    #[test]
    fn real_crlf_open_edit_save_reopen() {
        round_trip(LineEnding::CrLf);
    }

    fn round_trip(eol: LineEnding) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.txt");
        fs::write(&path, format!("first{0}second", eol.as_str())).unwrap();
        let mut document = Document::from_file(&path).unwrap();
        assert_eq!(document.line_ending(), eol);
        document.move_cursor(document.len());
        document.insert_newline();
        document.insert_text("third");
        document.save().unwrap();
        let expected = format!("first{0}second{0}third", eol.as_str());
        assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
        assert_eq!(Document::from_file(&path).unwrap().content(), expected);
    }

    #[test]
    fn save_as_updates_path_and_pristine_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new.txt");
        let mut document = Document::from_content("content");
        document.insert_text("!");
        document.save_as(&path).unwrap();
        assert_eq!(document.file_path(), Some(path.as_path()));
        assert_eq!(fs::read_to_string(path).unwrap(), "!content");
        assert!(!document.is_dirty());
    }

    #[test]
    fn predominant_eol_controls_inserted_newlines() {
        let mut document = Document::from_content("a\r\nb\r\nc\nd");
        assert_eq!(document.line_ending(), LineEnding::CrLf);
        document.move_cursor(document.len());
        document.insert_newline();
        assert!(document.content().ends_with("\r\n"));
    }

    #[test]
    fn copy_range_single_line_regression() {
        let mut document = Document::from_content("AAA\nBBB\nCCC");
        document.set_selection(4, 7);
        assert_eq!(document.selected_text().as_deref(), Some("BBB"));
    }

    #[test]
    fn copy_range_partial_line() {
        let mut document = Document::from_content("UserRepository");
        document.set_selection(4, 14);
        assert_eq!(document.selected_text().as_deref(), Some("Repository"));
    }

    #[test]
    fn copy_range_multiline() {
        let mut document = Document::from_content("line1\nline2\nline3\nline4");
        document.set_selection(6, 17);
        assert_eq!(document.selected_text().as_deref(), Some("line2\nline3"));
    }

    #[test]
    fn cut_range_and_undo_restore_exact_text() {
        let mut document = Document::from_content("AAA\nBBB\nCCC");
        document.set_selection(4, 7);
        assert_eq!(document.cut_selection().as_deref(), Some("BBB"));
        assert_eq!(document.content(), "AAA\n\nCCC");
        assert_eq!(document.cursor_offset(), 4);
        assert!(document.undo());
        assert_eq!(document.content(), "AAA\nBBB\nCCC");
    }

    #[test]
    fn paste_at_caret() {
        let mut document = Document::from_content("ac");
        document.move_cursor(1);
        document.paste_text("b");
        assert_eq!(document.content(), "abc");
        assert_eq!(document.cursor_offset(), 2);
    }

    #[test]
    fn paste_replaces_selection() {
        let mut document = Document::from_content("hello world");
        document.set_selection(6, 11);
        document.paste_text("Rust");
        assert_eq!(document.content(), "hello Rust");
    }

    #[test]
    fn unicode_copy_cut_and_paste_are_exact() {
        let mut document = Document::from_content("João ação");
        document.set_selection(6, 12);
        assert_eq!(document.selected_text().as_deref(), Some("ação"));
        assert_eq!(document.cut_selection().as_deref(), Some("ação"));
        assert_eq!(document.content(), "João ");
        document.paste_text("你好");
        assert_eq!(document.content(), "João 你好");
    }

    #[test]
    fn smoke_10k_lines() {
        large_smoke(10_000);
    }

    #[test]
    fn smoke_100k_lines() {
        large_smoke(100_000);
    }

    fn large_smoke(lines: usize) {
        let content: String = (0..lines).map(|i| format!("line {i}\n")).collect();
        let started = Instant::now();
        let mut document = Document::from_content(&content);
        document.move_cursor(0);
        document.insert_text("begin ");
        document.move_cursor(document.len() / 2);
        document.insert_text(" middle ");
        document.move_cursor(document.len());
        document.insert_text(" end");
        document.delete_backward();
        assert!(document.undo());
        assert!(document.redo());
        assert_eq!(document.line_count(), lines + 1);
        assert!(started.elapsed().as_secs() < 30);
    }
}
