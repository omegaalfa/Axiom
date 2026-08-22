use std::{
    cell::RefCell,
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use axiom_editor::Document;
use axiom_index::{ProjectSymbolIndex, ProjectSymbolKind};
use axiom_lsp::{PositionCodec, PositionEncoding, path_to_uri};
use axiom_php::{RuntimeSymbolIndex, SymbolKind as RuntimeKind};
use axiom_project::is_php_file;
use axiom_syntax::PhpSyntax;
use gpui::{
    Action, App, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, KeyDownEvent,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollStrategy, SharedString, Style, TextRun, UTF16Selection, UniformListScrollHandle, Window,
    actions, div, font, prelude::*, px, relative, uniform_list,
};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Diagnostic, DiagnosticSeverity,
    InsertTextFormat, Uri,
};

use crate::{
    lsp_bridge::LspBridge,
    syntax_theme::styled_segment,
    ui::{components::separator, metrics, theme},
};

actions!(
    editor,
    [
        Backspace,
        Delete,
        Enter,
        Tab,
        Outdent,
        Left,
        Right,
        Up,
        Down,
        Home,
        End,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Open,
        Save,
        Complete,
        HoverInfo,
        Definition,
        References,
        Reformat,
        SignatureHelp,
        Escape,
    ]
);

const GUTTER_WIDTH: f32 = 64.0;
const TEXT_PADDING: f32 = 12.0;
const FONT_SIZE: f32 = 14.0;
const LINE_HEIGHT: f32 = 22.0;

struct CachedLineLayout {
    text: String,
    shaped: gpui::ShapedLine,
}

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("backspace", Backspace, Some("Editor")),
        KeyBinding::new("delete", Delete, Some("Editor")),
        KeyBinding::new("enter", Enter, Some("Editor")),
        KeyBinding::new("tab", Tab, Some("Editor")),
        KeyBinding::new("shift-tab", Outdent, Some("Editor")),
        KeyBinding::new("left", Left, Some("Editor")),
        KeyBinding::new("right", Right, Some("Editor")),
        KeyBinding::new("up", Up, Some("Editor")),
        KeyBinding::new("down", Down, Some("Editor")),
        KeyBinding::new("home", Home, Some("Editor")),
        KeyBinding::new("end", End, Some("Editor")),
        KeyBinding::new("shift-left", SelectLeft, Some("Editor")),
        KeyBinding::new("shift-right", SelectRight, Some("Editor")),
        KeyBinding::new("shift-up", SelectUp, Some("Editor")),
        KeyBinding::new("shift-down", SelectDown, Some("Editor")),
        KeyBinding::new("secondary-a", SelectAll, Some("Editor")),
        KeyBinding::new("secondary-c", Copy, Some("Editor")),
        KeyBinding::new("secondary-x", Cut, Some("Editor")),
        KeyBinding::new("secondary-v", Paste, Some("Editor")),
        KeyBinding::new("secondary-z", Undo, Some("Editor")),
        KeyBinding::new("secondary-y", Redo, Some("Editor")),
        KeyBinding::new("secondary-s", Save, Some("Editor")),
        KeyBinding::new("ctrl-space", Complete, Some("Editor")),
        KeyBinding::new("secondary-k", HoverInfo, Some("Editor")),
        KeyBinding::new("secondary-b", Definition, Some("Editor")),
        KeyBinding::new("shift-f12", References, Some("Editor")),
        KeyBinding::new("ctrl-alt-l", Reformat, Some("Editor")),
        KeyBinding::new("secondary-alt-l", Reformat, Some("Editor")),
        KeyBinding::new("ctrl-shift-space", SignatureHelp, Some("Editor")),
        KeyBinding::new("escape", Escape, Some("Editor")),
    ]
}

pub struct EditorView {
    document: Document,
    syntax: Option<PhpSyntax>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    selection_anchor: Option<usize>,
    preferred_x: Option<Pixels>,
    marked_range: Option<Range<usize>>,
    selecting: bool,
    file_path: PathBuf,
    status: Option<SharedString>,
    lsp: Option<Arc<LspBridge>>,
    lsp_uri: Option<Uri>,
    lsp_version: i32,
    last_lsp_text: String,
    completions: Vec<CompletionItem>,
    completion_selected: usize,
    hover_popup: Option<String>,
    diagnostics: Vec<ByteDiagnostic>,
    context_menu: Option<Point<Pixels>>,
    ctrl_hover_range: Option<Range<usize>>,
    line_layouts: RefCell<HashMap<usize, CachedLineLayout>>,
    runtime_symbols: Option<Arc<RuntimeSymbolIndex>>,
    project_symbols: Option<Arc<std::sync::RwLock<ProjectSymbolIndex>>>,
}

#[derive(Clone)]
struct ByteDiagnostic {
    range: Range<usize>,
    severity: Option<DiagnosticSeverity>,
    message: String,
}

impl EditorView {
    pub fn from_document(
        path: PathBuf,
        document: Document,
        lsp: Option<Arc<LspBridge>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let syntax = is_php_file(&path)
            .then(|| PhpSyntax::parse(document.content()))
            .transpose()
            .expect("the PHP grammar and highlight query were validated at startup");
        let last_lsp_text = document.content();
        let lsp_uri = is_php_file(&path)
            .then(|| path_to_uri(&path).ok())
            .flatten();
        if let (Some(lsp), Some(uri)) = (&lsp, &lsp_uri) {
            lsp.with_server(|server| {
                if let Err(error) = server.did_open(uri.clone(), 1, last_lsp_text.clone()) {
                    tracing::warn!("didOpen failed: {error}");
                }
            });
        }
        let mut view = Self {
            document,
            syntax,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            selection_anchor: None,
            preferred_x: None,
            marked_range: None,
            selecting: false,
            file_path: path,
            status: None,
            lsp,
            lsp_uri,
            lsp_version: 1,
            last_lsp_text,
            completions: Vec::new(),
            completion_selected: 0,
            hover_popup: None,
            diagnostics: Vec::new(),
            context_menu: None,
            ctrl_hover_range: None,
            line_layouts: RefCell::new(HashMap::new()),
            runtime_symbols: None,
            project_symbols: None,
        };
        view.sync_syntax();
        view
    }

    pub fn title(&self) -> String {
        self.file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("untitled")
            .to_owned()
    }

    pub fn document_path(&self) -> Option<&Path> {
        self.document.file_path()
    }

    pub fn document_text(&self) -> String {
        self.document.content()
    }

    pub fn is_dirty(&self) -> bool {
        self.document.is_dirty()
    }

    pub fn lsp_uri(&self) -> Option<&Uri> {
        self.lsp_uri.as_ref()
    }

    pub fn current_lsp_position(&self) -> Option<lsp_types::Position> {
        self.lsp_position()
    }

