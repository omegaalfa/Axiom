use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axiom_editor::Document;
use axiom_index::{
    FindUsagesStatus, ProjectSymbolIndex, ProjectSymbolKind, SemanticEngine, VendorSymbolIndex,
};
use axiom_lsp::{PositionCodec, PositionEncoding, path_to_uri};
use axiom_php::{RuntimeSymbolIndex, Symbol as RuntimeSymbol, SymbolKind as RuntimeKind};
use axiom_project::is_php_file;
use axiom_syntax::PhpSyntax;
use gpui::background_executor;
use gpui::{
    Action, App, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, KeyDownEvent,
    LayoutId, ListHorizontalSizingBehavior, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollStrategy, SharedString, Style, TextRun, UTF16Selection,
    UniformListScrollHandle, Window, actions, div, prelude::*, px, relative, uniform_list,
};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Diagnostic, DiagnosticSeverity,
    InsertTextFormat, Uri,
};

use crate::{
    lsp_bridge::LspBridge,
    syntax_theme::styled_segment,
    ui::{
        components::separator,
        metrics,
        metrics::{CODE_FONT_FAMILY, code_font},
        theme,
    },
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
        NativeDefinition,
        Reformat,
        SignatureHelp,
        CompleteStatement,
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
        KeyBinding::new("ctrl-shift-enter", CompleteStatement, Some("Editor")),
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
    hover_anchor: Option<Point<Pixels>>,
    diagnostics: Vec<ByteDiagnostic>,
    context_menu: Option<Point<Pixels>>,
    ctrl_hover_range: Option<Range<usize>>,
    line_layouts: RefCell<HashMap<usize, CachedLineLayout>>,
    runtime_symbols: Option<Arc<RuntimeSymbolIndex>>,
    // LIMITATION: poisoned project/vendor locks are treated as unavailable.
    // Recovering with PoisonError::into_inner could expose an index whose
    // invariants were broken by the panic that poisoned a write guard.
    project_symbols: Option<Arc<std::sync::RwLock<ProjectSymbolIndex>>>,
    vendor_symbols: Option<Arc<std::sync::RwLock<VendorSymbolIndex>>>,
    semantic_engine: Option<Arc<SemanticEngine>>,
    project_index_revision: Option<Arc<AtomicU64>>,
    index_update_sender: Option<Sender<IndexUpdateRequest>>,
    last_completion_layout: Option<(u32, u32, u32, u32, bool)>,
    editor_scroll_hovered: bool,
    editor_scroll_drag_axis: Option<EditorScrollAxis>,
    editor_scroll_drag_start: Point<Pixels>,
    editor_scroll_drag_start_offset: Point<Pixels>,
    content_width: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorScrollAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone)]
struct ByteDiagnostic {
    range: Range<usize>,
    severity: Option<DiagnosticSeverity>,
    message: String,
}

struct IndexUpdateRequest {
    generation: u64,
    path: PathBuf,
    text: String,
    index: Arc<std::sync::RwLock<ProjectSymbolIndex>>,
    revision: Arc<AtomicU64>,
}

#[derive(Clone)]
pub struct VendorDefinitionRequest {
    pub index: Arc<std::sync::RwLock<VendorSymbolIndex>>,
    pub fqn: String,
    pub member: Option<String>,
    pub is_static: bool,
}

#[derive(Clone, Debug)]
pub enum DefinitionQuery {
    Name {
        fqn: String,
        written: String,
    },
    Method {
        owner_fqn: String,
        name: String,
        is_static: bool,
    },
    Function {
        name: String,
    },
}

fn run_index_update_worker(receiver: mpsc::Receiver<IndexUpdateRequest>) {
    while let Ok(mut request) = receiver.recv() {
        if let Ok(next) = receiver.recv_timeout(Duration::from_millis(150)) {
            request = next;
            while let Ok(next) = receiver.try_recv() {
                request = next;
            }
        }
        if request.revision.load(Ordering::SeqCst) != request.generation {
            continue;
        }
        if let Ok(mut index) = request.index.write()
            && request.revision.load(Ordering::SeqCst) == request.generation
        {
            let _ =
                index.index_file_text_with_source(request.path, request.text, "EditorDirtyUpdate");
        }
    }
}

impl EditorView {
    fn resolve_location(
        &self,
        file: &Path,
        offset: usize,
        current_text: &str,
    ) -> Option<(PathBuf, lsp_types::Position)> {
        let content = if file == self.file_path {
            current_text.to_owned()
        } else {
            fs::read_to_string(file).ok()?
        };
        Some((
            file.to_path_buf(),
            PositionCodec::offset_to_position(&content, offset, self.lsp_encoding()),
        ))
    }