    pub fn reveal_byte_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.move_to(range.start.min(self.document.len()), cx);
        self.marked_range = Some(range);
        self.ensure_cursor_visible();
        cx.notify();
    }

    /// Resolves the token at the caret without contacting the language server.
    /// This is intentionally conservative and only follows indexed classes and
    /// members with an obvious receiver type.
    pub fn native_definition_location(&self) -> Option<(PathBuf, lsp_types::Position)> {
        let syntax = self.syntax.as_ref()?;
        let token = syntax.token_at_byte(self.document.cursor_offset())?;
        let name = token.text.trim_start_matches('$');
        let text_at_cursor = self.document.content();
        if let Some(operator) = text_at_cursor[..token.range.start]
            .rfind("->")
            .or_else(|| text_at_cursor[..token.range.start].rfind("::"))
        {
            let is_static = text_at_cursor[operator..].starts_with("::");
            let owner_end = operator;
            let owner_start = text_at_cursor[..owner_end]
                .char_indices()
                .rev()
                .take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '_' || *ch == '$')
                .last()
                .map_or(owner_end, |(i, _)| i);
            let owner = text_at_cursor[owner_start..owner_end].trim_start_matches('$');
            if let Some(class_fqn) = self.resolve_native_type(owner, &text_at_cursor[..owner_start])
            {
                if let Some(index) = &self.project_symbols
                    && let Ok(index) = index.read()
                {
                    if let Some(symbol) =
                        index.find_methods(&class_fqn).into_iter().find(|symbol| {
                            symbol.name == name
                                && (is_static
                                    == symbol.modifiers.iter().any(|modifier| modifier == "static"))
                        })
                    {
                        let content = if symbol.file == self.file_path {
                            text_at_cursor.clone()
                        } else {
                            std::fs::read_to_string(&symbol.file).ok()?
                        };
                        return Some((
                            symbol.file.clone(),
                            PositionCodec::offset_to_position(
                                &content,
                                symbol.range.start,
                                self.lsp_encoding(),
                            ),
                        ));
                    }
                }
                if let Some(runtime) = &self.runtime_symbols {
                    if let Some(symbol) = runtime
                        .methods_of(&class_fqn)
                        .find(|symbol| symbol.name == name)
                    {
                        let content = if symbol.location.file == self.file_path {
                            text_at_cursor.clone()
                        } else {
                            std::fs::read_to_string(&symbol.location.file).ok()?
                        };
                        return Some((
                            symbol.location.file.clone(),
                            PositionCodec::offset_to_position(
                                &content,
                                symbol.location.range.start,
                                self.lsp_encoding(),
                            ),
                        ));
                    }
                }
            }
        }
        let target = if let Some(index) = &self.project_symbols {
            let index = index.read().ok()?;
            index
                .find_class(name)
                .map(|symbol| (symbol.file.clone(), symbol.range.clone()))
        } else {
            None
        }
        .or_else(|| {
            self.runtime_symbols.as_ref().and_then(|index| {
                index
                    .find_class(name)
                    .map(|symbol| (symbol.location.file.clone(), symbol.location.range.clone()))
            })
        })?;
        let text = if target.0 == self.file_path {
            self.document.content()
        } else {
            std::fs::read_to_string(&target.0).ok()?
        };
        Some((
            target.0,
            PositionCodec::offset_to_position(&text, target.1.start, self.lsp_encoding()),
        ))
    }

    fn lsp_encoding(&self) -> PositionEncoding {
        self.lsp
            .as_ref()
            .map(|lsp| lsp.encoding())
            .unwrap_or_default()
    }

    pub fn set_runtime_symbols(&mut self, symbols: Arc<RuntimeSymbolIndex>) {
        self.runtime_symbols = Some(symbols);
    }

    pub fn set_project_symbols(&mut self, symbols: Arc<std::sync::RwLock<ProjectSymbolIndex>>) {
        self.project_symbols = Some(symbols);
    }

    pub fn close_lsp_document(&self) {
        if let (Some(lsp), Some(uri)) = (&self.lsp, &self.lsp_uri) {
            lsp.with_server(|server| {
                if let Err(error) = server.did_close(uri.clone()) {
                    tracing::warn!("didClose failed: {error}");
                }
            });
        }
    }

    pub fn relocate_path(&mut self, old: &Path, new: &Path) {
        let Ok(relative) = self.file_path.strip_prefix(old) else {
            return;
        };
        self.close_lsp_document();
        let path = new.join(relative);
        self.file_path = path.clone();
        self.document.set_file_path(path.clone());
        self.syntax = is_php_file(&path)
            .then(|| PhpSyntax::parse(self.document.content()))
            .transpose()
            .expect("the PHP grammar and highlight query were validated at startup");
        self.lsp_uri = is_php_file(&path)
            .then(|| path_to_uri(&path).ok())
            .flatten();
        self.lsp_version = 1;
        self.last_lsp_text = self.document.content();
        if let (Some(lsp), Some(uri)) = (&self.lsp, &self.lsp_uri) {
            lsp.with_server(|server| {
                if let Err(error) =
                    server.did_open(uri.clone(), self.lsp_version, self.last_lsp_text.clone())
                {
                    tracing::warn!("didOpen after rename failed: {error}");
                }
            });
        }
    }

    pub fn save_now(&mut self) -> Result<(), String> {
        let result = if self.document.file_path().is_some() {
            self.document.save()
        } else {
            self.document.save_as(&self.file_path)
        };
        result.map_err(|error| error.to_string())?;
        if let (Some(lsp), Some(uri)) = (&self.lsp, &self.lsp_uri) {
            lsp.with_server(|server| {
                if let Err(error) = server.did_save(uri.clone(), None) {
                    tracing::warn!("didSave failed: {error}");
                }
            });
        }
        Ok(())
    }

    fn selected_range(&self) -> Range<usize> {
        self.document
            .selection_offsets()
            .map(|(start, end)| start..end)
            .unwrap_or_else(|| {
                let offset = self.document.cursor_offset();
                offset..offset
            })
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.document.move_cursor(offset);
        self.selection_anchor = None;
        self.preferred_x = None;
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let anchor = *self
            .selection_anchor
            .get_or_insert(self.document.cursor_offset());
        self.document.set_selection(anchor, offset);
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn ensure_cursor_visible(&self) {
        self.scroll.scroll_to_item(
            self.document.line_of_offset(self.document.cursor_offset()),
            ScrollStrategy::Center,
        );
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if let Some((start, _)) = self.document.selection_offsets() {
            start
        } else {
            self.document
                .previous_codepoint_offset(self.document.cursor_offset())
        };
        self.move_to(offset, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if let Some((_, end)) = self.document.selection_offsets() {
            end
        } else {
            self.document
                .next_codepoint_offset(self.document.cursor_offset())
        };
        self.move_to(offset, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(
            self.document
                .previous_codepoint_offset(self.document.cursor_offset()),
            cx,
        );
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(
            self.document
                .next_codepoint_offset(self.document.cursor_offset()),
            cx,
        );
    }

    fn vertical_target(&mut self, delta: isize, window: &mut Window) -> usize {
        let offset = self.document.cursor_offset();
        let line = self.document.line_of_offset(offset);
        let target_line = line
            .saturating_add_signed(delta)
            .min(self.document.line_count() - 1);
        let line_start = self.document.offset_of_line(line);
        let target_start = self.document.offset_of_line(target_line);
        let current_content = self.document.line_content(line);
        let target_content = self.document.line_content(target_line);
        let current = trim_eol(current_content.as_ref());
        let target = trim_eol(target_content.as_ref());
        let x = self.preferred_x.unwrap_or_else(|| {
            let layout = shape(window, current);
            layout.x_for_index((offset - line_start).min(current.len()))
        });
        self.preferred_x = Some(x);
        target_start + shape(window, target).closest_index_for_x(x)
    }

    fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        if !self.completions.is_empty() {
            self.completion_selected = self.completion_selected.saturating_sub(1);
            cx.notify();
            return;
        }
        let offset = self.vertical_target(-1, window);
        self.move_to(offset, cx);
    }

    fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        if !self.completions.is_empty() {
            self.completion_selected =
                (self.completion_selected + 1).min(self.completions.len() - 1);
            cx.notify();
            return;
        }
        let offset = self.vertical_target(1, window);
        self.move_to(offset, cx);
    }

    fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical_target(-1, window);
        self.select_to(offset, cx);
    }

    fn select_down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical_target(1, window);
        self.select_to(offset, cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(
            self.document
                .offset_of_line(self.document.line_of_offset(self.document.cursor_offset())),
            cx,
        );
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.document.line_of_offset(self.document.cursor_offset());
        let start = self.document.offset_of_line(line);
        let len = trim_eol(self.document.line_content(line).as_ref()).len();
        self.move_to(start + len, cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.document.cursor_offset();
        let text = self.document.content();
        if offset > 0 && offset < text.len() {
            let previous = text[..offset].chars().next_back();
            let next = text[offset..].chars().next();
            if matches!(
                (previous, next),
                (Some('{'), Some('}'))
                    | (Some('['), Some(']'))
                    | (Some('('), Some(')'))
                    | (Some('"'), Some('"'))
                    | (Some('\''), Some('\''))
            ) {
                self.document.delete_backward();
                self.document.delete_forward();
                self.after_edit(cx);
                return;
            }
        }
        self.document.delete_backward();
        self.after_edit(cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.document.delete_forward();
        self.after_edit(cx);
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        if !self.completions.is_empty() {
            self.accept_completion(cx);
            return;
        }
        let offset = self.document.cursor_offset();
        let content = self.document.content();
        let next = content[offset..].chars().next();
        let previous = content[..offset].chars().next_back();
        let pair = matches!(
            (previous, next),
            (Some('{'), Some('}')) | (Some('['), Some(']'))
        );
        if pair {
            let inner_indent = self.auto_indent();
            let base_indent = inner_indent.trim_end_matches("    ");
            let insertion = format!("\n{inner_indent}\n{base_indent}");
            self.document.insert_text(&insertion);
            self.document.move_cursor(
                self.document
                    .cursor_offset()
                    .saturating_sub(base_indent.len() + 1),
            );
            self.after_edit(cx);
            return;
        }
        let indent = self.auto_indent();
        self.document.insert_newline();
        if !indent.is_empty() {
            self.document.insert_text(&indent);
        }
        self.after_edit(cx);
    }

    fn auto_indent(&self) -> String {
        let line = self.document.line_of_offset(self.document.cursor_offset());
        let line_content = self.document.line_content(line);
        let current = trim_eol(line_content.as_ref());
        let base = current
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect::<String>();
        let trimmed = current.trim_end();
        let mut indent = base;
        if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
            indent.push_str("    ");
        }
        indent
    }

    fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        if self.document.selection_offsets().is_some() {
            self.transform_selected_lines(true, cx);
        } else {
            self.document.insert_text("    ");
            self.after_edit(cx);
        }
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.transform_selected_lines(false, cx);
    }

    fn transform_selected_lines(&mut self, indent: bool, cx: &mut Context<Self>) {
        let content = self.document.content();
        let (selection_start, selection_end) =
            self.document.selection_offsets().unwrap_or_else(|| {
                let cursor = self.document.cursor_offset();
                (cursor, cursor)
            });
        let start_line = self.document.line_of_offset(selection_start);
        let mut end_line = self.document.line_of_offset(selection_end);
        if selection_end > selection_start
            && selection_end == self.document.offset_of_line(end_line)
        {
            end_line = end_line.saturating_sub(1);
        }
        let replace_start = self.document.offset_of_line(start_line);
        let replace_end = if end_line + 1 < self.document.line_count() {
            self.document.offset_of_line(end_line + 1)
        } else {
            content.len()
        };
        let original = &content[replace_start..replace_end];
        let mut transformed = String::with_capacity(original.len() + 16);
        for segment in original.split_inclusive('\n') {
            let (line, newline) = segment
                .strip_suffix('\n')
                .map_or((segment, ""), |line| (line, "\n"));
            if indent {
                transformed.push_str("    ");
                transformed.push_str(line);
            } else {
                let remove = line
                    .as_bytes()
                    .iter()
                    .take(4)
                    .take_while(|byte| **byte == b' ')
                    .count();
                transformed.push_str(&line[remove..]);
            }
            transformed.push_str(newline);
        }
        if transformed == original {
            return;
        }
        self.document.set_selection(replace_start, replace_end);
        self.document.insert_text(&transformed);
        self.document
            .set_selection(replace_start, replace_start + transformed.len());
        self.after_edit(cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection_anchor = Some(0);
        self.document.select_all();
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.document.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.document.cut_selection() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.after_edit(cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.document.paste_text(&text);
            self.after_edit(cx);
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.document.undo();
        self.after_edit(cx);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.document.redo();
        self.after_edit(cx);
    }

    fn open(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        match Document::from_file(&self.file_path) {
            Ok(document) => {
                self.document = document;
                match is_php_file(&self.file_path)
                    .then(|| PhpSyntax::parse(self.document.content()))
                    .transpose()
                {
                    Ok(syntax) => {
                        self.syntax = syntax;
                        self.sync_lsp();
                        self.status = Some(format!("{} aberto", self.title()).into());
                    }
                    Err(error) => {
                        self.status =
                            Some(format!("Falha ao analisar {}: {error}", self.title()).into());
                    }
                }
            }
            Err(error) => {
                self.status = Some(format!("Falha ao abrir {}: {error}", self.title()).into())
            }
        }
        cx.notify();
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        let result = self.save_now();
        self.status = Some(match result {
            Ok(()) => format!("{} salvo", self.title()).into(),
            Err(error) => format!("Falha ao salvar {}: {error}", self.title()).into(),
        });
        cx.notify();
    }

    fn debug_keydown(&mut self, event: &KeyDownEvent, _: &mut Window, _: &mut Context<Self>) {
        if std::env::var_os("AXIOM_DEBUG_KEYS").is_some_and(|value| {
            !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
        }) {
            tracing::debug!(
                key = %event.keystroke.key,
                ctrl = event.keystroke.modifiers.control,
                alt = event.keystroke.modifiers.alt,
                shift = event.keystroke.modifiers.shift,
                context = "editor",
                "Axiom key event"
            );
        }
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.sync_syntax();
        self.sync_lsp();
        self.maybe_trigger_completion();
        let native = self.native_completions();
        if native.is_empty() {
            self.completions.clear();
        } else {
            self.set_completions(native, cx);
        }
        self.selection_anchor = None;
        self.preferred_x = None;
        self.marked_range = None;
        self.ctrl_hover_range = None;
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn maybe_trigger_completion(&self) {
        let Some((lsp, uri, position)) = self
            .lsp
            .as_ref()
            .zip(self.lsp_uri.as_ref())
            .zip(self.lsp_position())
            .map(|((lsp, uri), position)| (lsp, uri, position))
        else {
            return;
        };
        let text = self.document.content();
        let tail = &text[..self.document.cursor_offset().min(text.len())];
        let relevant = tail.ends_with("->")
            || tail.ends_with("::")
            || tail.ends_with("new ")
            || tail.ends_with("extends ")
            || tail.ends_with("implements ")
            || tail.ends_with("use ");
        if relevant {
            lsp.request_completion(uri.clone(), position);
        }
    }

    fn sync_syntax(&mut self) {
        let text = self.document.content();
        if let Some(syntax) = &mut self.syntax
            && let Err(error) = syntax.update_text(&text)
        {
            self.status = Some(format!("Falha ao atualizar sintaxe PHP: {error}").into());
        }
        self.diagnostics = self
            .syntax
            .as_ref()
            .map(|syntax| {
                syntax
                    .diagnostics()
                    .into_iter()
                    .map(|diagnostic| ByteDiagnostic {
                        range: diagnostic.range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: diagnostic.message,
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.add_native_inspections(&text);
    }

    fn add_native_inspections(&mut self, text: &str) {
        let Some(index) = &self.project_symbols else {
            return;
        };
        let Ok(index) = index.read() else { return };
        if !index.is_ready() {
            return;
        }
        let mut offset = 0;
        while let Some(relative) = text[offset..].find("new ") {
            let start = offset + relative + 4;
            let end = start
                + text[start..]
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '\\')
                    .map(char::len_utf8)
                    .sum::<usize>();
            if end > start {
                let name = text[start..end].trim_start_matches('\\');
                let known_project = index.find_class(name).is_some();
                let known_runtime = self
                    .runtime_symbols
                    .as_ref()
                    .is_some_and(|runtime| runtime.find_class(name).is_some());
                if !known_project && !known_runtime && !matches!(name, "self" | "static" | "parent")
                {
                    self.diagnostics.push(ByteDiagnostic {
                        range: start..end,
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: format!("Unknown class '{name}'"),
                    });
                }
            }
            offset = end.max(start + 1);
            if offset >= text.len() {
                break;
            }
        }
        for symbol in index
            .symbols()
            .iter()
            .filter(|symbol| symbol.file == self.file_path)
        {
            if index.symbols().iter().any(|other| {
                other.file != symbol.file
                    && other.fully_qualified_name == symbol.fully_qualified_name
                    && other.kind == symbol.kind
            }) {
                self.diagnostics.push(ByteDiagnostic {
                    range: symbol.range.clone(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: format!("Duplicate class {}", symbol.fully_qualified_name),
                });
            }
        }
        let constant_names: Vec<(String, String)> = index
            .symbols()
            .iter()
            .filter(|symbol| {
                matches!(
                    symbol.kind,
                    axiom_index::ProjectSymbolKind::Constant
                        | axiom_index::ProjectSymbolKind::ClassConstant
                )
            })
            .map(|symbol| (symbol.name.clone(), symbol.fully_qualified_name.clone()))
            .collect();
        drop(index);
        let runtime_symbols = self.runtime_symbols.clone();
        self.add_unknown_constant_inspections(text, &constant_names, runtime_symbols.as_ref());
    }

    fn add_unknown_constant_inspections(
        &mut self,
        text: &str,
        constants: &[(String, String)],
        runtime_symbols: Option<&Arc<RuntimeSymbolIndex>>,
    ) {
        const BUILT_INS: &[&str] = &[
            "PHP_VERSION",
            "PHP_VERSION_ID",
            "PHP_INT_MAX",
            "PHP_INT_MIN",
            "PHP_INT_SIZE",
            "PHP_OS",
            "PHP_OS_FAMILY",
            "DIRECTORY_SEPARATOR",
            "PATH_SEPARATOR",
            "E_ERROR",
            "E_WARNING",
            "E_PARSE",
            "E_NOTICE",
        ];
        let mut offset = 0;
        while let Some(relative) = text[offset..].find("echo ") {
            let start = offset + relative + 5;
            let name_end = start
                + text[start..]
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                    .map(char::len_utf8)
                    .sum::<usize>();
            let name = &text[start..name_end];
            if !name.is_empty()
                && name
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
                && !BUILT_INS.iter().any(|builtin| builtin == &name)
            {
                let declared = constants.iter().any(|(symbol_name, fqn)| {
                    symbol_name == name || fqn.ends_with(&format!("\\{name}"))
                });
                let runtime =
                    runtime_symbols.is_some_and(|symbols| symbols.find_constant(name).is_some());
                if !declared && !runtime {
                    self.diagnostics.push(ByteDiagnostic {
                        range: start..name_end,
                        severity: Some(DiagnosticSeverity::WARNING),
                        message: format!("Undefined constant '{name}'"),
                    });
                }
            }
            offset = name_end.max(start + 1);
            if offset >= text.len() {
                break;
            }
        }
    }

    fn sync_lsp(&mut self) {
        let text = self.document.content();
        if text == self.last_lsp_text {
            return;
        }
        self.last_lsp_text.clone_from(&text);
        self.lsp_version = self.lsp_version.saturating_add(1);
        if let (Some(lsp), Some(uri)) = (&self.lsp, &self.lsp_uri) {
            lsp.with_server(|server| {
                if let Err(error) = server.did_change(uri.clone(), self.lsp_version, text) {
                    tracing::warn!("didChange failed: {error}");
                }
            });
        }
    }

    fn lsp_position(&self) -> Option<lsp_types::Position> {
        let lsp = self.lsp.as_ref()?;
        Some(PositionCodec::offset_to_position(
            &self.document.content(),
            self.document.cursor_offset(),
            lsp.encoding(),
        ))
    }

    fn complete(&mut self, _: &Complete, _: &mut Window, cx: &mut Context<Self>) {
        self.completions.clear();
        let native = self.native_completions();
        if let (Some(lsp), Some(uri), Some(position)) =
            (&self.lsp, &self.lsp_uri, self.lsp_position())
        {
            lsp.request_completion(uri.clone(), position);
        }
        if !native.is_empty() {
            self.set_completions(native, cx);
        } else if self.lsp.is_none() {
            self.status = Some("Completion unavailable (no PHP index or language server)".into());
            cx.notify();
        }
    }

    fn native_completions(&self) -> Vec<CompletionItem> {
        let text = self.document.content();
        let cursor = self.document.cursor_offset().min(text.len());
        let before = &text[..cursor];
        let member_operator = before
            .char_indices()
            .rev()
            .find_map(|(index, ch)| (ch == '>' || ch == ':').then_some((index, ch)))
            .and_then(|(index, ch)| {
                if ch == '>' && before[..index].ends_with('-') {
                    Some((index.saturating_sub(1), false))
                } else if ch == ':' && before[..index].ends_with(':') {
                    Some((index.saturating_sub(1), true))
                } else {
                    None
                }
            });
        let start = text[..cursor]
            .char_indices()
            .rev()
            .take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '_')
            .last()
            .map_or(cursor, |(i, _)| i);
        let prefix = &text[start..cursor];
        if prefix.len() < 2 {
            return Vec::new();
        }
        if let Some((operator_start, is_static)) = member_operator {
            let owner_end = operator_start;
            let owner_start = text[..owner_end]
                .char_indices()
                .rev()
                .take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '_' || *ch == '$')
                .last()
                .map_or(owner_end, |(i, _)| i);
            let owner = text[owner_start..owner_end].trim_start_matches('$');
            if let Some(class_fqn) = self.resolve_native_type(owner, &text[..owner_start]) {
                let mut members = Vec::new();
                if let Some(index) = &self.project_symbols
                    && let Ok(index) = index.read()
                {
                    members.extend(
                        index
                            .find_methods(&class_fqn)
                            .into_iter()
                            .filter(|symbol| {
                                symbol.name.starts_with(prefix)
                                    && (is_static == symbol.modifiers.iter().any(|m| m == "static"))
                                    && (is_static
                                        || symbol.visibility != axiom_index::Visibility::Private)
                            })
                            .map(|symbol| CompletionItem {
                                label: symbol.name.clone(),
                                detail: Some(format!("{:?} • Project", symbol.kind)),
                                kind: Some(CompletionItemKind::METHOD),
                                ..Default::default()
                            }),
                    );
                }
                if let Some(index) = &self.runtime_symbols {
                    members.extend(
                        index
                            .methods_of(&class_fqn)
                            .filter(|symbol| {
                                symbol.name.starts_with(prefix)
                                    && (is_static || !symbol.name.starts_with('_'))
                            })
                            .map(|symbol| CompletionItem {
                                label: symbol.name.clone(),
                                detail: Some(runtime_signature_detail(symbol)),
                                kind: Some(CompletionItemKind::METHOD),
                                ..Default::default()
                            }),
                    );
                }
                let mut seen = std::collections::HashSet::new();
                members.retain(|item| seen.insert(item.label.to_ascii_lowercase()));
                return members.into_iter().take(40).collect();
            }
        }
        let mut items = self
            .runtime_symbols
            .as_ref()
            .map(|index| {
                index
                    .search_prefix(prefix)
                    .into_iter()
                    .take(40)
                    .map(|symbol| CompletionItem {
                        label: symbol.name.clone(),
                        detail: Some(format!("{:?} • PHP Runtime", symbol.kind)),
                        kind: Some(match symbol.kind {
                            RuntimeKind::Function => CompletionItemKind::FUNCTION,
                            RuntimeKind::Class
                            | RuntimeKind::Interface
                            | RuntimeKind::Trait
                            | RuntimeKind::Enum => CompletionItemKind::CLASS,
                            _ => CompletionItemKind::VALUE,
                        }),
                        ..Default::default()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(index) = &self.project_symbols
            && let Ok(index) = index.read()
        {
            items.extend(index.search_prefix(prefix).into_iter().map(|symbol| {
                let import = matches!(
                    symbol.kind,
                    ProjectSymbolKind::Class
                        | ProjectSymbolKind::Interface
                        | ProjectSymbolKind::Trait
                        | ProjectSymbolKind::Enum
                )
                .then(|| self.composer_import_edit(&symbol.fully_qualified_name))
                .flatten();
                CompletionItem {
                    label: symbol.name.clone(),
                    detail: Some(format!("{} • Project", symbol.fully_qualified_name)),
                    kind: Some(match symbol.kind {
                        ProjectSymbolKind::Function => CompletionItemKind::FUNCTION,
                        ProjectSymbolKind::Method => CompletionItemKind::METHOD,
                        ProjectSymbolKind::Class
                        | ProjectSymbolKind::Interface
                        | ProjectSymbolKind::Trait
                        | ProjectSymbolKind::Enum => CompletionItemKind::CLASS,
                        _ => CompletionItemKind::VALUE,
                    }),
                    additional_text_edits: import.map(|edit| vec![edit]),
                    ..Default::default()
                }
            }));
        }
        let mut seen = std::collections::HashSet::new();
        items.retain(|item| {
            seen.insert(format!(
                "{}:{}",
                item.label.to_ascii_lowercase(),
                item.detail.as_deref().unwrap_or_default()
            ))
        });
        items.into_iter().take(40).collect()
    }

    /// Builds a single additional edit for a Composer/project class. The edit
    /// is deliberately narrow: it only inserts a missing `use` statement and
    /// never rewrites or reformats the document.
    fn composer_import_edit(&self, fqn: &str) -> Option<lsp_types::TextEdit> {
        let fqn = fqn.trim_start_matches('\\');
        let text = self.document.content();
        let current_namespace = text.lines().find_map(|line| {
            let line = line.trim();
            line.strip_prefix("namespace ")
                .and_then(|value| value.trim_end_matches(';').split_whitespace().next())
                .map(|value| value.trim_matches('\\').to_owned())
        });
        let short = fqn.rsplit('\\').next().unwrap_or(fqn);
        if current_namespace
            .as_deref()
            .is_some_and(|namespace| fqn.strip_suffix(&format!("\\{short}")) == Some(namespace))
        {
            return None;
        }
        for line in text.lines() {
            let normalized = line.trim().trim_end_matches(';');
            let use_name = normalized
                .strip_prefix("use ")
                .and_then(|value| value.split_whitespace().next())
                .map(|value| value.trim_matches('\\'));
            if use_name == Some(fqn) || normalized.ends_with(&format!("\\{short} as {short}")) {
                return None;
            }
        }
        let mut last_use_end = None;
        let mut namespace_end = None;
        for (offset, line) in text.split_inclusive('\n').scan(0usize, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        }) {
            let trimmed = line.trim();
            if trimmed.starts_with("use ") && trimmed.ends_with(';') {
                last_use_end = Some(offset + line.len());
            }
            if trimmed.starts_with("namespace ") && trimmed.ends_with(';') {
                namespace_end = Some(offset + line.len());
            }
        }
        let insertion = last_use_end.or(namespace_end).unwrap_or_else(|| {
            text.find("<?php")
                .map(|pos| {
                    text[pos..]
                        .find('\n')
                        .map(|n| pos + n + 1)
                        .unwrap_or(text.len())
                })
                .unwrap_or(0)
        });
        let prefix = if insertion > 0 && !text[..insertion].ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let new_text = format!("{prefix}use {fqn};\n");
        let position = PositionCodec::offset_to_position(
            &text,
            insertion,
            self.lsp
                .as_ref()
                .map(|lsp| lsp.encoding())
                .unwrap_or_default(),
        );
        Some(lsp_types::TextEdit {
            range: lsp_types::Range::new(position, position),
            new_text,
        })
    }

    fn resolve_native_type(&self, owner: &str, context: &str) -> Option<String> {
        let mut candidate = owner.trim().trim_start_matches('$').to_owned();
        if candidate == "this" {
            candidate = self.file_path.file_stem()?.to_string_lossy().into_owned();
        }
        let variable = format!("${candidate}");
        let patterns = [
            format!("{variable} = new "),
            format!("{variable}=new "),
            format!("{variable}: "),
            format!("{variable} : "),
        ];
        for pattern in patterns {
            if let Some(pos) = context.rfind(&pattern) {
                let tail = &context[pos + pattern.len()..];
                let name: String = tail
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '\\')
                    .collect();
                if !name.is_empty() {
                    return Some(self.qualify_type(&name));
                }
            }
        }
        if let Some(pos) = context.rfind(&format!("${candidate}")) {
            let tail = &context[pos.saturating_sub(120)..pos];
            if let Some(colon) = tail.rfind(' ') {
                let ty: String = tail[colon + 1..]
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '\\')
                    .collect();
                if !ty.is_empty() {
                    return Some(self.qualify_type(&ty));
                }
            }
        }
        let lower = candidate.to_ascii_lowercase();
        if self.runtime_symbols.as_ref().is_some_and(|index| {
            index.find_class(&candidate).is_some()
                || index.find_class(&format!("\\{candidate}")).is_some()
        }) {
            return Some(candidate);
        }
        self.project_symbols
            .as_ref()
            .and_then(|index| index.read().ok())
            .and_then(|index| {
                index
                    .find_class(&candidate)
                    .map(|symbol| symbol.fully_qualified_name.clone())
            })
            .or_else(|| (!lower.is_empty()).then_some(candidate))
    }

    fn qualify_type(&self, name: &str) -> String {
        if name.starts_with('\\') || name.contains('\\') {
            return name.to_owned();
        }
        self.project_symbols
            .as_ref()
            .and_then(|index| index.read().ok())
            .and_then(|index| {
                index
                    .find_class(name)
                    .map(|symbol| symbol.fully_qualified_name.clone())
            })
            .unwrap_or_else(|| name.to_owned())
    }

    fn hover_info(&mut self, _: &HoverInfo, _: &mut Window, _: &mut Context<Self>) {
        if let (Some(lsp), Some(uri), Some(position)) =
            (&self.lsp, &self.lsp_uri, self.lsp_position())
        {
            lsp.request_hover(uri.clone(), position);
        }
    }

    fn definition(&mut self, _: &Definition, _: &mut Window, _: &mut Context<Self>) {
        if let (Some(lsp), Some(uri), Some(position)) =
            (&self.lsp, &self.lsp_uri, self.lsp_position())
        {
            lsp.request_definition(uri.clone(), position);
        }
    }

    fn references(&mut self, _: &References, _: &mut Window, _: &mut Context<Self>) {
        if let (Some(lsp), Some(uri), Some(position)) =
            (&self.lsp, &self.lsp_uri, self.lsp_position())
        {
            lsp.request_references(uri.clone(), position);
        }
    }

    fn reformat(&mut self, _: &Reformat, _: &mut Window, cx: &mut Context<Self>) {
        if let (Some(lsp), Some(uri)) = (&self.lsp, &self.lsp_uri)
            && lsp.status() == axiom_lsp::ServerStatus::Ready
        {
            tracing::info!("[FORMAT] provider=lsp");
            lsp.request_formatting(uri.clone(), 4, true);
        } else {
            let formatted = native_format_php(&self.document.content());
            if formatted != self.document.content() {
                let cursor = self.document.cursor_offset();
                self.document.select_all();
                self.document.insert_text(&formatted);
                self.document.move_cursor(cursor.min(formatted.len()));
                self.after_edit(cx);
                self.status = Some("Formatted with Axiom PHP Formatter".into());
                tracing::info!("[FORMAT] provider=axiom-native");
            } else {
                self.status = Some("Axiom PHP Formatter: no changes".into());
                tracing::info!("[FORMAT] provider=axiom-native changes=0");
            }
            cx.notify();
        }
    }

    fn signature_help(&mut self, _: &SignatureHelp, _: &mut Window, _: &mut Context<Self>) {
        if let (Some(lsp), Some(uri), Some(position)) =
            (&self.lsp, &self.lsp_uri, self.lsp_position())
        {
            lsp.request_signature_help(uri.clone(), position);
        }
    }

    pub fn apply_formatting(&mut self, edits: &[lsp_types::TextEdit], cx: &mut Context<Self>) {
        if edits.is_empty() {
            self.status = Some("Formatter returned no changes".into());
            cx.notify();
            return;
        }
        let encoding = self
            .lsp
            .as_ref()
            .map(|lsp| lsp.encoding())
            .unwrap_or_default();
        let formatted = axiom_lsp::apply_text_edits(&self.document.content(), edits, encoding);
        let cursor = self.document.cursor_offset();
        self.document.select_all();
        self.document.insert_text(&formatted);
        self.document.move_cursor(cursor.min(formatted.len()));
        self.after_edit(cx);
        self.status = Some("Code reformatted".into());
    }

    pub fn set_signature_help(&mut self, text: Option<String>, cx: &mut Context<Self>) {
        self.hover_popup = text;
        cx.notify();
    }

    fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        self.completions.clear();
        self.hover_popup = None;
        self.context_menu = None;
        self.ctrl_hover_range = None;
        cx.notify();
    }

    fn accept_completion(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.completions.get(self.completion_selected).cloned() else {
            return;
        };
        let (range, mut text) = match item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => {
                (self.lsp_range_to_bytes(edit.range), edit.new_text)
            }
            Some(CompletionTextEdit::InsertAndReplace(edit)) => {
                (self.lsp_range_to_bytes(edit.replace), edit.new_text)
            }
            None => (
                self.selected_range(),
                item.insert_text.unwrap_or_else(|| item.label.clone()),
            ),
        };
        if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
            text = strip_snippet_placeholders(&text);
        }
        let mut edits = item.additional_text_edits.unwrap_or_default();
        let encoding = self
            .lsp
            .as_ref()
            .map(|lsp| lsp.encoding())
            .unwrap_or_default();
        let content = self.document.content();
        let main_range = lsp_types::Range::new(
            PositionCodec::offset_to_position(&content, range.start, encoding),
            PositionCodec::offset_to_position(&content, range.end, encoding),
        );
        edits.push(lsp_types::TextEdit {
            range: main_range,
            new_text: text,
        });
        let updated = axiom_lsp::apply_text_edits(&content, &edits, encoding);
        self.document.select_all();
        self.document.insert_text(&updated);
        self.completions.clear();
        self.after_edit(cx);
    }

    fn lsp_range_to_bytes(&self, range: lsp_types::Range) -> Range<usize> {
        let text = self.document.content();
        let encoding = self
            .lsp
            .as_ref()
            .map(|lsp| lsp.encoding())
            .unwrap_or_default();
        PositionCodec::position_to_offset(&text, range.start, encoding)
            ..PositionCodec::position_to_offset(&text, range.end, encoding)
    }

    pub fn set_completions(&mut self, items: Vec<CompletionItem>, cx: &mut Context<Self>) {
        let mut merged = self.completions.clone();
        merged.extend(items);
        let mut seen = std::collections::HashSet::new();
        merged.retain(|item| seen.insert(item.label.to_ascii_lowercase()));
        self.completions = merged;
        self.completion_selected = 0;
        cx.notify();
    }

    pub fn set_hover(&mut self, text: Option<String>, cx: &mut Context<Self>) {
        self.hover_popup = text;
        cx.notify();
    }

    pub fn set_diagnostics(
        &mut self,
        version: Option<i32>,
        diagnostics: Vec<Diagnostic>,
        cx: &mut Context<Self>,
    ) {
        if version.is_some_and(|version| version < self.lsp_version) {
            return;
        }
        self.diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| ByteDiagnostic {
                range: self.lsp_range_to_bytes(diagnostic.range),
                severity: diagnostic.severity,
                message: diagnostic.message,
            })
            .collect();
        cx.notify();
    }

    pub fn reveal_lsp_position(&mut self, position: lsp_types::Position, cx: &mut Context<Self>) {
        let text = self.document.content();
        let encoding = self
            .lsp
            .as_ref()
            .map(|lsp| lsp.encoding())
            .unwrap_or_default();
        let offset = PositionCodec::position_to_offset(&text, position, encoding);
        self.move_to(offset, cx);
    }

    fn mouse_offset(&self, line: usize, x: Pixels, window: &mut Window) -> usize {
        let text = self.document.line_content(line);
        let text = trim_eol(text.as_ref());
        let viewport_left = self.scroll.0.borrow().base_handle.bounds().left();
        let local_x = px(axiom_app::interaction::text_local_x(
            x.into(),
            viewport_left.into(),
            GUTTER_WIDTH + TEXT_PADDING,
        ));
        self.document.offset_of_line(line)
            + self
                .line_layout(line, text, window)
                .closest_index_for_x(local_x)
    }

    fn line_layout(&self, line: usize, text: &str, window: &mut Window) -> gpui::ShapedLine {
        let mut layouts = self.line_layouts.borrow_mut();
        let layout = layouts.entry(line).or_insert_with(|| CachedLineLayout {
            text: text.to_owned(),
            shaped: shape(window, text),
        });
        if layout.text != text {
            layout.text = text.to_owned();
            layout.shaped = shape(window, text);
        }
        layout.shaped.clone()
    }

    fn mouse_line(&self, position: Point<Pixels>) -> usize {
        let scroll = self.scroll.0.borrow();
        let bounds = scroll.base_handle.bounds();
        axiom_app::interaction::viewport_y_to_line(
            position.y.into(),
            bounds.top().into(),
            scroll.base_handle.offset().y.into(),
            LINE_HEIGHT,
            self.document.line_count(),
        )
    }

    fn is_in_text_area(&self, position: Point<Pixels>) -> bool {
        let bounds = self.scroll.0.borrow().base_handle.bounds();
        position.x >= bounds.left() + px(GUTTER_WIDTH)
            && position.y >= bounds.top()
            && position.y <= bounds.bottom()
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_in_text_area(event.position) {
            return;
        }
        let line = self.mouse_line(event.position);
        self.ctrl_hover_range = None;
        let offset = self.mouse_offset(line, event.position.x, window);
        if event.modifiers.control && self.lsp.is_some() {
            self.move_to(offset, cx);
            self.selecting = false;
            window.dispatch_action(Definition.boxed_clone(), cx);
            return;
        }
        self.selecting = true;
        self.context_menu = None;
        self.completions.clear();
        self.hover_popup = None;
        window.focus(&self.focus);
        if event.click_count >= 3 {
            let start = self.document.offset_of_line(line);
            let end = start + self.document.line_content(line).len();
            self.document.set_selection(start, end);
            self.selection_anchor = Some(start);
            cx.notify();
        } else if event.click_count == 2 {
            let content = self.document.content();
            let range = axiom_app::interaction::word_range_at(&content, offset);
            self.document.set_selection(range.start, range.end);
            self.selection_anchor = Some(range.start);
            cx.notify();
        } else if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
            self.selection_anchor = Some(offset);
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.ctrl_hover_range = None;
            self.autoscroll_drag(event.position);
            let line = self.mouse_line(event.position);
            let offset = self.mouse_offset(line, event.position.x, window);
            self.select_to(offset, cx);
            return;
        }
        if !event.modifiers.control || !self.is_in_text_area(event.position) {
            if self.ctrl_hover_range.take().is_some() {
                cx.notify();
            }
            if self.is_in_text_area(event.position) {
                let line = self.mouse_line(event.position);
                let offset = self.mouse_offset(line, event.position.x, window);
                let next = self
                    .diagnostics
                    .iter()
                    .find(|diagnostic| diagnostic.range.contains(&offset))
                    .map(|diagnostic| format!("{}\nAxiom PHP Parser", diagnostic.message));
                if self.hover_popup != next {
                    self.hover_popup = next;
                    cx.notify();
                }
            }
            return;
        }
        self.hover_popup = None;
        let line = self.mouse_line(event.position);
        let offset = self.mouse_offset(line, event.position.x, window);
        let next = self.syntax.as_ref().and_then(|syntax| {
            let token = syntax.token_at_byte(offset)?;
            let interesting = matches!(
                token.kind.as_str(),
                "name" | "qualified_name" | "variable_name" | "member_name"
            ) && token
                .text
                .chars()
                .any(|ch| ch.is_alphanumeric() || ch == '_');
            interesting.then_some(token.range)
        });
        if self.ctrl_hover_range != next {
            self.ctrl_hover_range = next;
            cx.notify();
        }
    }

    fn autoscroll_drag(&self, position: Point<Pixels>) {
        let scroll = self.scroll.0.borrow();
        let handle = scroll.base_handle.clone();
        let bounds = handle.bounds();
        let current = handle.offset();
        let maximum = handle.max_offset().height;
        drop(scroll);
        let delta = if position.y < bounds.top() {
            (bounds.top() - position.y) * 0.35
        } else if position.y > bounds.bottom() {
            -(position.y - bounds.bottom()) * 0.35
        } else {
            return;
        };
        handle.set_offset(gpui::point(
            current.x,
            (current.y + delta).max(-maximum).min(px(0.)),
        ));
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_in_text_area(event.position) {
            return;
        }
        let line = self.mouse_line(event.position);
        window.focus(&self.focus);
        let offset = self.mouse_offset(line, event.position.x, window);
        let inside_selection = self
            .document
            .selection_offsets()
            .is_some_and(|(start, end)| start <= offset && offset < end);
        if !inside_selection {
            self.document.move_cursor(offset);
            self.selection_anchor = None;
        }
        let bounds = self.scroll.0.borrow().base_handle.bounds();
        self.context_menu = Some(gpui::point(
            (event.position.x - bounds.left()).max(px(0.)),
            (event.position.y - bounds.top()).max(px(0.)),
        ));
        self.completions.clear();
        self.hover_popup = None;
        self.ctrl_hover_range = None;
        cx.notify();
    }

    fn render_line(&self, line: usize, window: &mut Window) -> gpui::AnyElement {
        let t = theme();
        let raw = self.document.line_content(line);
        let text = trim_eol(raw.as_ref()).to_owned();
        let start = self.document.offset_of_line(line);
        let end = start + text.len();
        let selection = self.document.selection_offsets();
        let cursor = self.document.cursor_offset();
        let cursor_here = self.document.line_of_offset(cursor) == line;
        let selected_start = selection.map_or(end, |(a, _)| a.max(start).min(end)) - start;
        let selected_end = selection.map_or(end, |(_, b)| b.max(start).min(end)) - start;
        let cursor_column = cursor.saturating_sub(start).min(text.len());
        let hover = self.ctrl_hover_range.as_ref().and_then(|range| {
            (range.end > start && range.start < end).then(|| {
                let from = range.start.max(start) - start;
                let to = range.end.min(end) - start;
                let layout = self.line_layout(line, &text, window);
                (layout.x_for_index(from), layout.x_for_index(to))
            })
        });
        let highlights: Vec<_> = self
            .syntax
            .as_ref()
            .into_iter()
            .flat_map(|syntax| syntax.highlights_in(start..end))
            .cloned()
            .collect();
        let diagnostic = self
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.range.end > start && diagnostic.range.start < end);
        let diagnostic_underline = diagnostic.map(|diagnostic| {
            let from = diagnostic.range.start.max(start) - start;
            let to = diagnostic.range.end.min(end) - start;
            let layout = self.line_layout(line, &text, window);
            (
                layout.x_for_index(from),
                layout.x_for_index(to),
                diagnostic.severity,
            )
        });
        let content = if selected_start < selected_end {
            div()
                .flex()
                .h_full()
                .child(styled_segment(
                    text[..selected_start].to_owned(),
                    start,
                    &highlights,
                ))
                .child(div().h_full().bg(t.selection).child(styled_segment(
                    text[selected_start..selected_end].to_owned(),
                    start + selected_start,
                    &highlights,
                )))
                .child(styled_segment(
                    text[selected_end..].to_owned(),
                    start + selected_end,
                    &highlights,
                ))
        } else if cursor_here {
            let layout = self.line_layout(line, &text, window);
            let caret_x = layout.x_for_index(cursor_column);
            let metrics = shape(window, "M");
            let caret_height = (metrics.ascent + metrics.descent).min(px(LINE_HEIGHT));
            div()
                .relative()
                .h_full()
                .child(styled_segment(text, start, &highlights))
                .child(
                    div()
                        .absolute()
                        .left(caret_x)
                        .top((px(LINE_HEIGHT) - caret_height) / 2.)
                        .w(px(1.))
                        .h(caret_height)
                        .bg(t.text_primary),
                )
        } else {
            div()
                .flex()
                .h_full()
                .child(styled_segment(text, start, &highlights))
        };

        div()
            .id(line)
            .relative()
            .flex()
            .h(px(LINE_HEIGHT))
            .line_height(px(LINE_HEIGHT))
            .text_size(px(FONT_SIZE))
            .font_family("Cascadia Mono")
            .text_color(t.text_primary)
            .bg(if cursor_here {
                t.active_line
            } else {
                t.editor_background
            })
            .child(
                div()
                    .w(px(GUTTER_WIDTH))
                    .pr_3()
                    .bg(t.gutter_background)
                    .border_r_1()
                    .border_color(t.border_subtle)
                    .cursor(CursorStyle::Arrow)
                    .text_right()
                    .text_color(
                        if diagnostic.is_some_and(|diagnostic| {
                            diagnostic.severity == Some(DiagnosticSeverity::ERROR)
                        }) {
                            t.error
                        } else if diagnostic.is_some() {
                            t.warning
                        } else if cursor_here {
                            t.text_secondary
                        } else {
                            t.gutter_text
                        },
                    )
                    .child(format!(
                        "{}{}",
                        if diagnostic.is_some() { "● " } else { "" },
                        line + 1
                    )),
            )
            .child(div().flex_1().h_full().pl_3().child(content))
            .when_some(hover, |this, (from, to)| {
                this.child(
                    div()
                        .absolute()
                        .left(px(GUTTER_WIDTH) + px(TEXT_PADDING) + from)
                        .top(px(LINE_HEIGHT - 2.))
                        .w((to - from).max(px(1.)))
                        .h(px(1.))
                        .bg(t.accent),
                )
            })
            .when_some(diagnostic_underline, |this, (from, to, severity)| {
                this.child(
                    div()
                        .absolute()
                        .left(px(GUTTER_WIDTH) + px(TEXT_PADDING) + from)
                        .top(px(LINE_HEIGHT - 3.))
                        .w((to - from).max(px(1.)))
                        .h(px(1.))
                        .bg(if severity == Some(DiagnosticSeverity::ERROR) {
                            t.error
                        } else {
                            t.warning
                        }),
                )
            })
            .into_any_element()
    }

    fn context_action(
        label: &'static str,
        action: impl Action,
        focus: FocusHandle,
        enabled: bool,
    ) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        div()
            .id(SharedString::from(format!("context-action-{label}")))
            .h(m.toolbar_height)
            .px_3()
            .flex()
            .items_center()
            .text_color(if enabled {
                t.text_primary
            } else {
                t.text_muted
            })
            .when(enabled, |this| {
                this.hover(move |style| style.bg(t.hover))
                    .on_click(move |_, window, cx| {
                        window.focus(&focus);
                        window.dispatch_action(action.boxed_clone(), cx);
                    })
            })
            .child(label)
    }
}

fn native_format_php(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + text.len() / 4);
    let mut indent = 0usize;
    let mut block_comment = false;
    let mut quote: Option<char> = None;
    for (line_index, raw_line) in text.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            if line_index > 0 {
                result.push('\n');
            }
            continue;
        }
        let closes =
            trimmed.starts_with('}') || trimmed.starts_with(']') || trimmed.starts_with(')');
        if closes {
            indent = indent.saturating_sub(1);
        }
        if line_index > 0 {
            result.push('\n');
        }
        result.push_str(&"    ".repeat(indent));
        result.push_str(trimmed);
        let mut chars = trimmed.chars().peekable();
        let mut opens = 0usize;
        let mut closes_on_line = 0usize;
        while let Some(ch) = chars.next() {
            if block_comment {
                if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    block_comment = false;
                }
                continue;
            }
            if let Some(active_quote) = quote {
                if ch == '\\' {
                    chars.next();
                } else if ch == active_quote {
                    quote = None;
                }
                continue;
            }
            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block_comment = true;
            } else if (ch == '/' && chars.peek() == Some(&'/')) || ch == '#' {
                break;
            } else if matches!(ch, '\'' | '"' | '`') {
                quote = Some(ch);
            } else if matches!(ch, '{' | '[' | '(') {
                opens += 1;
            } else if matches!(ch, '}' | ']' | ')') {
                closes_on_line += 1;
            }
        }
        indent = indent.saturating_add(opens);
        indent = indent.saturating_sub(closes_on_line.saturating_sub(usize::from(closes)));
    }
    if text.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn runtime_signature_detail(symbol: &axiom_php::Symbol) -> String {
    let signature = symbol
        .signature
        .as_ref()
        .map(|signature| {
            let parameters = signature
                .parameters
                .iter()
                .map(|parameter| {
                    let ty = parameter.declared_type.as_deref().unwrap_or("");
                    let suffix = if parameter.optional { " = …" } else { "" };
                    format!(
                        "{} ${}{}",
                        ty,
                        parameter.name.trim_start_matches('$'),
                        suffix
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let return_type = signature
                .declared_return_type
                .as_deref()
                .or(signature.phpdoc_return_type.as_deref())
                .map(|value| format!(": {value}"))
                .unwrap_or_default();
            format!("({parameters}){return_type}")
        })
        .unwrap_or_default();
    format!("{}{} • {:?}", symbol.name, signature, symbol.origin)
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let status = self.status.clone().or_else(|| {
            self.diagnostics
                .first()
                .map(|diagnostic| diagnostic.message.clone().into())
        });
        let context_position = self.context_menu.map(|position| {
            let viewport = self.scroll.0.borrow().base_handle.bounds().size;
            gpui::point(
                position.x.min((viewport.width - px(220.)).max(px(0.))),
                position.y.min((viewport.height - px(280.)).max(px(0.))),
            )
        });
        let has_selection = self.document.selection_offsets().is_some();
        let php_navigation = is_php_file(&self.file_path) && self.lsp.is_some();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.editor_background)
            .key_context("Editor")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::debug_keydown))
            .cursor(if self.ctrl_hover_range.is_some() {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            })
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::complete))
            .on_action(cx.listener(Self::hover_info))
            .on_action(cx.listener(Self::definition))
            .on_action(cx.listener(Self::references))
            .on_action(cx.listener(Self::reformat))
            .on_action(cx.listener(Self::signature_help))
            .on_action(cx.listener(Self::escape))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event, window, cx| {
                            this.mouse_down(event, window, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event, window, cx| {
                            this.right_mouse_down(event, window, cx);
                        }),
                    )
                    .child(div().absolute().size_full().child(EditorInputElement {
                        editor: cx.entity(),
                    }))
                    .child(
                        uniform_list(
                            "editor-lines",
                            self.document.line_count(),
                            cx.processor(|this, range: Range<usize>, window, _| {
                                this.line_layouts
                                    .borrow_mut()
                                    .retain(|line, _| range.contains(line));
                                range.map(|line| this.render_line(line, window)).collect()
                            }),
                        )
                        .track_scroll(self.scroll.clone())
                        .h_full(),
                    )
                    .when(!self.completions.is_empty(), |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(GUTTER_WIDTH + 80.))
                                .bottom(px(12.))
                                .w(px(320.))
                                .max_h(px(220.))
                                .overflow_hidden()
                                .rounded(m.border_radius_medium)
                                .bg(t.popup_background)
                                .border_1()
                                .border_color(t.border)
                                .shadow_lg()
                                .children(self.completions.iter().take(8).enumerate().map(
                                    |(index, item)| {
                                        div()
                                            .h(m.toolbar_height)
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .bg(if index == self.completion_selected {
                                                t.selection
                                            } else {
                                                t.popup_background
                                            })
                                            .text_color(t.text_primary)
                                            .child(
                                                div().w(m.icon_size).text_color(t.info).child("ƒ"),
                                            )
                                            .child(item.label.clone())
                                            .when_some(item.detail.clone(), |row, detail| {
                                                row.child(
                                                    div()
                                                        .ml_auto()
                                                        .text_color(t.text_muted)
                                                        .child(detail),
                                                )
                                            })
                                    },
                                )),
                        )
                    })
                    .when_some(self.hover_popup.clone(), |this, hover| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(GUTTER_WIDTH + 80.))
                                .bottom(px(12.))
                                .max_w(px(520.))
                                .p_3()
                                .rounded(m.border_radius_medium)
                                .bg(t.popup_background)
                                .border_1()
                                .border_color(t.border)
                                .shadow_lg()
                                .text_color(t.text_primary)
                                .child(hover),
                        )
                    })
                    .when_some(context_position, |this, position| {
                        let focus = self.focus.clone();
                        let context_editor = cx.entity();
                        this.child(
                            div()
                                .id("editor-context-menu")
                                .absolute()
                                .left(position.x)
                                .top(position.y)
                                .w(px(220.))
                                .py_1()
                                .rounded(m.border_radius_medium)
                                .bg(t.menu_background)
                                .border_1()
                                .border_color(t.border)
                                .shadow_lg()
                                .occlude()
                                .cursor(CursorStyle::Arrow)
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                                    cx.stop_propagation();
                                    context_editor.update(cx, |this, cx| {
                                        this.context_menu = None;
                                        cx.notify();
                                    });
                                })
                                .child(Self::context_action("Undo", Undo, focus.clone(), true))
                                .child(Self::context_action("Redo", Redo, focus.clone(), true))
                                .child(separator())
                                .child(Self::context_action(
                                    "Cut",
                                    Cut,
                                    focus.clone(),
                                    has_selection,
                                ))
                                .child(Self::context_action(
                                    "Copy",
                                    Copy,
                                    focus.clone(),
                                    has_selection,
                                ))
                                .child(Self::context_action("Paste", Paste, focus.clone(), true))
                                .child(separator())
                                .child(Self::context_action(
                                    "Go to Definition",
                                    Definition,
                                    focus.clone(),
                                    php_navigation,
                                ))
                                .child(Self::context_action(
                                    "Find References",
                                    References,
                                    focus.clone(),
                                    php_navigation,
                                ))
                                .child(Self::context_action(
                                    "Completion",
                                    Complete,
                                    focus.clone(),
                                    php_navigation,
                                ))
                                .child(separator())
                                .child(Self::context_action("Select All", SelectAll, focus, true)),
                        )
                    }),
            )
            .when_some(status, |this, status| {
                this.child(
                    div()
                        .h(m.status_bar_height)
                        .px_3()
                        .bg(t.panel_background)
                        .text_color(t.info)
                        .child(status),
                )
            })
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EditorView {
    fn insert_text_with_pairs(&mut self, text: &str) {
        if text.len() == 1 {
            let closing = match text.as_bytes()[0] as char {
                '{' => Some('}'),
                '[' => Some(']'),
                '(' => Some(')'),
                '"' => Some('"'),
                '\'' => Some('\''),
                _ => None,
            };
            if let Some(close) = closing {
                self.document.insert_text(&format!("{text}{close}"));
                self.document
                    .move_cursor(self.document.cursor_offset().saturating_sub(1));
                return;
            }
            if matches!(text, "}" | "]" | ")" | "\"" | "'") {
                let content = self.document.content();
                let offset = self.document.cursor_offset();
                if content[offset..].starts_with(text) {
                    self.document
                        .move_cursor(self.document.next_codepoint_offset(offset));
                    return;
                }
            }
        }
        self.document.insert_text(text);
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range);
        actual.replace(self.range_to_utf16(&range));
        Some(self.document.text_in_range(range.start, range.end))
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range()),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range());
        self.document.set_selection(range.start, range.end);
        self.insert_text_with_pairs(text);
        self.after_edit(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range());
        self.document.set_selection(range.start, range.end);
        self.insert_text_with_pairs(text);
        let start = range.start;
        self.sync_syntax();
        self.marked_range = (!text.is_empty()).then_some(start..start + text.len());
        if let Some(selected) = selected {
            let selected = self.range_from_utf16(&selected);
            self.document
                .set_selection(start + selected.start, start + selected.end);
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: gpui::Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::Bounds<Pixels>> {
        Some(bounds)
    }

    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.document.cursor_offset()))
    }
}