    pub fn from_document(
        path: PathBuf,
        document: Document,
        lsp: Option<Arc<LspBridge>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let open_started = std::time::Instant::now();
        axiom_index::trace_path("document_open", "Document", &path);
        let (index_update_sender, index_update_receiver) = mpsc::channel::<IndexUpdateRequest>();
        background_executor()
            .spawn(async move { run_index_update_worker(index_update_receiver) })
            .detach();
        let syntax_started = std::time::Instant::now();
        let syntax = is_php_file(&path)
            .then(|| PhpSyntax::parse(document.content()))
            .transpose()
            .expect("the PHP grammar and highlight query were validated at startup");
        let syntax_us = syntax_started.elapsed().as_micros();
        let last_lsp_text = document.content();
        let lsp_uri = is_php_file(&path)
            .then(|| path_to_uri(&path).ok())
            .flatten();
        let lsp_started = std::time::Instant::now();
        if let (Some(lsp), Some(uri)) = (&lsp, &lsp_uri) {
            lsp.with_server(|server| {
                if let Err(error) = server.did_open(uri.clone(), 1, last_lsp_text.clone()) {
                    tracing::warn!("didOpen failed: {error}");
                }
            });
        }
        let lsp_setup_us = lsp_started.elapsed().as_micros();
        let mut view = Self {
            document,
            syntax,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            selection_anchor: None,
            preferred_x: None,
            marked_range: None,
            selecting: false,
            file_path: path.clone(),
            status: None,
            lsp,
            lsp_uri,
            lsp_version: 1,
            last_lsp_text,
            completions: Vec::new(),
            completion_selected: 0,
            hover_popup: None,
            hover_anchor: None,
            diagnostics: Vec::new(),
            context_menu: None,
            ctrl_hover_range: None,
            line_layouts: RefCell::new(HashMap::new()),
            runtime_symbols: None,
            project_symbols: None,
            vendor_symbols: None,
            semantic_engine: None,
            project_index_revision: None,
            index_update_sender: Some(index_update_sender),
            last_completion_layout: None,
            editor_scroll_hovered: false,
            editor_scroll_drag_axis: None,
            editor_scroll_drag_start: Point::default(),
            editor_scroll_drag_start_offset: Point::default(),
            content_width: px(0.),
        };
        let inspections_started = std::time::Instant::now();
        view.sync_syntax();
        let native_inspections_us = inspections_started.elapsed().as_micros();
        if debug_completion_enabled() {
            let source = if path
                .components()
                .any(|component| component.as_os_str() == "vendor")
            {
                "Vendor"
            } else {
                "Project"
            };
            tracing::info!(
                source,
                path = %path.display(),
                disk_read_us = 0_u128,
                editor_create_us = open_started.elapsed().as_micros(),
                syntax_us,
                native_inspections_us,
                completion_setup_us = 0_u128,
                lsp_setup_us,
                first_frame_us = 0_u128,
                total_us = open_started.elapsed().as_micros(),
                "[EDITOR OPEN PERF]"
            );
        }
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

    pub fn document_content(&self) -> String {
        self.document.content()
    }

    pub fn current_cursor_offset(&self) -> usize {
        self.document.cursor_offset()
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
            let (_, owner_expression) = extract_owner_expression(&text_at_cursor, owner_end);
            if let Some(class_fqn) =
                self.resolve_receiver_type(&owner_expression, &text_at_cursor[..owner_end])
            {
                if let Some(index) = &self.project_symbols
                    && let Ok(index) = index.try_read()
                {
                    if let Some(symbol) =
                        index.find_methods(&class_fqn).into_iter().find(|symbol| {
                            symbol.name == name
                                && (is_static
                                    == symbol.modifiers.iter().any(|modifier| modifier == "static"))
                        })
                    {
                        return self.resolve_location(
                            &symbol.file,
                            symbol.range.start,
                            &text_at_cursor,
                        );
                    }
                    // Keep navigation useful when a variable's type could not
                    // be inferred from a local assignment. A unique method
                    // name is safe to resolve without a full type engine.
                    let matches: Vec<_> = index
                        .symbols()
                        .iter()
                        .filter(|symbol| {
                            symbol.kind == ProjectSymbolKind::Method && symbol.name == name
                        })
                        .collect();
                    if matches.len() == 1 {
                        let symbol = matches[0];
                        return self.resolve_location(
                            &symbol.file,
                            symbol.range.start,
                            &text_at_cursor,
                        );
                    }
                }
                // Vendor members are resolved by the asynchronous definition
                // pipeline. This legacy synchronous path must never parse or
                // lock the Vendor index on the UI thread.
                if let Some(runtime) = &self.runtime_symbols {
                    let runtime_class_fqn = runtime
                        .find_class(&class_fqn)
                        .map(|symbol| symbol.fqn.clone())
                        .or_else(|| {
                            runtime
                                .find_class_by_short_name(&class_fqn)
                                .map(|symbol| symbol.fqn.clone())
                        })
                        .unwrap_or(class_fqn.clone());
                    if let Some(symbol) = runtime
                        .methods_of(&runtime_class_fqn)
                        .find(|symbol| symbol.name == name && symbol.is_static == is_static)
                    {
                        return self.resolve_location(
                            &symbol.location.file,
                            symbol.location.range.start,
                            &text_at_cursor,
                        );
                    }
                }
            }
        }
        if let Some(runtime) = &self.runtime_symbols
            && let Some(symbol) = runtime.find_function(name)
        {
            return self.resolve_location(
                &symbol.location.file,
                symbol.location.range.start,
                &text_at_cursor,
            );
        }
        if let Some(index) = &self.project_symbols
            && let Ok(index) = index.try_read()
            && let Some(symbol) = index
                .symbols()
                .iter()
                .find(|symbol| symbol.kind == ProjectSymbolKind::Function && symbol.name == name)
        {
            return self.resolve_location(&symbol.file, symbol.range.start, &text_at_cursor);
        }
        let target = if let Some(index) = &self.project_symbols {
            let index = index.try_read().ok()?;
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
                    .or_else(|| index.find_class_by_short_name(name))
                    .map(|symbol| (symbol.location.file.clone(), symbol.location.range.clone()))
            })
        })?;
        self.resolve_location(&target.0, target.1.start, &text_at_cursor)
    }

    /// Extracts a Vendor definition request without reading or parsing the
    /// dependency file. The caller must resolve/parse it off the UI thread.
    pub fn definition_query(&self) -> Option<DefinitionQuery> {
        let syntax = self.syntax.as_ref()?;
        let cursor_offset = self.document.cursor_offset();
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            tracing::info!(cursor_offset, "[DEFINITION QUERY INPUT]");
        }
        let token = syntax.token_at_byte(cursor_offset)?;
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            let node = syntax
                .tree()
                .root_node()
                .descendant_for_byte_range(token.range.start, token.range.end);
            tracing::info!(
                token_text = %token.text,
                token_kind = %token.kind,
                token_range = ?token.range,
                node_kind = ?node.as_ref().map(|node| node.kind()),
                node_range = ?node.as_ref().map(|node| node.byte_range()),
                node_text = ?node.as_ref().and_then(|node| {
                    node.utf8_text(self.document.content().as_bytes())
                        .ok()
                        .map(str::to_owned)
                }),
                keyword = syntax.is_keyword_at_byte(cursor_offset),
                "[DEFINITION QUERY TOKEN]"
            );
        }
        // A keyword is never a definition query.  This guard keeps legacy
        // project/vendor fallback from turning `return` (or another control
        // keyword) into a namespaced class such as `Namespace\\return`.
        if syntax.is_keyword_at_byte(cursor_offset) {
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                tracing::info!(branch = "KeywordNone", "[DEFINITION QUERY BRANCH]");
            }
            return None;
        }
        let text = self.document.content();
        let before = &text[..token.range.start];
        // Only inspect the current expression. Looking through the entire
        // document prefix lets an earlier `$this->...` make an unrelated
        // `new Future` token look like a method member.
        let expression_start = before
            .rfind(|ch: char| matches!(ch, ';' | '{' | '}' | '\n'))
            .map(|index| index + 1)
            .unwrap_or(0);
        let expression = &before[expression_start..];
        if let Some(relative_operator) = expression.rfind("->").or_else(|| expression.rfind("::")) {
            let operator = expression_start + relative_operator;
            let is_static = before[operator..].starts_with("::");
            let owner_end = operator;
            let (_, written_owner) = extract_owner_expression(before, owner_end);
            let owner_fqn = if written_owner.starts_with('$') {
                self.resolve_receiver_type(&written_owner, before)?
            } else {
                resolve_php_class_name(&written_owner, &text)
            };
            if debug_completion_enabled() {
                tracing::info!(token = %token.text, kind = "Method", written = %written_owner, resolved = %owner_fqn, via = "receiver-type", "[DEFINITION QUERY]");
            }
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                tracing::info!(branch = "Method", "[DEFINITION QUERY BRANCH]");
            }
            return Some(DefinitionQuery::Method {
                owner_fqn,
                name: token.text.trim_start_matches('$').to_owned(),
                is_static,
            });
        }
        let name = token.text.trim_start_matches('$').to_owned();
        let is_new = text[..token.range.start].trim_end().ends_with("new");
        if !is_new && text[token.range.end..].starts_with('(') {
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                tracing::info!(branch = "Function", "[DEFINITION QUERY BRANCH]");
            }
            return Some(DefinitionQuery::Function { name });
        }
        let fqn = resolve_php_class_name(&name, &text);
        if debug_completion_enabled() {
            let via = if text
                .lines()
                .any(|line| line.trim_start().starts_with("use ") && line.contains(&name))
            {
                "import"
            } else if name.contains('\\') {
                "fqn"
            } else {
                "namespace-or-global"
            };
            tracing::info!(token = %name, kind = "Name", written = %name, resolved = %fqn, via, "[DEFINITION QUERY]");
        }
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            tracing::info!(branch = "Name", "[DEFINITION QUERY BRANCH]");
        }
        Some(DefinitionQuery::Name { fqn, written: name })
    }

    pub fn vendor_definition_request(&self) -> Option<VendorDefinitionRequest> {
        let query = self.definition_query()?;
        let (fqn, member, is_static) = match query {
            DefinitionQuery::Method {
                owner_fqn,
                name,
                is_static,
                ..
            } => (owner_fqn, Some(name), is_static),
            DefinitionQuery::Name { fqn, .. } => (fqn, None, false),
            DefinitionQuery::Function { .. } => return None,
        };
        let index = self.vendor_symbols.clone()?;
        // Vendor metadata and dependency files are resolved by the background
        // worker. Never perform UNC filesystem probes on the UI thread.
        Some(VendorDefinitionRequest {
            index,
            fqn,
            member,
            is_static,
        })
    }

    /// Project-only lookup. Composer is deliberately not consulted here so
    /// workspace symbols always take precedence over Vendor metadata.
    pub fn project_definition_location(&self) -> Option<(PathBuf, lsp_types::Position)> {
        let text = self.document.content();
        let query = self.definition_query()?;
        let index = self.project_symbols.as_ref()?.try_read().ok()?;
        let symbol = match query {
            DefinitionQuery::Method {
                owner_fqn,
                name,
                is_static,
                ..
            } => index.find_methods(&owner_fqn).into_iter().find(|symbol| {
                symbol.name == name && symbol.modifiers.iter().any(|m| m == "static") == is_static
            }),
            DefinitionQuery::Name { fqn, written } => index
                .find_class(&fqn)
                .or_else(|| index.find_class(&written)),
            DefinitionQuery::Function { name } => index
                .symbols()
                .iter()
                .find(|symbol| symbol.kind == ProjectSymbolKind::Function && symbol.name == name),
        }?;
        self.resolve_location(&symbol.file, symbol.range.start, &text)
    }

    fn lsp_encoding(&self) -> PositionEncoding {
        self.lsp
            .as_ref()
            .map(|lsp| lsp.encoding())
            .unwrap_or_default()
    }

    pub fn set_runtime_symbols(&mut self, symbols: Arc<RuntimeSymbolIndex>) {
        self.runtime_symbols = Some(symbols);
        self.sync_syntax();
    }

    pub fn set_project_symbols(&mut self, symbols: Arc<std::sync::RwLock<ProjectSymbolIndex>>) {
        self.project_symbols = Some(symbols);
        self.project_index_revision = Some(Arc::new(AtomicU64::new(0)));
        self.sync_syntax();
    }

    pub fn set_vendor_symbols(&mut self, symbols: Arc<std::sync::RwLock<VendorSymbolIndex>>) {
        self.vendor_symbols = Some(symbols);
        self.sync_syntax();
    }

    pub fn set_semantic_engine(&mut self, engine: Arc<SemanticEngine>) {
        self.semantic_engine = Some(engine);
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

    fn completion_replacement_range(&self) -> Range<usize> {
        if self.document.selection_offsets().is_some() {
            return self.selected_range();
        }
        let content = self.document.content();
        let cursor = self.document.cursor_offset().min(content.len());
        let start = content[..cursor]
            .char_indices()
            .rev()
            .take_while(|(_, character)| {
                character.is_alphanumeric() || matches!(character, '_' | '$' | '\\')
            })
            .last()
            .map_or(cursor, |(index, _)| index);
        start..cursor
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
        if debug_input_enabled() {
            tracing::info!(
                offset,
                before_char = ?previous,
                after_char = ?next,
                "[EDITOR ENTER]"
            );
            tracing::info!(
                detected = pair,
                kind = if matches!((previous, next), (Some('{'), Some('}'))) {
                    "brace"
                } else if pair {
                    "bracket"
                } else {
                    "none"
                },
                "[PAIR ENTER]"
            );
        }
        if pair {
            let line = self.document.line_of_offset(offset);
            let line_content = self.document.line_content(line);
            let base_indent = line_content
                .chars()
                .take_while(|character| character.is_whitespace())
                .collect::<String>();
            let inner_indent = format!("{base_indent}    ");
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
        if !self.completions.is_empty() {
            self.accept_completion(cx);
        } else if self.document.selection_offsets().is_some() {
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
        if debug_input_enabled() {
            tracing::info!(
                indent,
                selection_start,
                selection_end,
                start_line,
                end_line,
                "[EDITOR INDENT]"
            );
        }
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
        if !cfg!(debug_assertions) {
            return;
        }
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
        let text = self.document.content();
        self.sync_syntax_text(&text);
        self.sync_lsp_text(&text);
        self.schedule_incremental_index_update(&text);
        self.maybe_trigger_completion();
        let cursor = self.document.cursor_offset();
        let content = self.document.content();
        if cursor > 0 && matches!(content[..cursor].chars().next_back(), Some('(' | ',')) {
            self.hover_popup = self.native_signature_help();
            self.hover_anchor = None;
        }
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

    fn should_expand_member_dash(&self) -> bool {
        let text = self.document.content();
        let cursor = self.document.cursor_offset();
        let before = &text[..cursor];
        if before.is_empty() || before.chars().next_back().is_some_and(char::is_whitespace) {
            return false;
        }
        // EntityInputHandler is called before the typed `-` is inserted. The
        // receiver therefore ends at the current caret, not at a trailing
        // dash. Keep the check here so `$obj -` remains subtraction while
        // `$obj-` can be rewritten as `$obj->`.
        let owner_start = before
            .char_indices()
            .rev()
            .take_while(|(_, ch)| ch.is_alphanumeric() || matches!(ch, '_' | '$' | '\\'))
            .last()
            .map_or(before.len(), |(index, _)| index);
        let owner = &before[owner_start..];
        if owner == "$this" {
            return before[..owner_start].contains("class ");
        }
        self.resolve_native_type(owner, before).is_some()
    }

    fn schedule_incremental_index_update(&self, text: &str) {
        let (Some(index), Some(revision)) = (
            self.project_symbols.clone(),
            self.project_index_revision.clone(),
        ) else {
            return;
        };
        let generation = revision.fetch_add(1, Ordering::SeqCst) + 1;
        let path = self.file_path.clone();
        axiom_index::trace_path("incremental_request", "EditorDirtyUpdate", &path);
        let text = text.to_owned();
        if let Some(sender) = &self.index_update_sender {
            let _ = sender.send(IndexUpdateRequest {
                generation,
                path,
                text,
                index,
                revision,
            });
        }
    }

    /// Returns false for offsets contained in PHP comments or string-like
    /// literals. Native inspections still use byte ranges for precise edits,
    /// but this AST guard prevents text scans from interpreting prose as code.
    fn is_code_offset(&self, offset: usize) -> bool {
        let Some(syntax) = &self.syntax else {
            return true;
        };
        if offset >= syntax.text().len() {
            return false;
        }
        let Some(mut node) = syntax
            .tree()
            .root_node()
            .descendant_for_byte_range(offset, offset + 1)
        else {
            return true;
        };
        loop {
            if matches!(
                node.kind(),
                "comment" | "string" | "encapsed_string" | "heredoc" | "nowdoc"
            ) {
                return false;
            }
            let Some(parent) = node.parent() else { break };
            node = parent;
        }
        true
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
        self.sync_syntax_text(&text);
    }

    fn sync_syntax_text(&mut self, text: &str) {
        if let Some(syntax) = &mut self.syntax
            && let Err(error) = syntax.update_text(text)
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
        self.add_native_inspections(text);
        self.add_native_argument_inspections(text);
    }

    fn add_native_argument_inspections(&mut self, text: &str) {
        let runtime = self.runtime_symbols.clone();
        let project = self.project_symbols.clone();
        let mut search = 0;
        while let Some(relative_open) = text[search..].find('(') {
            let open = search + relative_open;
            if !self.is_code_offset(open) {
                search = open.saturating_add(1);
                continue;
            }
            let Some(close) = matching_paren(text, open) else {
                break;
            };
            let callable_start = text[..open]
                .char_indices()
                .rev()
                .take_while(|(_, ch)| {
                    ch.is_alphanumeric() || matches!(ch, '_' | '$' | '\\' | ':' | '-' | '>')
                })
                .last()
                .map_or(open, |(index, _)| index);
            let callable = text[callable_start..open].trim();
            let name = callable
                .rsplit_once("::")
                .or_else(|| callable.rsplit_once("->"))
                .map(|(_, name)| name)
                .unwrap_or(callable)
                .trim_start_matches('$');
            let has_receiver = callable.contains("->") || callable.contains("::");
            if name.is_empty() {
                search = close.saturating_add(1);
                continue;
            }
            let runtime_signature = runtime.as_ref().and_then(|runtime| {
                callable
                    .rsplit_once("::")
                    .or_else(|| callable.rsplit_once("->"))
                    .and_then(|(owner, _)| {
                        let owner = self.resolve_native_type(owner.trim(), &text[..open])?;
                        runtime
                            .methods_of(&owner)
                            .find(|symbol| symbol.name == name)
                    })
                    .or_else(|| {
                        (!has_receiver)
                            .then(|| runtime.find_function(name))
                            .flatten()
                    })
                    .and_then(|symbol| {
                        symbol.signature.as_ref().map(|signature| {
                            (
                                signature
                                    .parameters
                                    .iter()
                                    .filter(|parameter| !parameter.optional && !parameter.variadic)
                                    .count(),
                                signature.parameters.len(),
                                signature
                                    .parameters
                                    .iter()
                                    .any(|parameter| parameter.variadic),
                            )
                        })
                    })
            });
            let signature_info = runtime_signature.or_else(|| {
                let resolved_owner = callable
                    .rsplit_once("::")
                    .or_else(|| callable.rsplit_once("->"))
                    .and_then(|(owner, _)| self.resolve_receiver_type(owner.trim(), &text[..open]));
                let project = project.as_ref()?.try_read().ok()?;
                let symbol = resolved_owner
                    .as_deref()
                    .and_then(|owner| {
                        project
                            .find_methods(owner)
                            .into_iter()
                            .find(|symbol| symbol.name == name)
                    })
                    .or_else(|| {
                        (!has_receiver)
                            .then(|| {
                                project.symbols().iter().find(|symbol| {
                                    symbol.kind == ProjectSymbolKind::Function
                                        && symbol.name == name
                                })
                            })
                            .flatten()
                    });
                let detail = project_callable_detail(symbol?)?;
                let (required, maximum, variadic) = signature_counts_from_detail(&detail);
                Some((required, maximum, variadic))
            });
            if let Some((required, maximum, variadic)) = signature_info {
                let arguments = count_call_arguments(&text[open + 1..close]);
                let too_few = arguments < required;
                let too_many = !variadic && arguments > maximum;
                if too_few || too_many {
                    let expected = if too_many {
                        format!("Expected at most {maximum} arguments, found {arguments}")
                    } else {
                        format!(
                            "Expected {} argument{}, found {arguments}",
                            required,
                            if required == 1 { "" } else { "s" }
                        )
                    };
                    self.diagnostics.push(ByteDiagnostic {
                        range: callable_start..close + 1,
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: expected,
                    });
                }
            }
            search = close.saturating_add(1);
        }
    }

    fn add_native_inspections(&mut self, text: &str) {
        // Do not publish definitive Unknown class diagnostics while either
        // project or Composer metadata is still loading.
        if self.project_symbols.is_none() || self.vendor_symbols.is_none() {
            return;
        }
        if self
            .project_symbols
            .as_ref()
            .and_then(|index| index.try_read().ok())
            .is_some_and(|index| !index.is_ready())
        {
            return;
        }
        let mut offset = 0;
        while let Some(relative) = text[offset..].find("new ") {
            let start = offset + relative + 4;
            if !self.is_code_offset(start) {
                offset = start.max(offset + 1);
                continue;
            }
            let end = start
                + text[start..]
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '\\')
                    .map(char::len_utf8)
                    .sum::<usize>();
            if end > start {
                let written = &text[start..end];
                let name = written.trim_start_matches('\\');
                let resolved = self.resolve_class_name(written, text);
                let project_symbol = self
                    .project_symbols
                    .as_ref()
                    .and_then(|index| index.try_read().ok())
                    .and_then(|index| index.find_class(&resolved).cloned());
                let known_project = project_symbol.is_some();
                let known_vendor = self
                    .vendor_symbols
                    .as_ref()
                    .and_then(|index| index.try_read().ok())
                    .is_some_and(|index| index.has_class_metadata(&resolved));
                // Composer resolution performs filesystem probes. Completion
                // must not perform those probes synchronously on the UI thread.
                let composer_found = false;
                let known_runtime = self.runtime_symbols.as_ref().is_some_and(|runtime| {
                    runtime.find_class(&resolved).is_some()
                        || runtime.find_class_by_short_name(name).is_some()
                });
                let lsp_found = self
                    .lsp
                    .as_ref()
                    .is_some_and(|lsp| lsp.status() == axiom_lsp::ServerStatus::Ready);
                let special = matches!(name, "self" | "static" | "parent");
                // LSP readiness is reported for diagnostics, but it does not
                // prove that this class exists. Native/project/runtime indexes
                // remain the source of truth for this local inspection.
                let diagnostic = !known_project
                    && !known_vendor
                    && !composer_found
                    && !known_runtime
                    && !special;
                if debug_completion_enabled() {
                    let via = if resolved == name {
                        if text[..start].contains("namespace ") {
                            "namespace-or-global"
                        } else {
                            "fqn-or-global"
                        }
                    } else if text[..start]
                        .lines()
                        .any(|line| line.trim_start().starts_with("use ") && line.contains(name))
                    {
                        "import"
                    } else {
                        "namespace"
                    };
                    tracing::info!(
                        written,
                        resolved = %resolved,
                        via,
                        project_found = known_project,
                        vendor_found = known_vendor,
                        composer_found,
                        runtime_found = known_runtime,
                        lsp_found,
                        diagnostic,
                        "[CLASS RESOLUTION]"
                    );
                }
                if diagnostic {
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
        let mut symbol_files: std::collections::HashMap<
            (&str, ProjectSymbolKind),
            std::collections::HashSet<&Path>,
        > = std::collections::HashMap::new();
        let Some(project_guard) = self
            .project_symbols
            .as_ref()
            .and_then(|index| index.try_read().ok())
        else {
            return;
        };
        let index = &project_guard;
        for symbol in index.symbols() {
            symbol_files
                .entry((symbol.fully_qualified_name.as_str(), symbol.kind))
                .or_default()
                .insert(symbol.file.as_path());
        }
        for symbol in index
            .symbols()
            .iter()
            .filter(|symbol| symbol.file == self.file_path)
        {
            if symbol_files
                .get(&(symbol.fully_qualified_name.as_str(), symbol.kind))
                .is_some_and(|files| files.len() > 1)
            {
                if debug_completion_enabled() {
                    let paths = symbol_files
                        .get(&(symbol.fully_qualified_name.as_str(), symbol.kind))
                        .into_iter()
                        .flat_map(|files| files.iter())
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>();
                    let candidates = symbol_files
                        .get(&(symbol.fully_qualified_name.as_str(), symbol.kind))
                        .into_iter()
                        .flat_map(|files| files.iter())
                        .enumerate()
                        .map(|(index, path)| {
                            let canonical = std::fs::canonicalize(path);
                            let source = if *path == self.file_path {
                                "EditorDirtyUpdate/Document"
                            } else {
                                "InitialProjectScan/Other"
                            };
                            format!(
                                "candidate_{}_source={source} path={:?} exists={} canonical={:?} canonicalize_error={:?}",
                                index + 1,
                                path,
                                path.exists(),
                                canonical.as_ref().ok(),
                                canonical.as_ref().err().map(ToString::to_string),
                            )
                        })
                        .collect::<Vec<_>>();
                    tracing::warn!(
                        symbol = %symbol.fully_qualified_name,
                        kind = ?symbol.kind,
                        current = %symbol.file.display(),
                        candidates = ?paths,
                        candidate_details = ?candidates,
                        "[DUPLICATE CLASS PATHS]"
                    );
                }
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
        let _ = index;
        let runtime_symbols = self.runtime_symbols.clone();
        drop(project_guard);
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
            if !self.is_code_offset(start) {
                offset = start.max(offset + 1);
                continue;
            }
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
        self.sync_lsp_text(&text);
    }

    fn sync_lsp_text(&mut self, text: &str) {
        if text == self.last_lsp_text {
            return;
        }
        self.last_lsp_text = text.to_owned();
        self.lsp_version = self.lsp_version.saturating_add(1);
        if let (Some(lsp), Some(uri)) = (&self.lsp, &self.lsp_uri) {
            lsp.with_server(|server| {
                if let Err(error) =
                    server.did_change(uri.clone(), self.lsp_version, text.to_owned())
                {
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
        if debug_completion_enabled() {
            tracing::info!(
                native_count = native.len(),
                cursor = self.document.cursor_offset(),
                "[COMPLETION REQUEST]"
            );
        }
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
                    let operator_start = index.saturating_sub(1);
                    let suffix = &before[index + 1..];
                    suffix
                        .chars()
                        .all(|character| character.is_alphanumeric() || character == '_')
                        .then_some((operator_start, false))
                } else if ch == ':' && before[..index].ends_with(':') {
                    let operator_start = index.saturating_sub(1);
                    let suffix = &before[index + 1..];
                    suffix
                        .chars()
                        .all(|character| character.is_alphanumeric() || character == '_')
                        .then_some((operator_start, true))
                } else {
                    None
                }
            });
        let start = text[..cursor]
            .char_indices()
            .rev()
            .take_while(|(_, ch)| ch.is_alphanumeric() || matches!(ch, '_' | '$'))
            .last()
            .map_or(cursor, |(i, _)| i);
        let prefix = &text[start..cursor];
        let preceded_by_new = before[..start].trim_end().ends_with("new");
        if prefix.starts_with('$') {
            return self.local_variable_completions(&text[..cursor], prefix);
        }
        let empty_prefix_context = before.ends_with("new ")
            || before.ends_with("extends ")
            || before.ends_with("implements ")
            || before.ends_with("use ");
        if prefix.is_empty() && member_operator.is_none() && !empty_prefix_context {
            return Vec::new();
        }
        // A class name by itself is also a static-access completion context.
        // This lets `CustomRuntime` expand directly to
        // `CustomRuntime::hello($name, $age)` without requiring `::` first.
        if member_operator.is_none()
            && !prefix.is_empty()
            && !empty_prefix_context
            && !preceded_by_new
        {
            let class_exists = self.runtime_symbols.as_ref().is_some_and(|index| {
                index.find_class(prefix).is_some()
                    || index.find_class_by_short_name(prefix).is_some()
            }) || self
                .project_symbols
                .as_ref()
                .and_then(|index| index.try_read().ok())
                .is_some_and(|index| index.find_class(prefix).is_some());
            if class_exists {
                let class_fqn = self.resolve_native_type(prefix, before);
                let mut items = Vec::new();
                if let (Some(runtime), Some(class_fqn)) =
                    (&self.runtime_symbols, class_fqn.as_ref())
                {
                    let runtime_fqn = runtime
                        .find_class(class_fqn)
                        .or_else(|| runtime.find_class_by_short_name(prefix))
                        .map(|symbol| symbol.fqn.clone())
                        .unwrap_or_else(|| class_fqn.clone());
                    items.extend(
                        runtime
                            .methods_of(&runtime_fqn)
                            .filter(|symbol| symbol.is_static)
                            .map(|symbol| {
                                let call = runtime_call_insert_text(symbol)
                                    .unwrap_or_else(|| format!("{}()", symbol.name));
                                CompletionItem {
                                    label: symbol.name.clone(),
                                    detail: Some(runtime_signature_detail(symbol)),
                                    kind: Some(CompletionItemKind::METHOD),
                                    insert_text: Some(format!("{prefix}::{call}")),
                                    ..Default::default()
                                }
                            }),
                    );
                }
                if let (Some(project), Some(class_fqn)) =
                    (&self.project_symbols, class_fqn.as_ref())
                    && let Ok(project) = project.try_read()
                {
                    items.extend(
                        project
                            .find_methods(class_fqn)
                            .into_iter()
                            .filter(|symbol| {
                                symbol.modifiers.iter().any(|modifier| modifier == "static")
                            })
                            .map(|symbol| {
                                let detail = project_method_detail(symbol);
                                let signature = detail
                                    .find('(')
                                    .and_then(|open| {
                                        detail.rfind(')').map(|close| &detail[open..=close])
                                    })
                                    .unwrap_or("()")
                                    .to_owned();
                                CompletionItem {
                                    label: symbol.name.clone(),
                                    detail: Some(detail),
                                    kind: Some(CompletionItemKind::METHOD),
                                    insert_text: Some(format!(
                                        "{prefix}::{}{signature}",
                                        symbol.name
                                    )),
                                    ..Default::default()
                                }
                            }),
                    );
                }
                if items.is_empty() {
                    items.push(CompletionItem {
                        label: "class".to_owned(),
                        detail: Some("Class name • Runtime".to_owned()),
                        kind: Some(CompletionItemKind::KEYWORD),
                        insert_text: Some(format!("{prefix}::class")),
                        ..Default::default()
                    });
                }
                return items.into_iter().take(40).collect();
            }
        }
        if let Some((operator_start, is_static)) = member_operator {
            let owner_end = operator_start;
            let (_, owner_expression) = extract_owner_expression(&text, owner_end);
            let owner = owner_expression.trim_start_matches('$');
            if debug_completion_enabled() && owner_expression.contains("->") {
                tracing::info!(owner = %owner_expression, "[DEFINITION RECEIVER CHAIN]");
            }
            if let Some(class_fqn) =
                self.resolve_receiver_type(&owner_expression, &text[..owner_end])
            {
                if debug_completion_enabled() {
                    tracing::info!(trigger = "MemberAccess", receiver_type = %class_fqn, "[COMPLETION CONTEXT]");
                }
                let mut members = Vec::new();
                if let Some(index) = &self.project_symbols
                    && let Ok(index) = index.try_read()
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
                                detail: Some(project_method_detail(symbol)),
                                kind: Some(CompletionItemKind::METHOD),
                                ..Default::default()
                            }),
                    );
                }
                if let Some(index) = &self.vendor_symbols
                    && let Ok(index) = index.try_read()
                {
                    members.extend(
                        index
                            .cached_symbols(&class_fqn)
                            .into_iter()
                            .filter(|symbol| {
                                symbol.name.starts_with(prefix)
                                    && (is_static == symbol.modifiers.iter().any(|m| m == "static"))
                            })
                            .map(|symbol| CompletionItem {
                                label: symbol.name,
                                detail: symbol.parameters.clone(),
                                kind: Some(CompletionItemKind::METHOD),
                                ..Default::default()
                            }),
                    );
                }
                if let Some(index) = &self.runtime_symbols {
                    let runtime_class_fqn = index
                        .find_class(&class_fqn)
                        .map(|symbol| symbol.fqn.clone())
                        .or_else(|| {
                            index
                                .find_class_by_short_name(owner)
                                .map(|symbol| symbol.fqn.clone())
                        })
                        .unwrap_or_else(|| class_fqn.clone());
                    if runtime_class_fqn != class_fqn && debug_completion_enabled() {
                        tracing::info!(
                            written = %class_fqn,
                            resolved = %runtime_class_fqn,
                            "[RUNTIME CLASS RESOLVE]"
                        );
                    }
                    members.extend(
                        index
                            .methods_of(&runtime_class_fqn)
                            .filter(|symbol| {
                                symbol.name.starts_with(prefix)
                                    && symbol.is_static == is_static
                                    && (is_static || !symbol.name.starts_with('_'))
                            })
                            .map(|symbol| {
                                if debug_completion_enabled() {
                                    tracing::info!(class = %class_fqn, member = %symbol.name, source = %symbol.location.file.display(), "[RUNTIME COMPLETION]");
                                }
                                CompletionItem {
                                    label: symbol.name.clone(),
                                    detail: Some(runtime_signature_detail(symbol)),
                                    kind: Some(CompletionItemKind::METHOD),
                                    insert_text: runtime_call_insert_text(symbol),
                                    ..Default::default()
                                }
                            }),
                    );
                }
                if is_static && members.is_empty() && prefix.is_empty() {
                    members.push(CompletionItem {
                        label: "class".to_owned(),
                        detail: Some("Class name • Runtime".to_owned()),
                        kind: Some(CompletionItemKind::KEYWORD),
                        insert_text: Some("class".to_owned()),
                        ..Default::default()
                    });
                }
                let mut seen = std::collections::HashSet::new();
                members.retain(|item| seen.insert(item.label.to_ascii_lowercase()));
                if debug_completion_enabled() {
                    tracing::info!(runtime_count = members.len(), "[COMPLETION PROVIDER]");
                    tracing::info!(result_count = members.len(), "[COMPLETION RESULT]");
                }
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
                    .map(|symbol| {
                        if debug_completion_enabled() {
                            tracing::info!(symbol = %symbol.name, source = %symbol.location.file.display(), "[RUNTIME COMPLETION]");
                        }
                        let import = matches!(symbol.kind, RuntimeKind::Class | RuntimeKind::Interface | RuntimeKind::Trait | RuntimeKind::Enum)
                            .then(|| self.composer_import_edit(&symbol.fqn))
                            .flatten();
                        CompletionItem {
                        label: symbol.name.clone(),
                        detail: Some(
                            if matches!(symbol.kind, RuntimeKind::Function | RuntimeKind::Method) {
                                runtime_signature_detail(symbol)
                            } else {
                                format!("{:?} • PHP Runtime", symbol.kind)
                            },
                        ),
                        kind: Some(match symbol.kind {
                            RuntimeKind::Function => CompletionItemKind::FUNCTION,
                            RuntimeKind::Class
                            | RuntimeKind::Interface
                            | RuntimeKind::Trait
                            | RuntimeKind::Enum => CompletionItemKind::CLASS,
                            _ => CompletionItemKind::VALUE,
                        }),
                        insert_text: runtime_call_insert_text(symbol),
                        additional_text_edits: import.map(|edit| vec![edit]),
                        ..Default::default()
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(index) = &self.project_symbols
            && let Ok(index) = index.try_read()
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
        if let Some(index) = &self.vendor_symbols
            && let Ok(index) = index.try_read()
        {
            items.extend(index.classes_matching(prefix).into_iter().map(|fqn| {
                let label = fqn.rsplit('\\').next().unwrap_or(&fqn).to_owned();
                let import = self.composer_import_edit(&fqn);
                CompletionItem {
                    label,
                    detail: Some(format!("{fqn} • Vendor")),
                    kind: Some(CompletionItemKind::CLASS),
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

    fn local_variable_completions(&self, context: &str, prefix: &str) -> Vec<CompletionItem> {
        let mut names = std::collections::BTreeSet::new();
        for (offset, _) in context.match_indices('$') {
            let tail = &context[offset..];
            let name = tail
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect::<String>();
            if !name.is_empty() {
                names.insert(format!("${name}"));
            }
        }
        let mut items = names
            .into_iter()
            .filter(|name| name.starts_with(prefix))
            .map(|name| {
                let detail = self
                    .resolve_native_type(&name, context)
                    .map(|ty| ty.to_owned());
                if debug_completion_enabled() {
                    tracing::info!(variable = %name, prefix, "[VARIABLE COMPLETION]");
                }
                CompletionItem {
                    label: name,
                    detail,
                    kind: Some(CompletionItemKind::VARIABLE),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        items.sort_by_key(|item| (!item.label.starts_with(prefix), item.label.clone()));
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
        if has_import(&text, fqn) {
            return None;
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
        let owner = owner.trim();
        if owner == "$this" {
            let resolved = declared_class_fqn(context);
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                eprintln!(
                    "[DEFINITION RECEIVER TYPE] owner={owner:?} candidate={:?} final={:?}",
                    "this", resolved
                );
            }
            return resolved;
        }
        let candidate = owner.trim_start_matches('$').to_owned();
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            eprintln!("[DEFINITION RECEIVER TYPE] owner={owner:?} candidate={candidate:?}");
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
                    let resolved = self.qualify_type(&name);
                    if debug_completion_enabled() {
                        tracing::info!(variable = %variable, written = %name, resolved = %resolved, source = "runtime/project", "[LOCAL TYPE]");
                    }
                    return Some(resolved);
                }
            }
        }
        let variable_name = format!("${candidate}");
        let occurrences: Vec<_> = context.match_indices(&variable_name).collect();
        for (pos, _) in occurrences.into_iter().rev() {
            let suffix = &context[pos + variable_name.len()..];
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                eprintln!(
                    "[DEFINITION RECEIVER OCCURRENCE] owner={owner:?} pos={pos} suffix={suffix:?}"
                );
            }
            // A usage in `$future->await` is not a declaration. The old
            // rfind-based heuristic selected this occurrence and read the
            // preceding statement word (`return`) as its type.
            if suffix.trim_start().starts_with("->") || suffix.trim_start().starts_with("::") {
                continue;
            }
            let declaration = &context[..pos];
            let ty = declaration
                .rsplit(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '\\' | '?' | '|')))
                .find(|part| !part.is_empty())
                .unwrap_or_default()
                .trim_start_matches('?')
                .trim_matches('|');
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                eprintln!("[DEFINITION RECEIVER TYPE] declaration={declaration:?} ty={ty:?}");
            }
            if !ty.is_empty()
                && ty
                    .chars()
                    .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '\\')
            {
                return Some(self.qualify_type(ty));
            }
        }
        let lower = candidate.to_ascii_lowercase();
        if self.runtime_symbols.as_ref().is_some_and(|index| {
            index.find_class(&candidate).is_some()
                || index.find_class(&format!("\\{candidate}")).is_some()
        }) {
            return Some(candidate);
        }
        let resolved = self
            .project_symbols
            .as_ref()
            .and_then(|index| index.try_read().ok())
            .and_then(|index| {
                index
                    .find_class(&candidate)
                    .map(|symbol| symbol.fully_qualified_name.clone())
            })
            .or_else(|| (!lower.is_empty()).then_some(candidate));
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            eprintln!("[DEFINITION RECEIVER TYPE] owner={owner:?} final={resolved:?}");
        }
        resolved
    }

    /// Resolves a receiver expression, including the common two-level
    /// `$this->property->method()` form. Property declarations are read from
    /// the current document so completion/definition can use their declared
    /// type without guessing from a variable name.
    fn resolve_receiver_type(&self, owner: &str, context: &str) -> Option<String> {
        let owner = owner.trim();
        if let Some((base, property)) = owner.rsplit_once("->") {
            let base_type = if base.trim() == "$this" {
                declared_class_fqn(context)
            } else {
                self.resolve_receiver_type(base, context)
                    .or_else(|| self.resolve_native_type(base, context))
            }?;
            let property_type = property_type_in_context(context, property.trim())?;
            let resolved = self.resolve_class_name(&property_type, context);
            if debug_completion_enabled() {
                tracing::info!(owner, property, base_type = %base_type, resolved = %resolved, "[DEFINITION RECEIVER]");
            }
            return Some(resolved);
        }
        self.resolve_native_type(owner, context)
    }

    fn qualify_type(&self, name: &str) -> String {
        self.resolve_class_name(name, &self.document.content())
    }

    fn resolve_class_name(&self, written: &str, context: &str) -> String {
        let candidate = resolve_php_class_name(written, context);
        let written = written.trim().trim_start_matches('\\');
        if written.contains('\\') {
            return candidate;
        }
        let contextual_exists = self
            .runtime_symbols
            .as_ref()
            .is_some_and(|index| index.find_class(&candidate).is_some())
            || self
                .project_symbols
                .as_ref()
                .and_then(|index| index.try_read().ok())
                .is_some_and(|index| index.find_class(&candidate).is_some());
        // Workspace symbols are authoritative. This keeps project-file
        // activation off the Composer path, which otherwise performs repeated
        // filesystem probes for every imported project type.
        if contextual_exists {
            return candidate;
        }
        let global_exists = self
            .runtime_symbols
            .as_ref()
            .is_some_and(|index| index.find_class(written).is_some())
            || self
                .project_symbols
                .as_ref()
                .and_then(|index| index.try_read().ok())
                .is_some_and(|index| index.find_class(written).is_some());
        if global_exists {
            return written.to_owned();
        }
        self.runtime_symbols
            .as_ref()
            .and_then(|runtime| runtime.find_class_by_short_name(written))
            .map(|symbol| symbol.fqn.clone())
            .unwrap_or(candidate)
    }

    fn hover_info(&mut self, _: &HoverInfo, _: &mut Window, _: &mut Context<Self>) {
        self.hover_anchor = None;
        if let (Some(lsp), Some(uri), Some(position)) =
            (&self.lsp, &self.lsp_uri, self.lsp_position())
        {
            lsp.request_hover(uri.clone(), position);
        }
    }

    fn definition(&mut self, _: &Definition, window: &mut Window, cx: &mut Context<Self>) {
        if debug_input_enabled() {
            tracing::info!(provider = "native-first", "[DEFINITION REQUEST]");
        }
        window.dispatch_action(NativeDefinition.boxed_clone(), cx);
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

    fn signature_help(&mut self, _: &SignatureHelp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.native_signature_help() {
            self.hover_popup = Some(text);
            self.hover_anchor = None;
            cx.notify();
        }
        if let (Some(lsp), Some(uri), Some(position)) =
            (&self.lsp, &self.lsp_uri, self.lsp_position())
        {
            lsp.request_signature_help(uri.clone(), position);
        }
    }

    fn native_signature_help(&self) -> Option<String> {
        let text = self.document.content();
        let cursor = self.document.cursor_offset().min(text.len());
        let before = &text[..cursor];
        let open = before.rfind('(')?;
        let callable = before[..open]
            .trim_end()
            .rsplit(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == ':'))
            .next()?;
        let name = callable.rsplit("::").next().unwrap_or(callable);
        let receiver = callable
            .rsplit_once("::")
            .map(|(owner, _)| owner.to_owned())
            .or_else(|| {
                before[..open].rfind("->").map(|operator| {
                    let (_, owner) = extract_owner_expression(before, operator);
                    owner
                })
            });
        if let Some(index) = &self.project_symbols {
            let owner = receiver.clone();
            if let Some(owner) = owner
                .as_deref()
                .and_then(|owner| self.resolve_receiver_type(owner, &text[..open]))
                && let Ok(index) = index.try_read()
                && let Some(method) = index
                    .find_methods(&owner)
                    .into_iter()
                    .find(|method| method.name == name)
            {
                let detail = project_method_detail(method);
                return Some(
                    detail
                        .strip_suffix(" • Project")
                        .unwrap_or(&detail)
                        .to_owned(),
                );
            }
        }
        let symbol = self.runtime_symbols.as_ref().and_then(|index| {
            index
                .find_function(name)
                .or_else(|| index.find_function(&format!("\\{name}")))
                .or_else(|| {
                    receiver.and_then(|owner| {
                        let owner = self.resolve_receiver_type(owner.as_str(), &text[..open])?;
                        let owner = index
                            .find_class(&owner)
                            .map(|symbol| symbol.fqn.clone())
                            .or_else(|| {
                                index
                                    .find_class_by_short_name(owner.as_str())
                                    .map(|symbol| symbol.fqn.clone())
                            })
                            .unwrap_or(owner);
                        index.methods_of(&owner).find(|symbol| symbol.name == name)
                    })
                })
        })?;
        let signature = symbol.signature.as_ref()?;
        if debug_completion_enabled() {
            tracing::info!(symbol = %format!("{}::{}", symbol.fqn, symbol.name), parameters = signature.parameters.len(), source = %symbol.location.file.display(), "[RUNTIME SIGNATURE]");
        }
        let active = before[open + 1..].chars().filter(|ch| *ch == ',').count();
        let params = signature
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                let ty = parameter.declared_type.as_deref().unwrap_or("");
                let optional = if parameter.optional { " = …" } else { "" };
                let variadic = if parameter.variadic { "..." } else { "" };
                let marker = if index == active { "▶ " } else { "" };
                format!(
                    "{marker}{ty} {variadic}${}{optional}",
                    parameter.name.trim_start_matches('$')
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
        Some(format!("{}({}){}", symbol.name, params, return_type))
    }

    #[allow(dead_code)]
    fn complete_statement(
        &mut self,
        _: &CompleteStatement,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cursor = self.document.cursor_offset();
        let text = self.document.content();
        let line_start = text[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let line_end = text[cursor..].find('\n').map_or(text.len(), |i| cursor + i);
        let line = &text[line_start..line_end];
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.ends_with([';', '{', '}', ':', ',']) {
            return;
        }
        if line[cursor.saturating_sub(line_start)..].trim().is_empty() {
            self.document.move_cursor(line_end);
            self.document.insert_text(";");
            self.after_edit(cx);
            if debug_input_enabled() {
                tracing::info!(
                    kind = "expression_statement",
                    semicolon = true,
                    "[COMPLETE STATEMENT]"
                );
            }
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
        self.hover_anchor = None;
        cx.notify();
    }

    fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        self.completions.clear();
        self.hover_popup = None;
        self.hover_anchor = None;
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
                self.completion_replacement_range(),
                item.insert_text.unwrap_or_else(|| item.label.clone()),
            ),
        };
        if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
            text = strip_snippet_placeholders(&text);
        }
        let content_before = self.document.content();
        let call_like = matches!(
            item.kind,
            Some(CompletionItemKind::FUNCTION)
                | Some(CompletionItemKind::METHOD)
                | Some(CompletionItemKind::CONSTRUCTOR)
        ) || (item.kind == Some(CompletionItemKind::CLASS)
            && content_before[..range.start].trim_end().ends_with("new"));
        let has_open = content_before[range.end..].starts_with('(')
            || text.ends_with('(')
            || text.ends_with(')')
            || text.contains("()");
        if call_like && !has_open {
            text.push('(');
            text.push(')');
        }
        let inserted_text = text.clone();
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
        let inserted = if call_like {
            inserted_text.trim_end_matches(')')
        } else {
            inserted_text.as_str()
        };
        // Search from the edited range. Searching the whole document can
        // select an earlier identical call and leave the caret inside an old
        // completion context, causing the same item to reappear on the next
        // Enter/newline.
        let search_start = range.start.min(updated.len());
        if let Some(relative) = updated[search_start..].find(inserted) {
            let position = search_start + relative;
            let caret = if call_like && inserted_text.ends_with("()") {
                position + inserted_text.len() - 1
            } else {
                position + inserted_text.len()
            };
            self.document.move_cursor(caret.min(updated.len()));
        }
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
        self.hover_anchor = None;
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
        // Claim focus as part of the same mouse gesture. Ctrl-click below
        // must not be lost when a tab was just reactivated or reopened.
        window.focus(&self.focus);
        let line = self.mouse_line(event.position);
        self.ctrl_hover_range = None;
        let offset = self.mouse_offset(line, event.position.x, window);
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() && event.modifiers.control {
            let token = self
                .syntax
                .as_ref()
                .and_then(|syntax| syntax.token_at_byte(offset));
            let ast_node = self.syntax.as_ref().and_then(|syntax| {
                token.as_ref().and_then(|token| {
                    syntax
                        .tree()
                        .root_node()
                        .descendant_for_byte_range(token.range.start, token.range.end)
                })
            });
            let line_start = self.document.offset_of_line(line);
            let line_text = trim_eol(self.document.line_content(line).as_ref()).to_owned();
            let await_range = line_text
                .find("await")
                .map(|start| line_start + start..line_start + start + "await".len());
            tracing::info!(
                line,
                x = ?event.position.x,
                y = ?event.position.y,
                mouse_byte = offset,
                line_start,
                line_text = %line_text,
                await_range = ?await_range,
                token_text = ?token.as_ref().map(|token| token.text.as_str()),
                token_kind = ?token.as_ref().map(|token| token.kind.as_str()),
                token_range = ?token.as_ref().map(|token| token.range.clone()),
                ast_node_kind = ?ast_node.as_ref().map(|node| node.kind()),
                ast_node_range = ?ast_node.as_ref().map(|node| node.byte_range()),
                "[DEFINITION MOUSE INPUT]"
            );
        }
        if event.modifiers.control {
            if debug_input_enabled() {
                tracing::info!(
                    ctrl = true,
                    button = "left",
                    x = ?event.position.x,
                    y = ?event.position.y,
                    line,
                    byte = offset,
                    "[EDITOR CTRL CLICK]"
                );
                if let Some(token) = self
                    .syntax
                    .as_ref()
                    .and_then(|syntax| syntax.token_at_byte(offset))
                {
                    tracing::info!(text = %token.text, kind = %token.kind, "[TOKEN]");
                }
            }
            self.move_to(offset, cx);
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                tracing::info!(
                    cursor_after_move = self.document.cursor_offset(),
                    "[DEFINITION CURSOR]"
                );
            }
            self.selecting = false;
            window.dispatch_action(Definition.boxed_clone(), cx);
            return;
        }
        self.selecting = true;
        self.context_menu = None;
        self.completions.clear();
        self.hover_popup = None;
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
                    self.hover_anchor = self.hover_popup.as_ref().map(|_| {
                        let line_start = self.document.offset_of_line(line);
                        let raw_line = self.document.line_content(line);
                        let line_text = trim_eol(raw_line.as_ref());
                        let column = offset.saturating_sub(line_start).min(line_text.len());
                        let caret_x = self
                            .line_layout(line, line_text, window)
                            .x_for_index(column);
                        let scroll_y = self.scroll.0.borrow().base_handle.offset().y;
                        gpui::point(
                            px(GUTTER_WIDTH + TEXT_PADDING) + caret_x,
                            px(line as f32 * LINE_HEIGHT) - scroll_y,
                        )
                    });
                    cx.notify();
                }
            }
            return;
        }
        self.hover_popup = None;
        self.hover_anchor = None;
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

    fn editor_scroll_hover(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        self.editor_scroll_hovered = *hovered;
        cx.notify();
    }

    fn editor_scroll_drag_start(
        &mut self,
        axis: EditorScrollAxis,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = self.scroll.0.borrow().base_handle.offset();
        self.editor_scroll_drag_axis = Some(axis);
        self.editor_scroll_drag_start = event.position;
        self.editor_scroll_drag_start_offset = offset;
        cx.notify();
    }

    fn editor_scroll_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        let Some(axis) = self.editor_scroll_drag_axis else {
            return;
        };
        let handle = self.scroll.0.borrow().base_handle.clone();
        let bounds = handle.bounds();
        let max = handle.max_offset();
        let delta = event.position - self.editor_scroll_drag_start;
        let (viewport, maximum, thumb) = match axis {
            EditorScrollAxis::Vertical => {
                let viewport: f32 = bounds.size.height.into();
                let maximum: f32 = max.height.into();
                let thumb = (viewport * viewport / (viewport + maximum).max(1.0)).max(24.0);
                (viewport, maximum, thumb)
            }
            EditorScrollAxis::Horizontal => {
                let viewport: f32 = bounds.size.width.into();
                let maximum: f32 = max.width.into();
                let thumb = (viewport * viewport / (viewport + maximum).max(1.0)).max(36.0);
                (viewport, maximum, thumb)
            }
        };
        let track = (viewport - thumb).max(1.0);
        let position = match axis {
            EditorScrollAxis::Vertical => {
                let start: f32 = (-self.editor_scroll_drag_start_offset.y).into();
                let movement: f32 = delta.y.into();
                (start + movement * maximum / track).clamp(0.0, maximum)
            }
            EditorScrollAxis::Horizontal => {
                let start: f32 = (-self.editor_scroll_drag_start_offset.x).into();
                let movement: f32 = delta.x.into();
                (start + movement * maximum / track).clamp(0.0, maximum)
            }
        };
        let mut next = self.editor_scroll_drag_start_offset;
        match axis {
            EditorScrollAxis::Vertical => next.y = px(-position),
            EditorScrollAxis::Horizontal => next.x = px(-position),
        }
        handle.set_offset(next);
    }

    fn editor_scroll_drag_end(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.editor_scroll_drag_axis = None;
        cx.notify();
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
        if event.modifiers.control && debug_input_enabled() {
            tracing::info!(
                button = "right",
                reason = "right_button",
                "[EDITOR CTRL CLICK IGNORED]"
            );
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
        self.hover_anchor = None;
        self.ctrl_hover_range = None;
        cx.notify();
    }

    fn render_line(&self, line: usize, window: &mut Window) -> gpui::AnyElement {
        let t = theme();
        let raw = self.document.line_content(line);
        let text = trim_eol(raw.as_ref()).to_owned();
        let shaped_width = self.line_layout(line, &text, window).width;
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
            .w(self
                .content_width
                .max(px(GUTTER_WIDTH + TEXT_PADDING) + shaped_width))
            .h(px(LINE_HEIGHT))
            .line_height(px(LINE_HEIGHT))
            .text_size(px(FONT_SIZE))
            .font_family(CODE_FONT_FAMILY)
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
            // Keep the code column at its shaped text width so the
            // unconstrained uniform list can expose horizontal scrolling.
            .child(div().flex_none().h_full().pl_3().child(content))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindUsagesSource {
    Semantic,
    LegacyOrLsp,
}

pub(crate) fn find_usages_source(status: FindUsagesStatus) -> FindUsagesSource {
    if matches!(status, FindUsagesStatus::Complete) {
        FindUsagesSource::Semantic
    } else {
        FindUsagesSource::LegacyOrLsp
    }
}

#[cfg(test)]
fn vendor_lookup_needed(project_found: bool) -> bool {
    !project_found
}

#[cfg(test)]
fn resolve_vendor_definition_target(
    index: &Arc<std::sync::RwLock<VendorSymbolIndex>>,
    resolved: &str,
) -> Option<(PathBuf, Range<usize>)> {
    let mut index = index.write().ok()?;
    index
        .symbols_of(resolved)
        .into_iter()
        .find(|symbol| {
            matches!(
                symbol.kind,
                ProjectSymbolKind::Class
                    | ProjectSymbolKind::Interface
                    | ProjectSymbolKind::Trait
                    | ProjectSymbolKind::Enum
            )
        })
        .map(|symbol| (symbol.file, symbol.range))
        .or_else(|| index.resolve_class(resolved).map(|file| (file, 0..0)))
}

fn resolve_php_class_name(written: &str, context: &str) -> String {
    let written = written.trim().trim_start_matches('\\');
    if matches!(written, "self" | "static") {
        return declared_class_fqn(context).unwrap_or_else(|| written.to_owned());
    }
    if written == "parent" {
        return declared_parent_fqn(context).unwrap_or_else(|| written.to_owned());
    }
    if written.contains('\\') {
        return written.to_owned();
    }
    for (fqn, alias) in context.lines().flat_map(parse_use_imports) {
        if alias == written {
            return fqn;
        }
    }
    context
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("namespace ")
                .map(|value| value.trim_end_matches(';').trim())
                .filter(|namespace| !namespace.is_empty())
        })
        .map(|namespace| format!("{namespace}\\{written}"))
        .unwrap_or_else(|| written.to_owned())
}

/// Returns `(fully-qualified import, local binding)` pairs for PHP `use`
/// declarations, including grouped imports.
fn parse_use_imports(line: &str) -> Vec<(String, String)> {
    let Some(value) = line.trim().strip_prefix("use ") else {
        return Vec::new();
    };
    let value = value.split(';').next().unwrap_or(value).trim();
    if let Some((prefix, members)) = value.split_once('{') {
        let prefix = prefix.trim().trim_end_matches('\\');
        return members
            .trim_end_matches('}')
            .split(',')
            .filter_map(|member| {
                let member = member.trim();
                if member.is_empty() {
                    return None;
                }
                let (name, alias) = member
                    .split_once(" as ")
                    .map(|(name, alias)| (name.trim(), alias.trim()))
                    .unwrap_or((member, member.rsplit('\\').next().unwrap_or(member)));
                let fqn = format!("{prefix}\\{}", name.trim_matches('\\'));
                Some((fqn, alias.to_owned()))
            })
            .collect();
    }
    let (fqn, alias) = value
        .split_once(" as ")
        .map(|(fqn, alias)| (fqn.trim(), alias.trim().to_owned()))
        .unwrap_or_else(|| {
            let fqn = value.trim();
            (fqn, fqn.rsplit('\\').next().unwrap_or(fqn).to_owned())
        });
    vec![(fqn.trim_matches('\\').to_owned(), alias)]
}

fn has_import(context: &str, fqn: &str) -> bool {
    context
        .lines()
        .flat_map(parse_use_imports)
        .any(|(import, _)| import == fqn)
}

fn extract_owner_expression(text: &str, owner_end: usize) -> (usize, String) {
    let prefix = &text[..owner_end];
    let mut start = owner_end;
    for (index, ch) in prefix.char_indices().rev() {
        if ch.is_alphanumeric() || matches!(ch, '_' | '$' | '\\' | '-' | '>') {
            start = index;
        } else {
            break;
        }
    }
    (start, prefix[start..].trim().to_owned())
}

fn current_namespace(context: &str) -> String {
    context
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("namespace ")
                .map(|value| value.split(';').next().unwrap_or(value).trim().to_owned())
        })
        .unwrap_or_default()
}

fn declared_class_fqn(context: &str) -> Option<String> {
    let declared = context.lines().find_map(|line| {
        let line = line.trim();
        // `$this` inside a trait resolves in the trait's lexical context.
        // Keep the same owner-FQN path for all PHP type declarations rather
        // than only recognizing classes.
        ["class ", "trait ", "interface ", "enum "]
            .into_iter()
            .find_map(|keyword| {
                let pos = line.find(keyword)?;
                let name = line[pos + keyword.len()..]
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                    .next()?;
                (!name.is_empty()).then_some(name.to_owned())
            })
    })?;
    let namespace = current_namespace(context);
    Some(if namespace.is_empty() {
        declared
    } else {
        format!("{namespace}\\{declared}")
    })
}

fn declared_parent_fqn(context: &str) -> Option<String> {
    let parent = context.lines().find_map(|line| {
        let line = line.trim();
        let pos = line.find("extends ")?;
        let name = line[pos + 8..]
            .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '\\')
            .next()?;
        (!name.is_empty()).then_some(name.to_owned())
    })?;
    Some(resolve_php_class_name(&parent, context))
}

fn property_type_in_context(context: &str, property: &str) -> Option<String> {
    let needle = format!("${property}");
    context.lines().find_map(|line| {
        let line = line.trim();
        let pos = line.find(&needle)?;
        let before = line[..pos].trim();
        let ty = before
            .split_whitespace()
            .last()?
            .trim_start_matches(['?', '&']);
        (ty.chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '\\' | '|'))
            && !ty.is_empty())
        .then(|| ty.split('|').next().unwrap_or(ty).to_owned())
    })
}