impl EditorView {
    fn offset_from_utf16(&self, offset: usize) -> usize {
        let content = self.document.content();
        content
            .chars()
            .scan((0, 0), |state, ch| {
                let current = *state;
                state.0 += ch.len_utf16();
                state.1 += ch.len_utf8();
                Some(current)
            })
            .find_map(|(utf16, utf8)| (utf16 >= offset).then_some(utf8))
            .unwrap_or(content.len())
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.document
            .text_in_range(0, offset)
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }
}

struct EditorInputElement {
    editor: Entity<EditorView>,
}

impl IntoElement for EditorInputElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorInputElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
        _: gpui::Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
    }
}

fn trim_eol(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}

fn strip_snippet_placeholders(snippet: &str) -> String {
    let mut output = String::new();
    let mut chars = snippet.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            output.push(ch);
            continue;
        }
        if chars.peek() == Some(&'{') {
            chars.next();
            let mut body = String::new();
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
                body.push(next);
            }
            if let Some((_, default)) = body.split_once(':') {
                output.push_str(default);
            }
        } else {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
        }
    }
    output
}

fn shape(window: &mut Window, text: &str) -> gpui::ShapedLine {
    let style = window.text_style();
    let text: SharedString = text.to_owned().into();
    let run = TextRun {
        len: text.len(),
        font: font("Cascadia Mono"),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(text, px(14.), &[run], None)
}

#[cfg(test)]
mod formatter_tests {
    use super::native_format_php;

    #[test]
    fn indents_nested_php_blocks() {
        let input = "<?php\nclass Test{\npublic function foo(){\n$value=1;\nif($value){\necho \"ok\";\n}\n}\n}\n";
        let output = native_format_php(input);
        assert!(output.contains("class Test{\n    public function foo(){\n        $value=1;"));
        assert!(output.contains("        if($value){\n            echo \"ok\";"));
    }

    #[test]
    fn ignores_braces_in_strings_and_comments() {
        let input = "<?php\nfunction test(){\n$text = \"{ not a block }\";\n// } remains a comment\nreturn $text;\n}\n";
        let output = native_format_php(input);
        assert!(output.contains("    $text = \"{ not a block }\";"));
        assert!(output.contains("    // } remains a comment"));
        assert!(output.contains("    return $text;"));
    }
}