fn native_format_php(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + text.len() / 4);
    let mut indent = 0usize;
    let mut block_comment = false;
    let mut quote: Option<char> = None;
    let mut heredoc: Option<String> = None;
    for (line_index, raw_line) in text.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            if line_index > 0 {
                result.push('\n');
            }
            continue;
        }
        if let Some(delimiter) = heredoc.as_ref() {
            if line_index > 0 {
                result.push('\n');
            }
            result.push_str(raw_line);
            if trimmed == delimiter || trimmed == format!("{delimiter};") {
                heredoc = None;
            }
            continue;
        }
        let heredoc_start = trimmed
            .find("<<<")
            .and_then(|marker| {
                let value = trimmed[marker + 3..].trim_start();
                let value = value
                    .strip_prefix('\'')
                    .or_else(|| value.strip_prefix('"'))?;
                let quote = value.chars().next_back()?;
                if quote == '\'' || quote == '"' {
                    return None;
                }
                Some(value.to_owned())
            })
            .or_else(|| {
                let value = trimmed.strip_prefix("<<<")?.trim_start();
                let delimiter = value
                    .split(|ch: char| ch.is_whitespace() || ch == ';')
                    .next()
                    .filter(|value| !value.is_empty())?;
                Some(delimiter.trim_matches(['\'', '"']).to_owned())
            });
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
        if let Some(delimiter) = heredoc_start.filter(|delimiter| !delimiter.is_empty()) {
            heredoc = Some(delimiter);
            continue;
        }
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

fn runtime_call_insert_text(symbol: &RuntimeSymbol) -> Option<String> {
    let signature = symbol.signature.as_ref()?;
    let parameters = signature
        .parameters
        .iter()
        .map(|parameter| {
            let mut name = parameter.name.trim_start_matches('$').to_owned();
            if parameter.variadic {
                name.insert_str(0, "...");
            }
            format!("${name}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("{}({parameters})", symbol.name))
}

fn project_method_detail(symbol: &axiom_index::ProjectSymbol) -> String {
    let signature = symbol.parameters.clone().unwrap_or_else(|| "()".to_owned());
    let return_type = symbol
        .return_type
        .as_deref()
        .map(|value| format!(": {value}"))
        .unwrap_or_default();
    format!("{}{}{} • Project", symbol.name, signature, return_type)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletionPresentation {
    primary: String,
    return_type: Option<String>,
    source: Option<String>,
}

fn completion_presentation(item: &CompletionItem) -> CompletionPresentation {
    let mut detail = item.detail.clone().unwrap_or_default();
    let source = detail.rsplit_once(" • ").map(|(_, source)| {
        let source = source.trim_matches(|ch| ch == '"' || ch == '\'');
        match source {
            "PhpRuntime" | "Composer" => "Runtime".to_owned(),
            "Project" => "Project".to_owned(),
            "LSP" => "LSP".to_owned(),
            other => other.to_owned(),
        }
    });
    if let Some((head, _)) = detail.rsplit_once(" • ") {
        detail = head.to_owned();
    }
    let label = item.label.trim();
    let signature = detail.find('(').and_then(|open| {
        detail
            .rfind(')')
            .filter(|close| *close >= open)
            .map(|close| {
                (
                    detail[open + 1..close].to_owned(),
                    detail[close + 1..].trim().to_owned(),
                )
            })
    });
    let (primary, return_type) = if let Some((parameters, suffix)) = signature {
        let params = parameters
            .split(',')
            .map(str::trim)
            .filter(|param| !param.is_empty())
            .map(compress_completion_parameter)
            .collect::<Vec<_>>()
            .join(", ");
        let return_type = suffix
            .strip_prefix(':')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        (
            format!("{}({params})", label.trim_end_matches("()")),
            return_type,
        )
    } else {
        (label.to_owned(), None)
    };
    CompletionPresentation {
        primary,
        return_type,
        source,
    }
}

fn compress_completion_parameter(parameter: &str) -> String {
    let mut value = parameter.trim().trim_start_matches('&').trim();
    value = value.strip_prefix("...").unwrap_or(value);
    value = value.split('=').next().unwrap_or(value).trim();
    value
        .rsplit_once('$')
        .map(|(_, name)| format!("${}", name.trim()))
        .unwrap_or_else(|| value.to_owned())
}

fn completion_icon(kind: Option<CompletionItemKind>) -> &'static str {
    match kind {
        Some(CompletionItemKind::METHOD | CompletionItemKind::FUNCTION) => "ƒ",
        Some(CompletionItemKind::CLASS | CompletionItemKind::CONSTRUCTOR) => "C",
        Some(CompletionItemKind::INTERFACE) => "I",
        Some(CompletionItemKind::ENUM) => "E",
        Some(CompletionItemKind::STRUCT) => "S",
        Some(CompletionItemKind::PROPERTY | CompletionItemKind::FIELD) => "·",
        Some(CompletionItemKind::CONSTANT | CompletionItemKind::ENUM_MEMBER) => "#",
        _ => "•",
    }
}

#[cfg(debug_assertions)]
fn debug_input_enabled() -> bool {
    std::env::var_os("AXIOM_DEBUG_INPUT").is_some()
}

#[cfg(not(debug_assertions))]
fn debug_input_enabled() -> bool {
    false
}

#[cfg(debug_assertions)]
fn debug_completion_enabled() -> bool {
    std::env::var_os("AXIOM_DEBUG_COMPLETION").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    })
}

#[cfg(not(debug_assertions))]
fn debug_completion_enabled() -> bool {
    false
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let viewport = self.scroll.0.borrow().base_handle.bounds();
        let scroll_handle = self.scroll.0.borrow().base_handle.clone();
        let scroll_max = scroll_handle.max_offset();
        let viewport_w: f32 = viewport.size.width.into();
        let viewport_h: f32 = viewport.size.height.into();
        let max_x: f32 = scroll_max.width.into();
        let max_y: f32 = scroll_max.height.into();
        let vertical_thumb = (viewport_h * viewport_h / (viewport_h + max_y).max(1.0))
            .max(24.0)
            .min(viewport_h.max(24.0));
        let horizontal_thumb = (viewport_w * viewport_w / (viewport_w + max_x).max(1.0))
            .max(36.0)
            .min(viewport_w.max(36.0));
        let offset = scroll_handle.offset();
        let offset_x: f32 = (-offset.x).into();
        let offset_y: f32 = (-offset.y).into();
        let mut content_width = viewport.size.width;
        for line in 0..self.document.line_count() {
            let raw = self.document.line_content(line);
            let text = trim_eol(raw.as_ref());
            let width =
                px(GUTTER_WIDTH + TEXT_PADDING) + self.line_layout(line, text, window).width;
            if width > content_width {
                content_width = width;
            }
        }
        self.content_width = content_width;
        let vertical_top = if max_y > 0.0 {
            offset_y / max_y * (viewport_h - vertical_thumb).max(0.0)
        } else {
            0.0
        };
        let horizontal_left = if max_x > 0.0 {
            offset_x / max_x * (viewport_w - horizontal_thumb).max(0.0)
        } else {
            0.0
        };
        let line = self.document.line_of_offset(self.document.cursor_offset());
        let line_start = self.document.offset_of_line(line);
        let line_content = self.document.line_content(line);
        let line_text = trim_eol(line_content.as_ref());
        let caret_x = shape(window, line_text).x_for_index(
            self.document
                .cursor_offset()
                .saturating_sub(line_start)
                .min(line_text.len()),
        );
        let presentations = self
            .completions
            .iter()
            .map(completion_presentation)
            .collect::<Vec<_>>();
        let estimated_width = presentations
            .iter()
            .map(|item| {
                item.primary.len().max(8) as f32 * 7.0
                    + item
                        .return_type
                        .as_ref()
                        .map_or(0.0, |v| v.len() as f32 * 6.0 + 12.0)
                    + item
                        .source
                        .as_ref()
                        .map_or(0.0, |v| v.len() as f32 * 6.0 + 12.0)
                    + 54.0
            })
            .fold(280.0, f32::max);
        let viewport_width: f32 = viewport.size.width.into();
        let popup_width = px(estimated_width
            .min(620.0)
            .min((viewport_width - 16.0).max(180.0)));
        let mut popup_x = viewport.left() + px(GUTTER_WIDTH + TEXT_PADDING) + caret_x;
        popup_x = popup_x.min((viewport.right() - popup_width).max(viewport.left()));
        let mut below_y = viewport.top() + px((line as f32 + 1.0) * LINE_HEIGHT)
            - self.scroll.0.borrow().base_handle.offset().y;
        let row_height = px(28.);
        let popup_height = px((presentations.len() as f32 * 28.0).min(224.0));
        if let Some(anchor) = self.hover_anchor {
            popup_x = anchor.x.max(viewport.left());
            popup_x = popup_x.min((viewport.right() - popup_width).max(viewport.left()));
            below_y = (anchor.y + px(LINE_HEIGHT)).min(viewport.bottom());
        }
        let opens_above = below_y + popup_height > viewport.bottom();
        let popup_y = if opens_above {
            (below_y - popup_height - px(4.)).max(viewport.top())
        } else {
            below_y.min((viewport.bottom() - popup_height).max(viewport.top()))
        };
        if self.completions.is_empty() {
            self.last_completion_layout = None;
        }
        if debug_completion_enabled() && !self.completions.is_empty() {
            let x: f32 = popup_x.into();
            let y: f32 = popup_y.into();
            let width: f32 = popup_width.into();
            let height: f32 = popup_height.into();
            let key = (x as u32, y as u32, width as u32, height as u32, opens_above);
            if self.last_completion_layout != Some(key) {
                self.last_completion_layout = Some(key);
                tracing::info!(
                    items = self.completions.len(),
                    x,
                    y,
                    width,
                    height,
                    row_height = 28,
                    max_width = 620,
                    placement = if opens_above { "above" } else { "below" },
                    "[COMPLETION LAYOUT]"
                );
            }
        }
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
            .on_action(cx.listener(Self::reformat))
            .on_action(cx.listener(Self::signature_help))
            .on_action(cx.listener(Self::complete_statement))
            .on_action(cx.listener(Self::escape))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .child(
                div()
                    .id("editor-viewport")
                    .relative()
                    .flex_1()
                    .on_hover(cx.listener(Self::editor_scroll_hover))
                    .on_mouse_move(cx.listener(Self::editor_scroll_drag_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::editor_scroll_drag_end))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::editor_scroll_drag_end))
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
                        .with_horizontal_sizing_behavior(
                            ListHorizontalSizingBehavior::Unconstrained,
                        )
                        .track_scroll(self.scroll.clone())
                        .h_full(),
                    )
                    .when(self.editor_scroll_hovered && max_y > 0.0, |this| {
                        this.child(
                            div()
                                .absolute()
                                .right(px(2.))
                                .top(px(0.))
                                .bottom(px(8.))
                                .w(px(8.))
                                .bg(t.scrollbar)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, event, window, cx| {
                                        this.editor_scroll_drag_start(
                                            EditorScrollAxis::Vertical,
                                            event,
                                            window,
                                            cx,
                                        )
                                    }),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(1.))
                                        .right(px(1.))
                                        .top(px(vertical_top))
                                        .h(px(vertical_thumb))
                                        .rounded(px(4.))
                                        .bg(t.scrollbar_hover),
                                ),
                        )
                    })
                    .when(self.editor_scroll_hovered && max_x > 0.0, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(0.))
                                .right(px(8.))
                                .bottom(px(2.))
                                .h(px(8.))
                                .bg(t.scrollbar)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, event, window, cx| {
                                        this.editor_scroll_drag_start(
                                            EditorScrollAxis::Horizontal,
                                            event,
                                            window,
                                            cx,
                                        )
                                    }),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(horizontal_left))
                                        .top(px(1.))
                                        .bottom(px(1.))
                                        .w(px(horizontal_thumb))
                                        .rounded(px(4.))
                                        .bg(t.scrollbar_hover),
                                ),
                        )
                    })
                    .when(!self.completions.is_empty(), |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(popup_x)
                                .top(popup_y)
                                .w(popup_width)
                                .max_h(px(224.))
                                .id("completion-popup-scroll")
                                .overflow_y_scroll()
                                .rounded(m.border_radius_medium)
                                .bg(t.popup_background)
                                .border_1()
                                .border_color(t.border)
                                .shadow_lg()
                                .occlude()
                                .children(
                                    self.completions
                                        .iter()
                                        .enumerate()
                                        .zip(presentations.iter())
                                        .map(|((index, item), presentation)| {
                                            let editor = cx.entity();
                                            div()
                                                .id(("completion-item", index))
                                                .h(row_height)
                                                .w_full()
                                                .px_2()
                                                .flex()
                                                .items_center()
                                                .overflow_hidden()
                                                .on_click(move |_, _, cx| {
                                                    editor.update(cx, |this, cx| {
                                                        this.completion_selected = index;
                                                        this.accept_completion(cx);
                                                    });
                                                })
                                                .bg(if index == self.completion_selected {
                                                    t.selection
                                                } else {
                                                    t.popup_background
                                                })
                                                .text_color(t.text_primary)
                                                .child(
                                                    div()
                                                        .w(px(18.))
                                                        .flex_none()
                                                        .text_color(t.info)
                                                        .child(completion_icon(item.kind)),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w(px(0.))
                                                        .overflow_hidden()
                                                        .child(presentation.primary.clone()),
                                                )
                                                .when_some(
                                                    presentation.return_type.clone(),
                                                    |row, return_type| {
                                                        row.child(
                                                            div()
                                                                .max_w(px(120.))
                                                                .flex_none()
                                                                .overflow_hidden()
                                                                .ml_2()
                                                                .text_color(t.text_muted)
                                                                .child(return_type),
                                                        )
                                                    },
                                                )
                                                .when_some(
                                                    presentation.source.clone(),
                                                    |row, source| {
                                                        row.child(
                                                            div()
                                                                .max_w(px(64.))
                                                                .flex_none()
                                                                .overflow_hidden()
                                                                .ml_2()
                                                                .text_color(t.text_muted)
                                                                .child(source),
                                                        )
                                                    },
                                                )
                                        }),
                                ),
                        )
                    })
                    .when_some(self.hover_popup.clone(), |this, hover| {
                        this.child(
                            div()
                                .absolute()
                                .left(popup_x)
                                .top(if opens_above {
                                    below_y
                                } else {
                                    popup_y + popup_height + px(4.)
                                })
                                .max_w(px(520.))
                                .p_3()
                                .rounded(m.border_radius_medium)
                                .bg(t.popup_background)
                                .border_1()
                                .border_color(t.border)
                                .shadow_lg()
                                .occlude()
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
        let smart_arrow =
            text == "-" && range.start == range.end && self.should_expand_member_dash();
        if smart_arrow && debug_completion_enabled() {
            tracing::info!(
                converted = true,
                receiver = "previous-token",
                "[SMART ARROW]"
            );
        }
        self.document.set_selection(range.start, range.end);
        let insertion = if smart_arrow { "->" } else { text };
        self.insert_text_with_pairs(insertion);
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

fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in text[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn project_callable_detail(symbol: &axiom_index::ProjectSymbol) -> Option<String> {
    Some(format!(
        "{}{}{}",
        symbol.name,
        symbol.parameters.as_deref().unwrap_or("()"),
        symbol
            .return_type
            .as_deref()
            .map(|value| format!(": {value}"))
            .unwrap_or_default()
    ))
}

fn signature_counts_from_detail(detail: &str) -> (usize, usize, bool) {
    let Some(open) = detail.find('(') else {
        return (0, 0, false);
    };
    let Some(close) = matching_paren(detail, open) else {
        return (0, 0, false);
    };
    let parameters = &detail[open + 1..close];
    if parameters.trim().is_empty() {
        return (0, 0, false);
    }
    let mut required = 0;
    let mut maximum = 0;
    let mut variadic = false;
    for parameter in parameters.split(',') {
        let parameter = parameter.trim();
        if parameter.is_empty() {
            continue;
        }
        maximum += 1;
        if parameter.contains("...") {
            variadic = true;
        }
        if !parameter.contains('=') && !parameter.contains("...") {
            required += 1;
        }
    }
    (required, maximum, variadic)
}

fn count_call_arguments(arguments: &str) -> usize {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let mut depth = 0usize;
    let mut count = 1usize;
    for ch in trimmed.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
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
        font: code_font(),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(text, px(14.), &[run], None)
}

#[cfg(test)]
mod formatter_tests {
    use super::{
        Arc, DefinitionQuery, EditorView, FindUsagesSource, ProjectSymbolIndex, VendorSymbolIndex,
        completion_presentation, declared_class_fqn, declared_parent_fqn, extract_owner_expression,
        find_usages_source, native_format_php, property_type_in_context, resolve_php_class_name,
        resolve_vendor_definition_target, vendor_lookup_needed,
    };
    use axiom_index::FindUsagesStatus;
    use lsp_types::CompletionItem;
    use std::time::Duration;

    #[gpui::test]
    fn definition_query_uses_method_at_real_editor_cursor(cx: &mut gpui::TestAppContext) {
        let source = "<?php\nnamespace Omegaalfa\\HttpClient\\Http;\nuse Omegaalfa\\FiberEventLoop\\Future;\nfunction await(Future $future): mixed\n{\n    return $future->await();\n}\n";
        let path = std::env::temp_dir().join("axiom-definition-query-integration.php");
        let source_for_view = source.to_owned();
        let path_for_view = path.clone();
        let (view, cx) = cx.add_window_view(move |_, cx| {
            EditorView::from_document(
                path_for_view,
                axiom_editor::Document::from_content(&source_for_view),
                None,
                cx,
            )
        });
        let offset = source.rfind("await").unwrap() + 2;
        view.update(cx, |editor, _| editor.document.move_cursor(offset));
        let query = view.update(cx, |editor, _| editor.definition_query());
        assert!(
            matches!(
                &query,
                Some(DefinitionQuery::Method {
                    owner_fqn,
                    name,
                    is_static: false,
                }) if owner_fqn == "Omegaalfa\\FiberEventLoop\\Future" && name == "await"
            ),
            "unexpected definition query: {query:?}"
        );
    }

    #[gpui::test]
    fn definition_query_uses_declared_fqn_for_this_receiver(cx: &mut gpui::TestAppContext) {
        let source = "<?php\nnamespace Omegaalfa\\FiberEventLoop;\nclass Future {\n    protected $loop;\n    public function await(): mixed {\n        $this->loop->next();\n    }\n}\n";
        let path = std::env::temp_dir().join("Future.php");
        let source_for_view = source.to_owned();
        let path_for_view = path.clone();
        let (view, cx) = cx.add_window_view(move |_, cx| {
            EditorView::from_document(
                path_for_view,
                axiom_editor::Document::from_content(&source_for_view),
                None,
                cx,
            )
        });
        let offset = source.rfind("loop->next").unwrap();
        view.update(cx, |editor, _| editor.document.move_cursor(offset));
        let query = view.update(cx, |editor, _| editor.definition_query());
        assert!(
            matches!(
                &query,
                Some(DefinitionQuery::Method { owner_fqn, name, .. })
                    if owner_fqn == "Omegaalfa\\FiberEventLoop\\Future" && name == "loop"
            ),
            "unexpected $this receiver query: {query:?}"
        );
    }

    #[gpui::test]
    fn definition_query_resolves_trait_members_and_imported_new_types(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = "<?php\nnamespace Omegaalfa\\FiberEventLoop\\Traits;\nuse Omegaalfa\\FiberEventLoop\\Future;\ntrait FiberManagerTrait\n{\n    protected int $nextId = 1;\n    protected function generateId(): int\n    {\n        return $this->nextId++;\n    }\n    public function defer(callable $callable): int\n    {\n        $id = $this->generateId();\n        return $id;\n    }\n    public function async(callable $callable): Future\n    {\n        $future = new Future($this);\n        return $future;\n    }\n}\n";
        let path = std::env::temp_dir().join("axiom-trait-definition-query.php");
        let source_for_view = source.to_owned();
        let path_for_view = path.clone();
        let (view, cx) = cx.add_window_view(move |_, cx| {
            EditorView::from_document(
                path_for_view,
                axiom_editor::Document::from_content(&source_for_view),
                None,
                cx,
            )
        });

        let mut assert_method = |offset: usize, name: &str| {
            view.update(cx, |editor, _| editor.document.move_cursor(offset));
            let query = view.update(cx, |editor, _| editor.definition_query());
            assert!(
                matches!(
                    &query,
                    Some(DefinitionQuery::Method { owner_fqn, name: actual, .. })
                        if owner_fqn == "Omegaalfa\\FiberEventLoop\\Traits\\FiberManagerTrait"
                            && actual == name
                ),
                "unexpected trait query for {name}: {query:?}"
            );
        };

        let generate = source.find("$this->generateId").unwrap() + "$this->".len();
        assert_method(generate, "generateId");
        let next_id = source.find("$this->nextId").unwrap() + "$this->".len();
        assert_method(next_id, "nextId");

        let future = source.find("new Future").unwrap() + "new ".len() + 2;
        view.update(cx, |editor, _| editor.document.move_cursor(future));
        let query = view.update(cx, |editor, _| editor.definition_query());
        assert!(
            matches!(
                &query,
                Some(DefinitionQuery::Name { fqn, written })
                    if fqn == "Omegaalfa\\FiberEventLoop\\Future" && written == "Future"
            ),
            "unexpected imported type query: {query:?}"
        );
    }

    #[test]
    fn declared_owner_fqn_supports_interfaces_and_traits() {
        let interface = "<?php\nnamespace Example;\ninterface Contract {\n    public function run(): void;\n}\n";
        assert_eq!(
            declared_class_fqn(interface).as_deref(),
            Some("Example\\Contract")
        );
        let trait_source = "<?php\nnamespace Example;\ntrait Helpers {\n    protected function run(): void {}\n}\n";
        assert_eq!(
            declared_class_fqn(trait_source).as_deref(),
            Some("Example\\Helpers")
        );
    }

    #[test]
    fn find_usages_source_policy_routes_only_complete_to_semantic() {
        assert_eq!(
            find_usages_source(FindUsagesStatus::Complete),
            FindUsagesSource::Semantic
        );
        for status in [
            FindUsagesStatus::Partial,
            FindUsagesStatus::Ambiguous,
            FindUsagesStatus::Deferred,
            FindUsagesStatus::Stale,
        ] {
            assert_eq!(find_usages_source(status), FindUsagesSource::LegacyOrLsp);
        }
    }

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

    #[test]
    fn heredoc_content_does_not_change_block_indentation() {
        let input = "<?php\nfunction render(){\n$value = <<<EOT\n{ not a PHP block }\nEOT;\nreturn $value;\n}\n";
        let output = native_format_php(input);
        assert!(output.contains("EOT;\n    return $value;"));
        assert!(!output.contains("        return $value;"));
    }

    #[test]
    fn completion_source_separated_from_return_type() {
        let item = CompletionItem {
            label: "ghost".into(),
            detail: Some(
                "ghost(Closure $initializer, string|object $class): LazyObject • PhpRuntime".into(),
            ),
            ..Default::default()
        };
        let view = completion_presentation(&item);
        assert_eq!(view.primary, "ghost($initializer, $class)");
        assert_eq!(view.return_type.as_deref(), Some("LazyObject"));
        assert_eq!(view.source.as_deref(), Some("Runtime"));
    }

    #[test]
    fn completion_long_signature_is_truncated_to_compact_parameters() {
        let item = CompletionItem {
            label: "reflect".into(),
            detail: Some("reflect(string|object $class = null): ReflectionClass • Project".into()),
            ..Default::default()
        };
        assert_eq!(completion_presentation(&item).primary, "reflect($class)");
    }

    #[test]
    fn completion_popup_layout_constants_keep_rows_deterministic() {
        let row_height = 28.0_f32;
        let max_width = 620.0_f32;
        assert!(row_height > 0.0 && row_height <= 32.0);
        assert!((280.0..=700.0).contains(&max_width));
        assert_eq!(row_height, 28.0);
    }

    #[test]
    fn debounce_worker_exits_when_editor_channel_is_dropped() {
        let (sender, receiver) = std::sync::mpsc::channel::<super::IndexUpdateRequest>();
        let worker = std::thread::spawn(|| super::run_index_update_worker(receiver));
        drop(sender);
        worker.join().expect("debounce worker should terminate");
    }

    #[test]
    fn imported_class_not_unknown() {
        assert_eq!(
            resolve_php_class_name(
                "FiberEventLoop",
                "namespace Omegaalfa\\HttpClient\\Http;\nuse Omegaalfa\\FiberEventLoop\\FiberEventLoop;"
            ),
            "Omegaalfa\\FiberEventLoop\\FiberEventLoop"
        );
    }

    #[test]
    fn group_import_resolves_members_and_aliases() {
        let context = r#"use Foo\Bar\{Baz, Qux as Q};"#;
        assert_eq!(resolve_php_class_name("Baz", context), "Foo\\Bar\\Baz");
        assert_eq!(resolve_php_class_name("Q", context), "Foo\\Bar\\Qux");
    }

    #[test]
    fn simple_import_forms_remain_unchanged() {
        assert_eq!(resolve_php_class_name("Bar", "use Foo\\Bar;"), "Foo\\Bar");
        assert_eq!(
            resolve_php_class_name("Alias", "use Foo\\Bar as Alias;"),
            "Foo\\Bar"
        );
        assert!(super::has_import("use Foo\\Bar\\{Baz};", "Foo\\Bar\\Baz"));
    }

    #[test]
    fn poisoned_project_index_read_is_currently_discarded() {
        let index = std::sync::Arc::new(std::sync::RwLock::new(ProjectSymbolIndex::new()));
        let poisoned = index.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.write().expect("write lock");
            panic!("intentional poison");
        })
        .join();
        assert!(index.try_read().is_err());
    }

    #[test]
    fn aliased_import_not_unknown() {
        assert_eq!(
            resolve_php_class_name(
                "Loop",
                "use Omegaalfa\\FiberEventLoop\\FiberEventLoop as Loop;"
            ),
            "Omegaalfa\\FiberEventLoop\\FiberEventLoop"
        );
    }

    #[test]
    fn same_namespace_class_not_unknown() {
        assert_eq!(
            resolve_php_class_name("UserService", "namespace App\\Service;"),
            "App\\Service\\UserService"
        );
    }

    #[test]
    fn fully_qualified_class_not_unknown() {
        assert_eq!(
            resolve_php_class_name("\\Vendor\\Package\\Thing", "namespace App;"),
            "Vendor\\Package\\Thing"
        );
    }

    #[test]
    fn actually_unknown_class_remains_qualified() {
        assert_eq!(
            resolve_php_class_name("Missing", "namespace App\\Service;"),
            "App\\Service\\Missing"
        );
    }

    #[test]
    fn self_and_static_resolve_to_declared_class() {
        let text =
            "namespace App\\Http; class Client { function f() { self::run(); new static(); } }";
        assert_eq!(
            declared_class_fqn(text).as_deref(),
            Some("App\\Http\\Client")
        );
        assert_eq!(resolve_php_class_name("self", text), "App\\Http\\Client");
        assert_eq!(resolve_php_class_name("static", text), "App\\Http\\Client");
    }

    #[test]
    fn parent_resolves_extends_and_is_safe_without_extends() {
        let text = "namespace App\\Http;\nuse Base\\Client as ParentClient;\nclass Child extends ParentClient {}";
        assert_eq!(declared_parent_fqn(text).as_deref(), Some("Base\\Client"));
        assert_eq!(resolve_php_class_name("parent", text), "Base\\Client");
        assert_eq!(
            resolve_php_class_name("parent", "namespace App; class Child {}"),
            "parent"
        );
    }

    #[test]
    fn receiver_chain_and_typed_property_resolve() {
        let text = "namespace App;\nuse Vendor\\FiberEventLoop\\FiberEventLoop;\nclass Client {\n private FiberEventLoop $loop;\n function f() { $this->loop->run(); }\n}";
        let operator = text.find("$this->loop->run").unwrap() + "$this->loop".len();
        let (_, owner) = extract_owner_expression(text, operator);
        assert_eq!(owner, "$this->loop");
        assert_eq!(
            property_type_in_context(text, "loop").as_deref(),
            Some("FiberEventLoop")
        );
        assert_eq!(
            resolve_php_class_name("FiberEventLoop", text),
            "Vendor\\FiberEventLoop\\FiberEventLoop"
        );
    }

    #[test]
    fn vendor_definition_target_does_not_deadlock_on_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let composer = dir.path().join("vendor/composer");
        let file = dir.path().join("vendor/acme/pkg/src/FiberEventLoop.php");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&composer).unwrap();
        std::fs::write(&file, "<?php namespace Acme; class FiberEventLoop {}").unwrap();
        std::fs::write(
            composer.join("autoload_classmap.php"),
            "<?php return ['Acme\\\\FiberEventLoop' => $vendorDir . '/acme/pkg/src/FiberEventLoop.php'];",
        )
        .unwrap();
        let index = Arc::new(std::sync::RwLock::new(
            VendorSymbolIndex::load(dir.path()).unwrap(),
        ));
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_index = index.clone();
        std::thread::spawn(move || {
            let result = resolve_vendor_definition_target(&worker_index, "Acme\\FiberEventLoop");
            let _ = sender.send(result);
        });
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("vendor definition resolution timed out (possible RwLock deadlock)");
        assert!(result.is_some());
    }

    #[test]
    fn project_hit_short_circuits_vendor_lookup() {
        assert!(!vendor_lookup_needed(true));
        assert!(vendor_lookup_needed(false));
    }
}
