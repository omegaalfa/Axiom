use axiom_editor::{Document, DocumentEdit};
use axiom_index::{
    DeclaredType, DefinitionSyntaxContext, FindUsagesStatus, MemberAccess, MemberResolution,
    PersistentFileKey, ProjectSymbolIndex, ProjectSymbolKind, SemanticEngine, SemanticSnapshot,
    TypeCompatibility, VendorSymbolIndex, declared_type_compatibility, declared_type_label,
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
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
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

pub type DocumentSessionId = u64;

static NEXT_DOCUMENT_SESSION: AtomicU64 = AtomicU64::new(1);
pub(crate) static LAST_UI_EDIT_GENERATION: AtomicU64 = AtomicU64::new(0);
pub(crate) static LAST_UI_HEARTBEAT_NS: AtomicU64 = AtomicU64::new(0);
pub(crate) static UI_STAGE: AtomicU64 = AtomicU64::new(0);
pub(crate) static UI_STAGE_ENTERED_AT_NS: AtomicU64 = AtomicU64::new(0);
pub(crate) static LAST_UI_KEY_EVENT_ID: AtomicU64 = AtomicU64::new(0);
pub(crate) static LAST_UI_RENDERED_GENERATION: AtomicU64 = AtomicU64::new(0);
pub(crate) static CARET_TASKS_STARTED: AtomicU64 = AtomicU64::new(0);
pub(crate) static CARET_TASKS_ACTIVE: AtomicU64 = AtomicU64::new(0);

pub(crate) const UI_STAGE_KEY_CALLBACK: u64 = 1;
pub(crate) const UI_STAGE_AFTER_EDIT: u64 = 2;
pub(crate) const UI_STAGE_COMPLETION: u64 = 3;
pub(crate) const UI_STAGE_RENDER: u64 = 4;
pub(crate) const UI_STAGE_WIDTH_CACHE_REBUILD: u64 = 5;
pub(crate) const UI_STAGE_POLL_CYCLE: u64 = 6;
pub(crate) const UI_STAGE_POLL_LSP: u64 = 7;
pub(crate) const UI_STAGE_POLL_INDEX: u64 = 8;
pub(crate) const UI_STAGE_POLL_VENDOR: u64 = 9;
pub(crate) const UI_STAGE_POLL_PROJECT: u64 = 10;
pub(crate) const UI_STAGE_POLL_RUNTIME_STUB: u64 = 11;
pub(crate) const UI_STAGE_POLL_RUNTIME_WATCHER: u64 = 12;
pub(crate) const UI_STAGE_POLL_SEMANTIC: u64 = 13;
pub(crate) const UI_STAGE_SEMANTIC_PUBLISH: u64 = 14;
pub(crate) const UI_STAGE_TAB_UPDATE: u64 = 15;

pub(crate) fn ui_stage_name(stage: u64) -> &'static str {
    match stage {
        UI_STAGE_KEY_CALLBACK => "key_callback",
        UI_STAGE_AFTER_EDIT => "after_edit",
        UI_STAGE_COMPLETION => "completion",
        UI_STAGE_RENDER => "render",
        UI_STAGE_WIDTH_CACHE_REBUILD => "width_cache_rebuild",
        UI_STAGE_POLL_CYCLE => "poll_cycle",
        UI_STAGE_POLL_LSP => "poll_lsp",
        UI_STAGE_POLL_INDEX => "poll_index",
        UI_STAGE_POLL_VENDOR => "poll_vendor",
        UI_STAGE_POLL_PROJECT => "poll_project",
        UI_STAGE_POLL_RUNTIME_STUB => "poll_runtime_stub",
        UI_STAGE_POLL_RUNTIME_WATCHER => "poll_runtime_watcher",
        UI_STAGE_POLL_SEMANTIC => "poll_semantic",
        UI_STAGE_SEMANTIC_PUBLISH => "semantic_publish",
        UI_STAGE_TAB_UPDATE => "tab_update",
        _ => "idle",
    }
}

pub(crate) fn ui_clock_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

pub(crate) struct UiStageGuard {
    previous: u64,
}

impl UiStageGuard {
    pub(crate) fn new(stage: u64) -> Self {
        let previous = UI_STAGE.swap(stage, Ordering::Relaxed);
        UI_STAGE_ENTERED_AT_NS.store(ui_clock_ns(), Ordering::Relaxed);
        Self { previous }
    }
}

impl Drop for UiStageGuard {
    fn drop(&mut self) {
        UI_STAGE.store(self.previous, Ordering::Relaxed);
        UI_STAGE_ENTERED_AT_NS.store(ui_clock_ns(), Ordering::Relaxed);
    }
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
    document_session: DocumentSessionId,
    last_lsp_text: String,
    completions: Vec<CompletionItem>,
    completion_selected: usize,
    hover_popup: Option<String>,
    hover_anchor: Option<Point<Pixels>>,
    diagnostics: DiagnosticStore,
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
    semantic_update_sender: Option<Sender<(u64, PathBuf, String)>>,
    semantic_update_generation: u64,
    workspace_root: Option<PathBuf>,
    workspace_source: bool,
    editor_scroll_hovered: bool,
    editor_scroll_drag_axis: Option<EditorScrollAxis>,
    editor_scroll_drag_start: Point<Pixels>,
    editor_scroll_drag_start_offset: Point<Pixels>,
    content_width: Pixels,
    line_width_cache: HashMap<usize, Pixels>,
    max_width_line: Option<usize>,
    width_cache_line_count: usize,
    width_cache_dirty: bool,
    native_inspection_generation: u64,
    native_inspection_latest_generation: Arc<AtomicU64>,
    initial_native_inspection_scheduled: bool,
    native_inspection_sender: Sender<NativeInspectionResult>,
    native_inspection_results: Receiver<NativeInspectionResult>,
    caret_visible: bool,
    caret_blink_generation: u64,
    caret_last_activity: Instant,
    caret_last_toggle: Instant,
    caret_blink_task_active: bool,
    edit_generation: u64,
    last_mutation_at: Option<Instant>,
    last_rendered_edit_generation: u64,
    last_render_at: Option<Instant>,
    last_notify_at: Option<(u64, Instant)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorScrollAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorScrollEventSource {
    Viewport,
    Scrollbar,
}

#[derive(Clone)]
struct ByteDiagnostic {
    range: Range<usize>,
    severity: Option<DiagnosticSeverity>,
    message: String,
}

struct NativeInspectionResult {
    document_session: DocumentSessionId,
    generation: u64,
    diagnostics: Vec<ByteDiagnostic>,
}

struct NativeInspectionWorkItem {
    document_session: DocumentSessionId,
    generation: u64,
    latest_generation: Arc<AtomicU64>,
    unknown_class: UnknownClassInspectionInput,
    unknown_constant: UnknownConstantInspectionInput,
    duplicate_class: DuplicateClassInspectionInput,
    arguments: ArgumentInspectionInput,
}

enum NativeInspectionCapture {
    Ready(NativeInspectionWorkItem),
    RetryLockBusy,
    Unavailable,
}

const NATIVE_INSPECTION_CAPTURE_RETRY_DELAY_MS: u64 = 15;
const NATIVE_INSPECTION_CAPTURE_MAX_LOCK_RETRIES: usize = 3;

fn run_native_inspection_rules<UnknownClass, UnknownConstant, DuplicateClass, Arguments>(
    latest_generation: &AtomicU64,
    generation: u64,
    unknown_class: UnknownClass,
    unknown_constant: UnknownConstant,
    duplicate_class: DuplicateClass,
    arguments: Arguments,
) -> Option<Vec<ByteDiagnostic>>
where
    UnknownClass: FnOnce() -> Vec<ByteDiagnostic>,
    UnknownConstant: FnOnce() -> Vec<ByteDiagnostic>,
    DuplicateClass: FnOnce() -> Vec<ByteDiagnostic>,
    Arguments: FnOnce() -> Vec<ByteDiagnostic>,
{
    let stale = || latest_generation.load(Ordering::Acquire) != generation;
    if stale() {
        return None;
    }
    let mut diagnostics = unknown_class();
    if stale() {
        return None;
    }
    diagnostics.extend(unknown_constant());
    if stale() {
        return None;
    }
    diagnostics.extend(duplicate_class());
    if stale() {
        return None;
    }
    diagnostics.extend(arguments());
    Some(diagnostics)
}

#[derive(Clone)]
struct UnknownClassInspectionInput {
    text: Arc<str>,
    project_classes: std::collections::HashSet<String>,
    runtime_symbols: Option<Arc<RuntimeSymbolIndex>>,
    vendor_symbols: Option<Arc<VendorSymbolIndex>>,
}

fn compute_unknown_class_inspections(input: &UnknownClassInspectionInput) -> Vec<ByteDiagnostic> {
    let Ok(syntax) = PhpSyntax::parse(input.text.as_ref()) else {
        return Vec::new();
    };
    let mut diagnostics = Vec::new();
    let mut offset = 0;
    while let Some(relative) = input.text[offset..].find("new ") {
        let start = offset + relative + 4;
        let Some(node) = syntax
            .tree()
            .root_node()
            .descendant_for_byte_range(start, start + 1)
        else {
            break;
        };
        if node.kind() == "comment" || node.kind().contains("string") {
            offset = start + 1;
            continue;
        }
        let end = start
            + input.text[start..]
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '\\')
                .map(char::len_utf8)
                .sum::<usize>();
        if end > start {
            let written = &input.text[start..end];
            let name = written.trim_start_matches('\\');
            let resolved = resolve_php_class_name(written, input.text.as_ref());
            let known_project =
                input.project_classes.contains(&resolved) || input.project_classes.contains(name);
            let known_runtime = input.runtime_symbols.as_ref().is_some_and(|index| {
                index.find_class(&resolved).is_some()
                    || index.find_class_by_short_name(name).is_some()
            });
            let known_vendor = input
                .vendor_symbols
                .as_ref()
                .is_some_and(|index| index.has_class_metadata(&resolved));
            if !known_project
                && !known_runtime
                && !known_vendor
                && !matches!(name, "self" | "static" | "parent")
            {
                diagnostics.push(ByteDiagnostic {
                    range: start..end,
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: format!("Unknown class '{name}'"),
                });
            }
        }
        offset = end.max(start + 1);
        if offset >= input.text.len() {
            break;
        }
    }
    diagnostics
}

#[derive(Clone)]
struct UnknownConstantInspectionInput {
    text: Arc<str>,
    known_constants: Vec<(String, String)>,
    runtime_symbols: Option<Arc<RuntimeSymbolIndex>>,
}

#[derive(Clone)]
struct DuplicateClassDeclaration {
    fqn: String,
    file: PersistentFileKey,
    range: Range<usize>,
}

#[derive(Clone)]
struct DuplicateClassInspectionInput {
    path: PersistentFileKey,
    declarations: Vec<DuplicateClassDeclaration>,
}

fn compute_duplicate_class_inspections(
    input: &DuplicateClassInspectionInput,
) -> Vec<ByteDiagnostic> {
    let mut groups: HashMap<String, Vec<&DuplicateClassDeclaration>> = HashMap::new();
    for declaration in &input.declarations {
        groups
            .entry(declaration.fqn.clone())
            .or_default()
            .push(declaration);
    }
    groups
        .into_values()
        .filter(|declarations| {
            declarations
                .iter()
                .any(|declaration| declaration.file == input.path)
                && declarations
                    .iter()
                    .filter(|declaration| declaration.file != input.path)
                    .count()
                    > 0
        })
        .flat_map(|declarations| {
            declarations
                .into_iter()
                .filter(|declaration| declaration.file == input.path)
        })
        .map(|declaration| ByteDiagnostic {
            range: declaration.range.clone(),
            severity: Some(DiagnosticSeverity::ERROR),
            message: format!("Duplicate class {}", declaration.fqn),
        })
        .collect()
}

fn compute_unknown_constant_inspections(
    input: &UnknownConstantInspectionInput,
) -> Vec<ByteDiagnostic> {
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
    let Ok(syntax) = PhpSyntax::parse(input.text.as_ref()) else {
        return Vec::new();
    };
    let mut diagnostics = Vec::new();
    let mut offset = 0;
    while let Some(relative) = input.text[offset..].find("echo ") {
        let start = offset + relative + 5;
        let Some(node) = syntax
            .tree()
            .root_node()
            .descendant_for_byte_range(start, start + 1)
        else {
            break;
        };
        if node.kind() == "comment" || node.kind().contains("string") {
            offset = start + 1;
            continue;
        }
        let end = start
            + input.text[start..]
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .map(char::len_utf8)
                .sum::<usize>();
        let name = &input.text[start..end];
        if !name.is_empty()
            && name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            && !BUILT_INS.contains(&name)
        {
            let declared = input.known_constants.iter().any(|(symbol_name, fqn)| {
                symbol_name == name || fqn.ends_with(&format!("\\{name}"))
            });
            let runtime = input
                .runtime_symbols
                .as_ref()
                .is_some_and(|symbols| symbols.find_constant(name).is_some());
            if !declared && !runtime {
                diagnostics.push(ByteDiagnostic {
                    range: start..end,
                    severity: Some(DiagnosticSeverity::WARNING),
                    message: format!("Undefined constant '{name}'"),
                });
            }
        }
        offset = end.max(start + 1);
        if offset >= input.text.len() {
            break;
        }
    }
    diagnostics
}

#[derive(Clone)]
struct ArgumentInspectionInput {
    text: Arc<str>,
    project_symbols: Vec<axiom_index::ProjectSymbol>,
    runtime_symbols: Option<Arc<RuntimeSymbolIndex>>,
    semantic_snapshot: Option<Arc<SemanticSnapshot>>,
    file_key: PersistentFileKey,
}

fn compute_argument_inspections(input: &ArgumentInspectionInput) -> Vec<ByteDiagnostic> {
    let Ok(syntax) = PhpSyntax::parse(input.text.as_ref()) else {
        return Vec::new();
    };
    let mut pending = vec![syntax.tree().root_node()];
    let mut calls = Vec::new();
    while let Some(node) = pending.pop() {
        if matches!(
            node.kind(),
            "function_call_expression"
                | "member_call_expression"
                | "nullsafe_member_call_expression"
                | "static_call_expression"
                | "scoped_call_expression"
                | "object_creation_expression"
        ) {
            calls.push(node);
        }
        pending.extend(node.named_children(&mut node.walk()));
    }
    calls.sort_by_key(|node| node.start_byte());

    let mut out = Vec::new();
    for call in calls {
        let arguments_node = call.child_by_field_name("arguments").or_else(|| {
            call.named_children(&mut call.walk())
                .find(|node| node.kind() == "arguments")
        });
        let Some(arguments_node) = arguments_node else {
            continue;
        };
        let open = arguments_node.start_byte();
        if call.kind() == "object_creation_expression" {
            let class_node = call.child_by_field_name("class").or_else(|| {
                call.named_children(&mut call.walk())
                    .find(|node| matches!(node.kind(), "name" | "qualified_name"))
            });
            let Some(class_node) = class_node else {
                continue;
            };
            let class_text = class_node
                .utf8_text(input.text.as_bytes())
                .unwrap_or("")
                .trim();
            let Some(snapshot) = input.semantic_snapshot.as_ref() else {
                continue;
            };
            let Some(scope) = snapshot.scope_id_at(&input.file_key, call.start_byte()) else {
                continue;
            };
            let written = class_text;
            let Some(resolved) = snapshot.resolve_class_name(scope, written) else {
                continue;
            };
            let receiver = DeclaredType::Named {
                written: written.to_owned(),
                resolved: resolved.clone(),
            };
            let resolution = snapshot.member_resolver().resolve_method(
                scope,
                &receiver,
                "__construct",
                MemberAccess::Instance,
            );
            let MemberResolution::Resolved(id) = resolution else {
                // Missing, inaccessible, or incompatible constructors are
                // handled by their respective semantic policies, not by an
                // argument-count diagnostic.
                continue;
            };
            let Some(parameters) = snapshot
                .symbol(id)
                .and_then(|symbol| symbol.parameters.as_deref())
            else {
                continue;
            };
            let arity = signature_counts_from_detail(parameters);
            let arguments = arguments_node.named_child_count() as usize;
            if arguments < arity.required_count
                || (!arity.variadic && arguments > arity.maximum_count)
            {
                out.push(ByteDiagnostic {
                    range: arguments_node.start_byte()..arguments_node.end_byte(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: if arguments > arity.maximum_count && !arity.variadic {
                        format!(
                            "Expected at most {} arguments, found {arguments}",
                            arity.maximum_count
                        )
                    } else {
                        format!(
                            "Expected {} argument{}, found {arguments}",
                            arity.required_count,
                            if arity.required_count == 1 { "" } else { "s" }
                        )
                    },
                });
            }
            emit_project_type_diagnostics(input, snapshot, scope, id, arguments_node, &mut out);
            continue;
        }
        let callable_start = input.text[..open]
            .char_indices()
            .rev()
            .take_while(|(_, ch)| {
                ch.is_alphanumeric() || matches!(ch, '_' | '$' | '\\' | ':' | '-' | '>')
            })
            .last()
            .map_or(open, |(i, _)| i);
        let callable = input.text[callable_start..open].trim();
        let name = callable
            .rsplit_once("::")
            .or_else(|| callable.rsplit_once("->"))
            .map(|(_, n)| n)
            .unwrap_or(callable)
            .trim_start_matches('$');
        if !name.is_empty() {
            let owner = callable
                .rsplit_once("::")
                .or_else(|| callable.rsplit_once("->"))
                .map(|(o, _)| {
                    let owner = o.trim();
                    if owner.starts_with('$') {
                        let pattern = format!("{} ", owner);
                        input.text[..open]
                            .rfind(&pattern)
                            .and_then(|pos| {
                                let before = &input.text[..pos];
                                before
                                    .rsplit(|ch: char| {
                                        !(ch.is_alphanumeric()
                                            || matches!(ch, '_' | '\\' | '?' | '|'))
                                    })
                                    .find(|part| !part.is_empty())
                                    .map(|ty| {
                                        resolve_php_class_name(
                                            ty.trim_start_matches('?'),
                                            &input.text,
                                        )
                                    })
                            })
                            .unwrap_or_else(|| resolve_php_class_name(owner, &input.text))
                    } else {
                        resolve_php_class_name(owner, &input.text)
                    }
                });
            let semantic_method = if matches!(
                call.kind(),
                "member_call_expression" | "nullsafe_member_call_expression"
            ) {
                let receiver = call
                    .child_by_field_name("object")
                    .filter(|node| node.kind() == "variable_name")
                    .and_then(|node| node.utf8_text(input.text.as_bytes()).ok())
                    .map(str::trim)
                    .map(str::to_owned);
                input.semantic_snapshot.as_ref().and_then(|snapshot| {
                    let Some(scope) = snapshot.scope_id_at(&input.file_key, call.start_byte())
                    else {
                        return None;
                    };
                    let Some(receiver) = receiver.as_deref() else {
                        return None;
                    };
                    let result = match snapshot
                        .member_resolver()
                        .resolve_binding_method(scope, receiver, name)
                    {
                        MemberResolution::Resolved(id) => Some(id),
                        _ => None,
                    };
                    result
                })
            } else {
                None
            };
            let symbol = if semantic_method.is_none() {
                owner
                    .as_deref()
                    .and_then(|o| {
                        input.project_symbols.iter().find(|s| {
                            s.kind == ProjectSymbolKind::Method
                                && s.fully_qualified_name.starts_with(&format!("{o}::"))
                                && s.name == name
                        })
                    })
                    .or_else(|| {
                        owner
                            .is_none()
                            .then(|| {
                                input.project_symbols.iter().find(|s| {
                                    s.kind == ProjectSymbolKind::Function && s.name == name
                                })
                            })
                            .flatten()
                    })
            } else {
                None
            };
            let detail = semantic_method
                .and_then(|id| input.semantic_snapshot.as_ref()?.symbol(id))
                .and_then(|s| s.parameters.as_deref())
                .map(signature_counts_from_detail)
                .or_else(|| {
                    symbol
                        .and_then(|s| s.parameters.as_deref())
                        .map(signature_counts_from_detail)
                });
            let runtime_detail = input.runtime_symbols.as_ref().and_then(|r| {
                owner
                    .as_deref()
                    .and_then(|o| r.methods_of(o).find(|s| s.name == name))
                    .and_then(|s| s.signature.as_ref())
                    .map(|s| ParameterArity {
                        required_count: s
                            .parameters
                            .iter()
                            .filter(|p| !p.optional && !p.variadic)
                            .count(),
                        maximum_count: s.parameters.len(),
                        variadic: s.parameters.iter().any(|p| p.variadic),
                    })
            });
            let counts = if semantic_method.is_some() {
                detail
            } else {
                runtime_detail.or(detail)
            };
            if let Some(arity) = counts {
                let required = arity.required_count;
                let maximum = arity.maximum_count;
                let variadic = arity.variadic;
                let arguments = arguments_node.named_child_count() as usize;
                if arguments < required || (!variadic && arguments > maximum) {
                    out.push(ByteDiagnostic {
                        range: arguments_node.start_byte()..arguments_node.end_byte(),
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: if arguments > maximum && !variadic {
                            format!("Expected at most {maximum} arguments, found {arguments}")
                        } else {
                            format!(
                                "Expected {required} argument{}, found {arguments}",
                                if required == 1 { "" } else { "s" }
                            )
                        },
                    });
                }
            }
            if let (Some(id), Some(snapshot)) = (semantic_method, input.semantic_snapshot.as_ref())
            {
                if let Some(scope) = snapshot.scope_id_at(&input.file_key, call.start_byte()) {
                    emit_project_type_diagnostics(
                        input,
                        snapshot,
                        scope,
                        id,
                        arguments_node,
                        &mut out,
                    );
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct DiagnosticStore {
    native_syntax: Vec<ByteDiagnostic>,
    native_inspections: Vec<ByteDiagnostic>,
    lsp: Vec<ByteDiagnostic>,
    combined_cache: Arc<Vec<ByteDiagnostic>>,
}

impl DiagnosticStore {
    fn rebuild(&mut self) {
        let mut diagnostics = self
            .native_syntax
            .iter()
            .chain(self.native_inspections.iter())
            .chain(self.lsp.iter())
            .cloned()
            .collect::<Vec<_>>();
        diagnostics.sort_by(|a, b| {
            a.range
                .start
                .cmp(&b.range.start)
                .then_with(|| a.range.end.cmp(&b.range.end))
                .then_with(|| {
                    diagnostic_severity_rank(a.severity).cmp(&diagnostic_severity_rank(b.severity))
                })
                .then_with(|| a.message.cmp(&b.message))
        });
        self.combined_cache = Arc::new(diagnostics);
    }

    fn set_native_syntax(&mut self, diagnostics: Vec<ByteDiagnostic>) {
        self.native_syntax = diagnostics;
        self.rebuild();
    }

    fn set_native_inspections(&mut self, diagnostics: Vec<ByteDiagnostic>) {
        self.native_inspections = diagnostics;
        self.rebuild();
    }

    fn set_lsp(&mut self, diagnostics: Vec<ByteDiagnostic>) {
        self.lsp = diagnostics;
        self.rebuild();
    }

    fn combined(&self) -> Arc<Vec<ByteDiagnostic>> {
        self.combined_cache.clone()
    }
}

fn diagnostic_severity_rank(severity: Option<DiagnosticSeverity>) -> u8 {
    match severity {
        Some(DiagnosticSeverity::ERROR) => 0,
        Some(DiagnosticSeverity::WARNING) => 1,
        Some(DiagnosticSeverity::INFORMATION) => 2,
        Some(DiagnosticSeverity::HINT) => 3,
        Some(_) => 4,
        None => 4,
    }
}

struct IndexUpdateRequest {
    generation: u64,
    semantic_generation: u64,
    path: PathBuf,
    text: String,
    index: Arc<std::sync::RwLock<ProjectSymbolIndex>>,
    revision: Arc<AtomicU64>,
    semantic_updates: Option<Sender<(u64, PathBuf, String)>>,
}

#[derive(Clone)]
pub struct VendorDefinitionRequest {
    pub index: Arc<std::sync::RwLock<VendorSymbolIndex>>,
    pub fqn: String,
    pub member: Option<String>,
    pub is_static: bool,
    pub chain: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum DefinitionQuery {
    Name {
        fqn: String,
    },
    Method {
        owner_fqn: String,
        name: String,
        is_static: bool,
        chain: Vec<String>,
    },
    Function,
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
            let path = request.path.clone();
            let text = request.text.clone();
            let result =
                index.index_file_text_with_source(request.path, request.text, "EditorDirtyUpdate");
            if result.is_ok() {
                if let Some(sender) = request.semantic_updates {
                    let _ = sender.send((request.semantic_generation, path, text));
                }
            }
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
        let initial_line_count = document.line_count();
        let document_session = NEXT_DOCUMENT_SESSION.fetch_add(1, Ordering::Relaxed);
        let (native_inspection_sender, native_inspection_results) = mpsc::channel();
        if let (Some(lsp), Some(uri)) = (&lsp, &lsp_uri) {
            lsp.register_document_session(uri.clone(), document_session);
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
            document_session,
            last_lsp_text,
            completions: Vec::new(),
            completion_selected: 0,
            hover_popup: None,
            hover_anchor: None,
            diagnostics: DiagnosticStore::default(),
            context_menu: None,
            ctrl_hover_range: None,
            line_layouts: RefCell::new(HashMap::new()),
            runtime_symbols: None,
            project_symbols: None,
            vendor_symbols: None,
            semantic_engine: None,
            project_index_revision: None,
            index_update_sender: Some(index_update_sender),
            semantic_update_sender: None,
            semantic_update_generation: 0,
            workspace_root: None,
            workspace_source: false,
            editor_scroll_hovered: false,
            editor_scroll_drag_axis: None,
            editor_scroll_drag_start: Point::default(),
            editor_scroll_drag_start_offset: Point::default(),
            content_width: px(0.),
            line_width_cache: HashMap::new(),
            max_width_line: None,
            width_cache_line_count: initial_line_count,
            width_cache_dirty: true,
            native_inspection_generation: 0,
            native_inspection_latest_generation: Arc::new(AtomicU64::new(0)),
            initial_native_inspection_scheduled: false,
            native_inspection_sender,
            native_inspection_results,
            caret_visible: true,
            caret_blink_generation: 0,
            caret_last_activity: Instant::now(),
            caret_last_toggle: Instant::now(),
            caret_blink_task_active: false,
            edit_generation: 0,
            last_mutation_at: None,
            last_rendered_edit_generation: 0,
            last_render_at: None,
            last_notify_at: None,
        };
        let inspections_started = std::time::Instant::now();
        view.sync_syntax();
        let native_inspections_us = inspections_started.elapsed().as_micros();
        if debug_input_enabled() {
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

    pub fn definition_syntax_context(&self, offset: usize) -> Option<DefinitionSyntaxContext> {
        let started = std::time::Instant::now();
        let syntax = self.syntax.as_ref()?;
        let token = syntax.token_at_byte(offset)?;
        let context = DefinitionSyntaxContext {
            token,
            is_keyword: syntax.is_keyword_at_byte(offset),
            tree: syntax.tree().clone(),
            build_us: started.elapsed().as_micros(),
        };
        Some(context)
    }

    pub fn current_cursor_offset(&self) -> usize {
        self.document.cursor_offset()
    }

    pub fn is_dirty(&self) -> bool {
        self.document.is_dirty()
    }

    pub fn document_session(&self) -> DocumentSessionId {
        self.document_session
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
        // A member query is valid only when the token immediately follows the
        // member operator.  Looking for any earlier `->` misclassified the
        // RHS of expressions such as `$this->loop = $loop ?? new FiberEventLoop()`
        // as a method on `$this`.
        let member_operator = before
            .trim_end()
            .strip_suffix("->")
            .map(|_| ("->", false))
            .or_else(|| before.trim_end().strip_suffix("::").map(|_| ("::", true)));
        if let Some((operator_text, is_static)) = member_operator {
            let operator = before.trim_end().len() - operator_text.len();
            let owner_end = operator;
            let (_, written_owner) = extract_owner_expression(before, owner_end);
            let mut chain = Vec::new();
            let owner_fqn = if written_owner.starts_with('$') {
                if let Some((base, methods)) = split_method_chain(&written_owner) {
                    let base_fqn = self.resolve_native_type(base, before)?;
                    chain = methods;
                    base_fqn
                } else {
                    self.resolve_receiver_type(&written_owner, before)?
                }
            } else {
                resolve_php_class_name(&written_owner, &text)
            };
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                tracing::info!(token = %token.text, kind = "Method", written = %written_owner, resolved = %owner_fqn, via = "receiver-type", "[DEFINITION QUERY]");
            }
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                tracing::info!(branch = "Method", "[DEFINITION QUERY BRANCH]");
            }
            return Some(DefinitionQuery::Method {
                owner_fqn,
                name: token.text.trim_start_matches('$').to_owned(),
                is_static,
                chain,
            });
        }
        let name = token.text.trim_start_matches('$').to_owned();
        let is_new = text[..token.range.start].trim_end().ends_with("new");
        if !is_new && text[token.range.end..].starts_with('(') {
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                tracing::info!(branch = "Function", "[DEFINITION QUERY BRANCH]");
            }
            return Some(DefinitionQuery::Function);
        }
        let fqn = resolve_php_class_name(&name, &text);
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
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
        Some(DefinitionQuery::Name { fqn })
    }

    pub fn vendor_definition_request(&self) -> Option<VendorDefinitionRequest> {
        let query = self.definition_query()?;
        let (fqn, member, is_static, chain) = match query {
            DefinitionQuery::Method {
                owner_fqn,
                name,
                is_static,
                chain,
                ..
            } => (owner_fqn, Some(name), is_static, chain),
            DefinitionQuery::Name { fqn } => (fqn, None, false, Vec::new()),
            DefinitionQuery::Function => return None,
        };
        let index = self.vendor_symbols.clone()?;
        // Vendor metadata and dependency files are resolved by the background
        // worker. Never perform UNC filesystem probes on the UI thread.
        Some(VendorDefinitionRequest {
            index,
            fqn,
            member,
            is_static,
            chain,
        })
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

    pub fn set_project_symbols(
        &mut self,
        symbols: Arc<std::sync::RwLock<ProjectSymbolIndex>>,
        cx: &mut Context<Self>,
    ) {
        self.project_symbols = Some(symbols);
        self.project_index_revision = Some(Arc::new(AtomicU64::new(0)));
        self.sync_syntax();
        self.maybe_schedule_initial_native_inspections(cx);
    }

    pub fn set_semantic_update_sender(
        &mut self,
        sender: Sender<(u64, PathBuf, String)>,
        generation: u64,
    ) {
        if self.workspace_source {
            self.semantic_update_sender = Some(sender);
            self.semantic_update_generation = generation;
        } else {
            self.semantic_update_sender = None;
        }
    }

    pub fn set_workspace_root(&mut self, root: PathBuf, workspace_source: bool) {
        self.workspace_source = workspace_source;
        self.workspace_root = Some(root);
    }

    pub fn set_vendor_symbols(&mut self, symbols: Arc<std::sync::RwLock<VendorSymbolIndex>>) {
        self.vendor_symbols = Some(symbols);
        self.sync_syntax();
    }

    pub fn set_semantic_engine(&mut self, engine: Arc<SemanticEngine>, cx: &mut Context<Self>) {
        self.semantic_engine = Some(engine);
        self.maybe_schedule_initial_native_inspections(cx);
    }

    fn maybe_schedule_initial_native_inspections(&mut self, cx: &mut Context<Self>) {
        if self.initial_native_inspection_scheduled || self.project_symbols.is_none() {
            return;
        }
        let index_ready = self
            .project_symbols
            .as_ref()
            .and_then(|index| index.try_read().ok())
            .is_some_and(|index| index.is_ready());
        if !index_ready {
            return;
        }
        self.initial_native_inspection_scheduled = true;
        self.schedule_native_inspections("", cx);
    }

    pub fn close_lsp_document(&self) {
        if let (Some(lsp), Some(uri)) = (&self.lsp, &self.lsp_uri) {
            lsp.invalidate_document_session(uri, self.document_session);
        }
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
        self.reset_caret_blink(cx);
        self.document.move_cursor(offset);
        self.selection_anchor = None;
        self.preferred_x = None;
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn reset_caret_blink(&mut self, cx: &mut Context<Self>) {
        self.caret_visible = true;
        self.caret_blink_generation = self.caret_blink_generation.wrapping_add(1);
        self.caret_last_activity = Instant::now();
        self.caret_last_toggle = Instant::now();
        if self.caret_blink_task_active {
            cx.notify();
            return;
        }
        self.caret_blink_task_active = true;
        CARET_TASKS_STARTED.fetch_add(1, Ordering::Relaxed);
        CARET_TASKS_ACTIVE.fetch_add(1, Ordering::Relaxed);
        cx.spawn(async move |this, cx| {
            for _ in 0..240 {
                gpui::Timer::after(Duration::from_millis(100)).await;
                let alive = this
                    .update(cx, |editor, cx| {
                        let now = Instant::now();
                        if now.duration_since(editor.caret_last_activity)
                            >= Duration::from_millis(500)
                            && now.duration_since(editor.caret_last_toggle)
                                >= Duration::from_millis(500)
                        {
                            editor.caret_visible = !editor.caret_visible;
                            editor.caret_last_toggle = now;
                            cx.notify();
                        }
                        true
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
            let _ = this.update(cx, |editor, _| editor.caret_blink_task_active = false);
            CARET_TASKS_ACTIVE.fetch_sub(1, Ordering::Relaxed);
        })
        .detach();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.reset_caret_blink(cx);
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
                self.line_width_cache.clear();
                self.max_width_line = None;
                self.width_cache_line_count = self.document.line_count();
                self.width_cache_dirty = true;
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
        let _stage = UiStageGuard::new(UI_STAGE_AFTER_EDIT);
        let edit_started = Instant::now();
        self.edit_generation = self.edit_generation.wrapping_add(1);
        LAST_UI_EDIT_GENERATION.store(self.edit_generation, Ordering::Relaxed);
        self.last_mutation_at = Some(edit_started);
        self.reset_caret_blink(cx);
        let edited_line = self.document.line_of_offset(self.document.cursor_offset());
        self.line_width_cache.remove(&edited_line);
        if self.max_width_line == Some(edited_line) {
            self.width_cache_dirty = true;
        }
        let text = self.document.content();
        let edit = self.document.take_last_edit();
        let syntax_started = Instant::now();
        self.sync_syntax_text(&text, edit.as_ref());
        let syntax_us = syntax_started.elapsed().as_micros();
        let inspection_started = Instant::now();
        self.schedule_native_inspections(&text, cx);
        let inspection_schedule_us = inspection_started.elapsed().as_micros();
        let lsp_started = Instant::now();
        self.sync_lsp_text(&text, edit.as_ref());
        let lsp_queue_us = lsp_started.elapsed().as_micros();
        let semantic_started = Instant::now();
        self.schedule_incremental_index_update(&text);
        let semantic_schedule_us = semantic_started.elapsed().as_micros();
        let completion_started = Instant::now();
        let clear_started = Instant::now();
        let (trigger_us, native_total_us, native_empty) = if !self.is_php_completion_context() {
            // Keep the rest of the edit pipeline (syntax, indexing, LSP
            // text synchronization, cursor and rendering) active for
            // generic files, but never enter the PHP completion pipeline.
            self.completions.clear();
            (0_u128, 0_u128, true)
        } else {
            let trigger_started = Instant::now();
            self.maybe_trigger_completion(&text);
            let trigger_us = trigger_started.elapsed().as_micros();
            let cursor = self.document.cursor_offset();
            if cursor > 0 && matches!(text[..cursor].chars().next_back(), Some('(' | ',')) {
                self.hover_popup = self.native_signature_help_for_text(&text);
                self.hover_anchor = None;
            }
            let native_started = Instant::now();
            let native = self.native_completions_for_text(&text);
            let native_total_us = native_started.elapsed().as_micros();
            let native_empty = native.items.is_empty();
            if native_empty {
                self.completions.clear();
            } else {
                let set_started = Instant::now();
                if let Some(prefix_range) = native.new_prefix {
                    let prefix = &text[prefix_range];
                    let mut items = native.items;
                    filter_new_completion_items(&mut items, prefix);
                    rank_new_completion_items(&mut items, prefix);
                    self.completions = items;
                    self.completion_selected = 0;
                    cx.notify();
                } else {
                    self.set_completions(native.items, cx);
                }
                let set_completions_us = set_started.elapsed().as_micros();
                let completion_total_us = completion_started.elapsed().as_micros();
                if debug_ui_stall_enabled() && completion_total_us >= 3_000 {
                    tracing::info!(target: "axiom.ui_stall",
                        trigger_us,
                        native_total_us,
                        project_search_us = 0_u128,
                        vendor_search_us = 0_u128,
                        runtime_search_us = 0_u128,
                        member_resolution_us = 0_u128,
                        import_edits_us = 0_u128,
                        set_completions_us,
                        clear_completions_us = clear_started.elapsed().as_micros(),
                        other_us = completion_total_us.saturating_sub(
                            trigger_us + native_total_us + set_completions_us,
                        ),
                        total_us = completion_total_us,
                        "[UI COMPLETION DETAIL]"
                    );
                }
            }
            (trigger_us, native_total_us, native_empty)
        };
        let completion_us = completion_started.elapsed().as_micros();
        if native_empty {
            if debug_ui_stall_enabled() && completion_us >= 3_000 {
                tracing::info!(target: "axiom.ui_stall",
                    trigger_us,
                    native_total_us,
                    project_search_us = 0_u128,
                    vendor_search_us = 0_u128,
                    runtime_search_us = 0_u128,
                    member_resolution_us = 0_u128,
                    import_edits_us = 0_u128,
                    set_completions_us = 0_u128,
                    clear_completions_us = clear_started.elapsed().as_micros(),
                    other_us = completion_us.saturating_sub(trigger_us + native_total_us),
                    total_us = completion_us,
                    "[UI COMPLETION DETAIL]"
                );
            }
        }
        self.selection_anchor = None;
        self.preferred_x = None;
        self.marked_range = None;
        self.ctrl_hover_range = None;
        self.ensure_cursor_visible();
        let notify_started = Instant::now();
        cx.notify();
        self.last_notify_at = Some((self.edit_generation, Instant::now()));
        if debug_ui_stall_enabled() && edit_started.elapsed().as_micros() >= 5_000 {
            let total_us = edit_started.elapsed().as_micros();
            let accounted = syntax_us
                + inspection_schedule_us
                + lsp_queue_us
                + semantic_schedule_us
                + completion_us
                + notify_started.elapsed().as_micros();
            tracing::info!(target: "axiom.ui_stall",
                total_us,
                syntax_us,
                inspection_schedule_us,
                lsp_queue_us,
                semantic_schedule_us,
                completion_us,
                notify_us = notify_started.elapsed().as_micros(),
                other_us = total_us.saturating_sub(accounted),
                "[UI AFTER EDIT]"
            );
        }
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
        let Some(_root) = &self.workspace_root else {
            return;
        };
        if !self.workspace_source {
            if debug_input_enabled() {
                tracing::debug!(path = %self.file_path.display(), "[SEMANTIC UPDATE REJECTED] reason=non_workspace_source");
            }
            return;
        }
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
                semantic_generation: self.semantic_update_generation,
                semantic_updates: self.semantic_update_sender.clone(),
            });
        }
    }

    fn schedule_native_inspections(&mut self, _: &str, cx: &mut Context<Self>) {
        self.native_inspection_generation = self.native_inspection_generation.wrapping_add(1);
        let generation = self.native_inspection_generation;
        self.native_inspection_latest_generation
            .store(generation, Ordering::Release);
        let session = self.document_session;
        let sender = self.native_inspection_sender.clone();
        cx.spawn(async move |this, cx| {
            gpui::Timer::after(Duration::from_millis(200)).await;
            let mut lock_retries = 0usize;
            let work = loop {
                let capture = match this.update(cx, |editor, _| {
                    if editor.document_session != session
                        || editor.native_inspection_generation != generation
                    {
                        return None;
                    }
                    let text: Arc<str> = Arc::from(editor.document.content());
                    let work = editor.capture_native_inspection_work(text, session, generation);
                    Some(work)
                }) {
                    Ok(Some(capture)) => capture,
                    Ok(None) | Err(_) => return,
                };
                match capture {
                    NativeInspectionCapture::Ready(work) => break work,
                    NativeInspectionCapture::Unavailable => return,
                    NativeInspectionCapture::RetryLockBusy => {
                        if lock_retries >= NATIVE_INSPECTION_CAPTURE_MAX_LOCK_RETRIES {
                            return;
                        }
                        lock_retries += 1;
                        gpui::Timer::after(Duration::from_millis(
                            NATIVE_INSPECTION_CAPTURE_RETRY_DELAY_MS,
                        ))
                        .await;
                    }
                }
            };
            std::thread::spawn(move || {
                let Some(diagnostics) = run_native_inspection_rules(
                    &work.latest_generation,
                    work.generation,
                    || compute_unknown_class_inspections(&work.unknown_class),
                    || compute_unknown_constant_inspections(&work.unknown_constant),
                    || compute_duplicate_class_inspections(&work.duplicate_class),
                    || compute_argument_inspections(&work.arguments),
                ) else {
                    return;
                };
                let _ = sender.send(NativeInspectionResult {
                    document_session: work.document_session,
                    generation: work.generation,
                    diagnostics,
                });
            });
            for _ in 0..600 {
                gpui::Timer::after(Duration::from_millis(16)).await;
                if this
                    .update(cx, |editor, cx| editor.poll_native_inspection_results(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn capture_native_inspection_work(
        &self,
        text: Arc<str>,
        session: DocumentSessionId,
        generation: u64,
    ) -> NativeInspectionCapture {
        let Some(project_symbols) = self.project_symbols.as_ref() else {
            return NativeInspectionCapture::Unavailable;
        };
        let index = match project_symbols.try_read() {
            Ok(index) => index,
            Err(_) => return NativeInspectionCapture::RetryLockBusy,
        };
        if !index.is_ready() {
            return NativeInspectionCapture::Unavailable;
        }
        let symbols = index.symbols().to_vec();
        let project_classes = symbols
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    ProjectSymbolKind::Class
                        | ProjectSymbolKind::Interface
                        | ProjectSymbolKind::Trait
                        | ProjectSymbolKind::Enum
                )
            })
            .map(|s| s.fully_qualified_name.clone())
            .collect();
        let known_constants = symbols
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    ProjectSymbolKind::Constant | ProjectSymbolKind::ClassConstant
                )
            })
            .map(|s| (s.name.clone(), s.fully_qualified_name.clone()))
            .collect();
        let declarations = symbols
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    ProjectSymbolKind::Class
                        | ProjectSymbolKind::Interface
                        | ProjectSymbolKind::Trait
                        | ProjectSymbolKind::Enum
                )
            })
            .map(|s| DuplicateClassDeclaration {
                fqn: s.fully_qualified_name.clone(),
                // ProjectSymbolIndex stores discovered files in canonical form.
                // The lexical key is therefore sufficient here and avoids a
                // filesystem call in the debounced UI capture callback.
                file: PersistentFileKey::workspace_lexical(&s.file),
                range: s.range.clone(),
            })
            .collect();
        let vendor_symbols = self
            .vendor_symbols
            .as_ref()
            .and_then(|v| v.try_read().ok())
            .map(|v| Arc::new(v.clone()));
        let semantic_snapshot = self
            .semantic_engine
            .as_ref()
            .map(|engine| engine.snapshot());
        let file_key = PersistentFileKey::workspace_lexical(&self.file_path);
        NativeInspectionCapture::Ready(NativeInspectionWorkItem {
            document_session: session,
            generation,
            latest_generation: self.native_inspection_latest_generation.clone(),
            unknown_class: UnknownClassInspectionInput {
                text: text.clone(),
                project_classes,
                runtime_symbols: self.runtime_symbols.clone(),
                vendor_symbols,
            },
            unknown_constant: UnknownConstantInspectionInput {
                text: text.clone(),
                known_constants,
                runtime_symbols: self.runtime_symbols.clone(),
            },
            duplicate_class: DuplicateClassInspectionInput {
                // The editor only schedules this work for a workspace source;
                // avoid canonicalization/filesystem access on the UI callback.
                path: PersistentFileKey::workspace_lexical(&self.file_path),
                declarations,
            },
            arguments: ArgumentInspectionInput {
                text,
                project_symbols: symbols,
                runtime_symbols: self.runtime_symbols.clone(),
                semantic_snapshot,
                file_key,
            },
        })
    }

    fn poll_native_inspection_results(&mut self, cx: &mut Context<Self>) {
        let mut latest = None;
        while let Ok(result) = self.native_inspection_results.try_recv() {
            latest = Some(result);
        }
        if let Some(result) = latest {
            if result.document_session == self.document_session
                && result.generation == self.native_inspection_generation
            {
                self.diagnostics.set_native_inspections(result.diagnostics);
                cx.notify();
            }
        }
    }

    fn maybe_trigger_completion(&self, text: &str) {
        let (Some(lsp), Some(uri)) = (self.lsp.as_ref(), self.lsp_uri.as_ref()) else {
            return;
        };
        let tail = &text[..self.document.cursor_offset().min(text.len())];
        if tail.ends_with("->")
            || tail.ends_with("::")
            || tail.ends_with("new ")
            || tail.ends_with("extends ")
            || tail.ends_with("implements ")
            || tail.ends_with("use ")
        {
            let position = PositionCodec::offset_to_position(
                text,
                self.document.cursor_offset(),
                lsp.encoding(),
            );
            lsp.request_completion(uri.clone(), position);
        }
    }

    /// Returns whether the current document can enter the PHP completion
    /// pipeline. This is intentionally O(1) and metadata-only: no text
    /// snapshot, parsing, filesystem access, or provider lookup is involved.
    fn is_php_completion_context(&self) -> bool {
        is_php_file(&self.file_path)
    }

    fn sync_syntax(&mut self) {
        let text = self.document.content();
        self.sync_syntax_text(&text, None);
    }

    fn sync_syntax_text(&mut self, text: &str, edit: Option<&DocumentEdit>) {
        if let Some(syntax) = &mut self.syntax {
            let result = match edit {
                Some(edit) => match syntax
                    .apply_edit_profiled(edit.old_range_bytes.clone(), &edit.inserted_text)
                {
                    Ok(profile) if syntax.text() == text => Ok(profile),
                    // A stale or non-representable delta must not leave the
                    // resident tree out of sync; fall back to the complete
                    // document update in that rare path.
                    Ok(_) | Err(_) => syntax.update_text_profiled(text),
                },
                None => syntax.update_text_profiled(text),
            };
            match result {
                Ok(_) => {}
                Err(error) => {
                    self.status = Some(format!("Falha ao atualizar sintaxe PHP: {error}").into())
                }
            }
        }
        let diagnostics = self
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
        self.diagnostics.set_native_syntax(diagnostics);
    }

    fn sync_lsp(&mut self) {
        let text = self.document.content();
        self.sync_lsp_text(&text, None);
    }

    fn sync_lsp_text(&mut self, text: &str, edit: Option<&DocumentEdit>) {
        if text == self.last_lsp_text {
            return;
        }
        self.last_lsp_text = text.to_owned();
        self.lsp_version = self.lsp_version.saturating_add(1);
        if let (Some(lsp), Some(uri)) = (&self.lsp, &self.lsp_uri) {
            let encoding = lsp.encoding();
            let encode = |line: &str, column: usize| match encoding {
                PositionEncoding::Utf8 => column,
                PositionEncoding::Utf16 => utf8_column_to_utf16(line, column),
                PositionEncoding::Utf32 => line
                    .get(..column.min(line.len()))
                    .map(|prefix| prefix.chars().count())
                    .unwrap_or_else(|| line.chars().count()),
            };
            let range = edit.and_then(|edit| {
                Some(lsp_types::Range::new(
                    lsp_types::Position::new(
                        edit.old_start_line as u32,
                        encode(&edit.old_start_line_text, edit.old_start_column_bytes) as u32,
                    ),
                    lsp_types::Position::new(
                        edit.old_end_line as u32,
                        encode(&edit.old_end_line_text, edit.old_end_column_bytes) as u32,
                    ),
                ))
            });
            lsp.queue_did_change_range(
                self.document_session,
                uri.clone(),
                self.lsp_version,
                range,
                edit.map(|edit| edit.inserted_text.clone())
                    .unwrap_or_else(|| text.to_owned()),
            );
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
        // Keep manual completion on the same cheap, metadata-only gate used
        // by the automatic after-edit path.
        if !self.is_php_completion_context() {
            self.completions.clear();
            cx.notify();
            return;
        }
        self.completions.clear();
        let native = self.native_completions();
        if let (Some(lsp), Some(uri), Some(position)) =
            (&self.lsp, &self.lsp_uri, self.lsp_position())
        {
            lsp.request_completion(uri.clone(), position);
        }
        if !native.items.is_empty() {
            self.set_completions(native.items, cx);
        } else if self.lsp.is_none() {
            self.status = Some("Completion unavailable (no PHP index or language server)".into());
            cx.notify();
        }
    }

    fn native_completions(&self) -> NativeCompletionBatch {
        let text = self.document.content();
        self.native_completions_for_text(&text)
    }

    fn native_completions_for_text(&self, text: &str) -> NativeCompletionBatch {
        let _stage = UiStageGuard::new(UI_STAGE_COMPLETION);
        let started = Instant::now();
        let result = self.native_completions_impl(text);
        if debug_ui_stall_enabled() && started.elapsed().as_micros() >= 3_000 {
            tracing::info!(target: "axiom.ui_stall",
                candidates = result.items.len(),
                total_us = started.elapsed().as_micros(),
                "[UI COMPLETION]"
            );
        }
        result
    }

    fn native_completions_impl(&self, text: &str) -> NativeCompletionBatch {
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
            return NativeCompletionBatch {
                items: self.local_variable_completions(&text[..cursor], prefix),
                new_prefix: None,
            };
        }
        let empty_prefix_context = before.ends_with("new ")
            || before.ends_with("extends ")
            || before.ends_with("implements ")
            || before.ends_with("use ");
        if prefix.is_empty() && member_operator.is_none() && !empty_prefix_context {
            return NativeCompletionBatch {
                items: Vec::new(),
                new_prefix: None,
            };
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
                return NativeCompletionBatch {
                    items: items.into_iter().take(40).collect(),
                    new_prefix: None,
                };
            }
        }
        if let Some((operator_start, is_static)) = member_operator {
            let owner_end = operator_start;
            let (_, owner_expression) = extract_owner_expression(&text, owner_end);
            let owner = owner_expression.trim_start_matches('$');

            // For direct instance-member completion, prefer the resident
            // semantic snapshot. It already knows the binding type and walks
            // inherited/trait owners without touching the filesystem or
            // scanning the project index. The legacy path below remains only
            // for contexts that cannot be represented by a snapshot binding
            // (for example an unresolved chained expression).
            if !is_static && let Some(engine) = &self.semantic_engine {
                let snapshot = engine.snapshot();
                let file_key = PersistentFileKey::workspace_lexical(&self.file_path);
                if let Some(scope) = snapshot.scope_id_at(&file_key, cursor)
                    && snapshot.lookup_binding(scope, &owner_expression).is_some()
                {
                    let ids = snapshot.member_resolver().completion_methods_for_binding(
                        scope,
                        &owner_expression,
                        prefix,
                    );
                    let members = ids
                        .into_iter()
                        .filter_map(|id| snapshot.symbol(id))
                        .map(|symbol| CompletionItem {
                            label: symbol.name.clone(),
                            detail: Some(format!(
                                "{}{}{} • Project",
                                symbol.name,
                                symbol.parameters.clone().unwrap_or_else(|| "()".to_owned()),
                                symbol
                                    .return_type
                                    .as_deref()
                                    .map(|value| format!(": {value}"))
                                    .unwrap_or_default()
                            )),
                            kind: Some(CompletionItemKind::METHOD),
                            ..Default::default()
                        })
                        .take(40)
                        .collect::<Vec<_>>();
                    return NativeCompletionBatch {
                        items: members,
                        new_prefix: None,
                    };
                }
            }

            if let Some(class_fqn) =
                self.resolve_receiver_type(&owner_expression, &text[..owner_end])
            {
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
                    members.extend(
                        index
                            .methods_of(&runtime_class_fqn)
                            .filter(|symbol| {
                                symbol.name.starts_with(prefix)
                                    && symbol.is_static == is_static
                                    && (is_static || !symbol.name.starts_with('_'))
                            })
                            .map(|symbol| CompletionItem {
                                label: symbol.name.clone(),
                                detail: Some(runtime_signature_detail(symbol)),
                                kind: Some(CompletionItemKind::METHOD),
                                insert_text: runtime_call_insert_text(symbol),
                                ..Default::default()
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
                return NativeCompletionBatch {
                    items: members.into_iter().take(40).collect(),
                    new_prefix: None,
                };
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
                        let import = matches!(
                            symbol.kind,
                            RuntimeKind::Class
                                | RuntimeKind::Interface
                                | RuntimeKind::Trait
                                | RuntimeKind::Enum
                        )
                        .then(|| self.composer_import_edit(&symbol.fqn))
                        .flatten();
                        CompletionItem {
                            label: symbol.name.clone(),
                            detail: Some(
                                if matches!(
                                    symbol.kind,
                                    RuntimeKind::Function | RuntimeKind::Method
                                ) {
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
        let completion_search_started = Instant::now();
        let mut completion_search_us = 0u128;
        let mut completion_import_us = 0u128;
        let mut completion_symbols_total = 0usize;
        let mut completion_matches = 0usize;
        let mut completion_index_unavailable = false;
        if let Some(index) = &self.project_symbols {
            if let Ok(index) = index.try_read() {
                completion_symbols_total = index.symbols().len();
                let search_started = Instant::now();
                let project_matches = index.search_prefix(prefix);
                completion_search_us = search_started.elapsed().as_micros();
                completion_matches = project_matches.len();
                items.extend(project_matches.into_iter().map(|symbol| {
                    let import_started = Instant::now();
                    let import = matches!(
                        symbol.kind,
                        ProjectSymbolKind::Class
                            | ProjectSymbolKind::Interface
                            | ProjectSymbolKind::Trait
                            | ProjectSymbolKind::Enum
                    )
                    .then(|| self.composer_import_edit(&symbol.fully_qualified_name))
                    .flatten();
                    completion_import_us += import_started.elapsed().as_micros();
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
            } else {
                completion_index_unavailable = true;
            }
        }
        let completion_inner_us = completion_search_started.elapsed().as_micros();
        if debug_ui_stall_enabled() && completion_index_unavailable {
            tracing::info!(target: "axiom.ui_stall", index_unavailable = true, "[UI COMPLETION SEARCH]");
        } else if debug_ui_stall_enabled() && completion_inner_us >= 3_000 {
            tracing::info!(target: "axiom.ui_stall",
                prefix_len = prefix.len(),
                symbols_total = completion_symbols_total,
                matches = completion_matches,
                search_prefix_us = completion_search_us,
                import_edits_us = completion_import_us,
                total_inner_us = completion_inner_us,
                "[UI COMPLETION SEARCH]"
            );
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
        if preceded_by_new {
            rank_new_completion_items(&mut items, prefix);
        }
        let mut seen = std::collections::HashSet::new();
        items.retain(|item| {
            seen.insert(format!(
                "{}:{}",
                item.label.to_ascii_lowercase(),
                item.detail.as_deref().unwrap_or_default()
            ))
        });
        NativeCompletionBatch {
            items: items.into_iter().take(40).collect(),
            new_prefix: preceded_by_new.then_some(start..cursor),
        }
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
            let _ = if base.trim() == "$this" {
                declared_class_fqn(context)
            } else {
                self.resolve_receiver_type(base, context)
                    .or_else(|| self.resolve_native_type(base, context))
            }?;
            let property_type = property_type_in_context(context, property.trim())?;
            let resolved = self.resolve_class_name(&property_type, context);
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
        self.native_signature_help_for_text(&text)
    }

    fn native_signature_help_for_text(&self, text: &str) -> Option<String> {
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
        self.diagnostics.set_lsp(
            diagnostics
                .into_iter()
                .map(|diagnostic| ByteDiagnostic {
                    range: self.lsp_range_to_bytes(diagnostic.range),
                    severity: diagnostic.severity,
                    message: diagnostic.message,
                })
                .collect(),
        );
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
        let scroll = self.scroll.0.borrow();
        let bounds = scroll.base_handle.bounds();
        let max = scroll.base_handle.max_offset();
        let vertical = max.height > px(0.);
        let horizontal = max.width > px(0.);
        let in_vertical = vertical
            && position.x >= bounds.right() - px(10.)
            && position.y <= bounds.bottom() - if horizontal { px(10.) } else { px(0.) };
        let in_horizontal = horizontal
            && position.y >= bounds.bottom() - px(10.)
            && position.x <= bounds.right() - if vertical { px(10.) } else { px(0.) };
        let in_corner = vertical
            && horizontal
            && position.x > bounds.right() - px(10.)
            && position.y > bounds.bottom() - px(10.);
        !in_vertical
            && !in_horizontal
            && !in_corner
            && position.x >= bounds.left() + px(GUTTER_WIDTH)
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
                "\n\n\n[DEFINITION MOUSE INPUT]"
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
                    "\n\n\n[EDITOR CTRL CLICK]"
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
                    "\n\n\n[DEFINITION CURSOR]"
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
        if self.editor_scroll_drag_axis.is_some() {
            return;
        }
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
                let combined_diagnostics = self.diagnostics.combined();
                let next = combined_diagnostics
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
        source: EditorScrollEventSource,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(axis) = self.editor_scroll_drag_axis else {
            return;
        };
        // The viewport is the single logical owner. The scrollbar callback
        // still emits the raw trace, but must not apply the same movement.
        if source == EditorScrollEventSource::Scrollbar {
            return;
        }
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
        let offset_before = handle.offset();
        let changed = next != offset_before;
        if changed {
            handle.set_offset(next);
            cx.notify();
        }
    }

    fn editor_scroll_drag_end(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(_axis) = self.editor_scroll_drag_axis else {
            return;
        };
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

    fn render_line(
        &self,
        line: usize,
        window: &mut Window,
        diagnostics: &[ByteDiagnostic],
    ) -> gpui::AnyElement {
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
        // Render at most one marker per line, choosing the most severe
        // diagnostic when several ranges overlap this line.
        let diagnostic = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.range.end > start && diagnostic.range.start < end)
            .min_by_key(|diagnostic| diagnostic_severity_rank(diagnostic.severity));
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
        } else if cursor_here && self.caret_visible && self.focus.is_focused(window) {
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
                let color = match severity {
                    Some(DiagnosticSeverity::ERROR) => t.error,
                    Some(DiagnosticSeverity::WARNING) => t.warning,
                    Some(DiagnosticSeverity::INFORMATION) => t.info,
                    Some(DiagnosticSeverity::HINT) => t.accent,
                    _ => t.warning,
                };
                let width: f32 = (to - from).max(px(1.)).into();
                let wave_step = 2.0_f32;
                let segment_count = ((width / wave_step).ceil() as usize).max(4);
                this.child(
                    div()
                        .absolute()
                        .left(px(GUTTER_WIDTH) + px(TEXT_PADDING) + from)
                        .top(px(LINE_HEIGHT - 4.))
                        .w(px(width))
                        .h(px(4.))
                        .overflow_hidden()
                        .children((0..segment_count).map(move |index| {
                            let phase = index % 4;
                            div()
                                .absolute()
                                .left(px(index as f32 * wave_step))
                                .top(match phase {
                                    0 | 2 => px(1.),
                                    1 => px(0.),
                                    _ => px(2.),
                                })
                                .w(px(wave_step + 1.))
                                .h(px(0.5))
                                .bg(color)
                        })),
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

struct NativeCompletionBatch {
    items: Vec<CompletionItem>,
    new_prefix: Option<std::ops::Range<usize>>,
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

fn split_method_chain(owner: &str) -> Option<(&str, Vec<String>)> {
    let mut parts = owner.split("->");
    let base = parts.next()?.trim();
    let methods: Vec<String> = parts
        .map(|part| part.trim().trim_end_matches("()"))
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    (!methods.is_empty()).then_some((base, methods))
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
        Some(CompletionItemKind::METHOD) => "M",
        Some(CompletionItemKind::FUNCTION) => "F",
        Some(CompletionItemKind::CLASS | CompletionItemKind::CONSTRUCTOR) => "C",
        Some(CompletionItemKind::INTERFACE) => "I",
        Some(CompletionItemKind::ENUM) => "E",
        Some(CompletionItemKind::STRUCT) => "S",
        Some(CompletionItemKind::PROPERTY | CompletionItemKind::FIELD) => "P",
        Some(CompletionItemKind::CONSTANT | CompletionItemKind::ENUM_MEMBER) => "#",
        _ => ".",
    }
}

/// Orders candidates already collected for a `new <prefix>` context. This is
/// deliberately a presentation-only ranking pass: it performs no lookups and
/// uses only the existing label, detail/source and kind fields.
fn rank_new_completion_items(items: &mut [CompletionItem], prefix: &str) {
    items.sort_by_key(|item| {
        let text_rank = if !prefix.is_empty() && item.label.eq_ignore_ascii_case(prefix) {
            0_u8 // exact short-name match
        } else if !prefix.is_empty() && starts_with_ascii_case_insensitive(&item.label, prefix) {
            1_u8 // short-name prefix match
        } else {
            2_u8 // other candidates already accepted by the provider
        };
        let source_rank = item
            .detail
            .as_deref()
            .map(|detail| {
                if detail.contains("Project") {
                    0_u8
                } else if detail.contains("Vendor") {
                    1_u8
                } else if detail.contains("Runtime") {
                    2_u8
                } else {
                    1_u8
                }
            })
            .unwrap_or(1);
        let kind_rank = match item.kind {
            Some(CompletionItemKind::CLASS)
            | Some(CompletionItemKind::CONSTRUCTOR)
            | Some(CompletionItemKind::INTERFACE)
            | Some(CompletionItemKind::ENUM) => 0_u8,
            _ => 1_u8,
        };
        (text_rank, source_rank, kind_rank)
    });
}

fn filter_new_completion_items(items: &mut Vec<CompletionItem>, prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    items.retain(|item| {
        item.label.eq_ignore_ascii_case(prefix)
            || starts_with_ascii_case_insensitive(&item.label, prefix)
    });
}

fn starts_with_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

#[cfg(debug_assertions)]
fn debug_input_enabled() -> bool {
    std::env::var_os("AXIOM_DEBUG_INPUT").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    })
}

#[cfg(debug_assertions)]
fn debug_ui_stall_enabled() -> bool {
    std::env::var_os("AXIOM_DEBUG_UI_STALL").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    })
}

#[cfg(not(debug_assertions))]
fn debug_ui_stall_enabled() -> bool {
    false
}

#[cfg(not(debug_assertions))]
fn debug_input_enabled() -> bool {
    false
}

fn utf8_column_to_utf16(line: &str, byte_column: usize) -> usize {
    line.get(..byte_column.min(line.len()))
        .map(|prefix| prefix.encode_utf16().count())
        .unwrap_or_else(|| line.encode_utf16().count())
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stage = UiStageGuard::new(UI_STAGE_RENDER);
        let frame_started = Instant::now();
        let previous_render = self.last_render_at.replace(frame_started);
        if debug_ui_stall_enabled()
            && let Some(previous) = previous_render
        {
            let gap_us = frame_started.duration_since(previous).as_micros();
            if gap_us >= 30_000 && self.edit_generation > self.last_rendered_edit_generation {
                tracing::info!(target: "axiom.ui_stall",
                    gap_us,
                    dirty_document = true,
                    pending_edit_generation = self.edit_generation,
                    last_rendered_generation = self.last_rendered_edit_generation,
                    "[UI FRAME GAP]"
                );
            }
        }
        self.poll_native_inspection_results(cx);
        let t = theme();
        let m = metrics();
        let combined_diagnostics = self.diagnostics.combined();
        let status = self.status.clone().or_else(|| {
            combined_diagnostics
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
        let line_count = self.document.line_count();
        if debug_ui_stall_enabled() && self.edit_generation != self.last_rendered_edit_generation {
            let generation = self.edit_generation;
            self.last_rendered_edit_generation = generation;
            LAST_UI_RENDERED_GENERATION.store(generation, Ordering::Relaxed);
            if let Some(mutation) = self.last_mutation_at {
                let latency_us = frame_started.duration_since(mutation).as_micros();
                if latency_us >= 10_000 {
                    tracing::info!(target: "axiom.ui_stall",
                        generation,
                        latency_us,
                        file = %self.file_path.display(),
                        lines = line_count,
                        "[UI EDIT TO RENDER]"
                    );
                }
            }
            if let Some((notify_generation, notified_at)) = self.last_notify_at
                && notify_generation == generation
            {
                let latency_us = frame_started.duration_since(notified_at).as_micros();
                if latency_us >= 10_000 {
                    tracing::info!(target: "axiom.ui_stall", generation, latency_us, "[UI NOTIFY TO RENDER]");
                }
            }
        }
        if line_count != self.width_cache_line_count {
            self.line_width_cache.clear();
            self.max_width_line = None;
            self.width_cache_line_count = line_count;
            self.width_cache_dirty = true;
        }
        let profile_started = std::time::Instant::now();
        let mut lines_laid_out = 0usize;
        let mut cache_hits = 0usize;
        let mut cache_misses = 0usize;
        if self.width_cache_dirty {
            let _stage = UiStageGuard::new(UI_STAGE_WIDTH_CACHE_REBUILD);
            let width_rebuild_started = Instant::now();
            self.line_width_cache.clear();
            self.max_width_line = None;
            for line in 0..line_count {
                let raw = self.document.line_content(line);
                let text = trim_eol(raw.as_ref());
                let width =
                    px(GUTTER_WIDTH + TEXT_PADDING) + self.line_layout(line, text, window).width;
                self.line_width_cache.insert(line, width);
                if self
                    .max_width_line
                    .is_none_or(|max| width > self.line_width_cache[&max])
                {
                    self.max_width_line = Some(line);
                }
                lines_laid_out += 1;
                cache_misses += 1;
            }
            self.width_cache_dirty = false;
            let elapsed_us = width_rebuild_started.elapsed().as_micros();
            if debug_ui_stall_enabled() && elapsed_us >= 3_000 {
                tracing::info!(target: "axiom.ui_stall",
                    elapsed_us,
                    lines = line_count,
                    layouts_reused = 0usize,
                    layouts_reshaped = lines_laid_out,
                    "[UI WIDTH CACHE REBUILD]"
                );
            }
        }
        let cursor_line = self.document.line_of_offset(self.document.cursor_offset());
        if !self.line_width_cache.contains_key(&cursor_line) {
            let raw = self.document.line_content(cursor_line);
            let text = trim_eol(raw.as_ref());
            let width =
                px(GUTTER_WIDTH + TEXT_PADDING) + self.line_layout(cursor_line, text, window).width;
            self.line_width_cache.insert(cursor_line, width);
            self.max_width_line = self.max_width_line.filter(|line| {
                self.line_width_cache
                    .get(line)
                    .is_some_and(|old| *old >= width)
            });
            if self
                .max_width_line
                .is_none_or(|line| width > self.line_width_cache[&line])
            {
                self.max_width_line = Some(cursor_line);
            }
            lines_laid_out += 1;
            cache_misses += 1;
        } else {
            cache_hits += 1;
        }
        self.content_width = self
            .max_width_line
            .and_then(|line| self.line_width_cache.get(&line).copied())
            .unwrap_or(viewport.size.width);
        if debug_input_enabled() {
            tracing::debug!(
                lines_total = line_count,
                lines_laid_out_this_frame = lines_laid_out,
                width_cache_hits = cache_hits,
                width_cache_misses = cache_misses,
                width_scan_all = cache_misses == line_count && line_count > 0,
                render_us = profile_started.elapsed().as_micros(),
                "[EDITOR RENDER PROFILE]"
            );
        }
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
            .fold(360.0, f32::max);
        let viewport_width: f32 = viewport.size.width.into();
        let popup_width = px(estimated_width
            .min(520.0)
            .min((viewport_width - 16.0).max(180.0)));
        let mut popup_x = px(GUTTER_WIDTH + TEXT_PADDING) + caret_x;
        popup_x = popup_x.min((px(viewport_w) - popup_width).max(px(0.)));
        let scroll_y = self.scroll.0.borrow().base_handle.offset().y;
        let mut below_y = px((line as f32 + 1.0) * LINE_HEIGHT) + scroll_y;
        let row_height = px(26.0);
        let popup_height = px((self.completions.len() as f32 * 26.0).min(224.0));
        if let Some(anchor) = self.hover_anchor {
            popup_x = anchor.x.max(px(0.0));
            popup_x = popup_x.min((px(viewport_w) - popup_width).max(px(0.0)));
            below_y = (anchor.y + px(LINE_HEIGHT)).min(px(viewport_h));
        }
        let opens_above = below_y + popup_height > px(viewport_h);
        let mut popup_y = if opens_above {
            (below_y - popup_height - px(4.0)).max(px(0.0))
        } else {
            below_y.min((px(viewport_h) - popup_height).max(px(0.0)))
        };
        popup_y = popup_y.min((px(viewport_h) - popup_height).max(px(0.0)));
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
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                        this.editor_scroll_drag_move(
                            EditorScrollEventSource::Viewport,
                            event,
                            window,
                            cx,
                        )
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, window, cx| {
                            this.editor_scroll_drag_end(event, window, cx);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, window, cx| {
                            this.editor_scroll_drag_end(event, window, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
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
                            cx.processor(move |this, range: Range<usize>, window, _| {
                                this.line_layouts
                                    .borrow_mut()
                                    .retain(|line, _| range.contains(line));
                                range
                                    .map(|line| {
                                        this.render_line(line, window, &combined_diagnostics)
                                    })
                                    .collect()
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
                                .on_mouse_move(cx.listener(
                                    |this, event: &MouseMoveEvent, window, cx| {
                                        this.editor_scroll_drag_move(
                                            EditorScrollEventSource::Scrollbar,
                                            event,
                                            window,
                                            cx,
                                        )
                                    },
                                ))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                        this.editor_scroll_drag_start(
                                            EditorScrollAxis::Vertical,
                                            event,
                                            window,
                                            cx,
                                        )
                                    }),
                                )
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseUpEvent, window, cx| {
                                        this.editor_scroll_drag_end(event, window, cx);
                                    }),
                                )
                                .on_mouse_up_out(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseUpEvent, window, cx| {
                                        this.editor_scroll_drag_end(event, window, cx);
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
                                        .bg(
                                            if self.editor_scroll_drag_axis
                                                == Some(EditorScrollAxis::Vertical)
                                            {
                                                t.accent
                                            } else {
                                                t.scrollbar_hover
                                            },
                                        ),
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
                                .on_mouse_move(cx.listener(
                                    |this, event: &MouseMoveEvent, window, cx| {
                                        this.editor_scroll_drag_move(
                                            EditorScrollEventSource::Scrollbar,
                                            event,
                                            window,
                                            cx,
                                        )
                                    },
                                ))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                        this.editor_scroll_drag_start(
                                            EditorScrollAxis::Horizontal,
                                            event,
                                            window,
                                            cx,
                                        )
                                    }),
                                )
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseUpEvent, window, cx| {
                                        this.editor_scroll_drag_end(event, window, cx);
                                    }),
                                )
                                .on_mouse_up_out(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseUpEvent, window, cx| {
                                        this.editor_scroll_drag_end(event, window, cx);
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
                                        .bg(
                                            if self.editor_scroll_drag_axis
                                                == Some(EditorScrollAxis::Horizontal)
                                            {
                                                t.accent
                                            } else {
                                                t.scrollbar_hover
                                            },
                                        ),
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
                                .bg(t.elevated_surface)
                                .border_1()
                                .border_color(t.border_subtle)
                                .shadow_lg()
                                .occlude()
                                .children(
                                    self.completions
                                        .iter()
                                        .enumerate()
                                        .zip(presentations.iter())
                                        .map(|((index, item), presentation)| {
                                            let editor = cx.entity();
                                            let selected = index == self.completion_selected;
                                            div()
                                                .id(("completion-item", index))
                                                .h(row_height)
                                                .w_full()
                                                .px_1()
                                                .flex()
                                                .items_center()
                                                .overflow_hidden()
                                                .on_click(move |_, _, cx| {
                                                    editor.update(cx, |this, cx| {
                                                        this.completion_selected = index;
                                                        this.accept_completion(cx);
                                                    });
                                                })
                                                .bg(if selected {
                                                    t.selection
                                                } else {
                                                    t.elevated_surface
                                                })
                                                .hover(move |style| {
                                                    style.bg(if selected {
                                                        t.selection
                                                    } else {
                                                        t.hover
                                                    })
                                                })
                                                .text_color(t.text_primary)
                                                .child(
                                                    div()
                                                        .w(px(16.0))
                                                        .flex_none()
                                                        .mr_1()
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
                                                                .w(px(72.))
                                                                .flex_none()
                                                                .overflow_hidden()
                                                                .ml_1()
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
                                                                .w(px(64.))
                                                                .flex_none()
                                                                .overflow_hidden()
                                                                .ml_1()
                                                                .justify_end()
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
        self.reset_caret_blink(cx);
        let range = range
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range());
        let smart_arrow =
            text == "-" && range.start == range.end && self.should_expand_member_dash();
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
    text.strip_suffix(
        "
",
    )
    .or_else(|| text.strip_suffix('\n'))
    .unwrap_or(text)
}

fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (offset, ch) in text[open..].char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
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

fn emit_project_type_diagnostics(
    input: &ArgumentInspectionInput,
    snapshot: &SemanticSnapshot,
    scope: axiom_index::ScopeId,
    symbol_id: axiom_index::SymbolId,
    arguments_node: tree_sitter::Node<'_>,
    out: &mut Vec<ByteDiagnostic>,
) {
    let Some(symbol) = snapshot.symbol(symbol_id) else {
        return;
    };
    if symbol.structured_parameters.is_empty() {
        return;
    }
    let resolver = axiom_index::ExpressionResolver::new(snapshot, scope);
    let parameters = &symbol.structured_parameters;
    let mut parameter_index = 0usize;
    for argument in arguments_node.named_children(&mut arguments_node.walk()) {
        // Named arguments and unpacking need dedicated mapping semantics; do
        // not guess at their parameter or type in this conservative phase.
        if argument.kind() == "named_argument" || argument.kind() == "variadic_unpacking" {
            parameter_index += 1;
            continue;
        }
        let Some(parameter) = parameters
            .get(parameter_index)
            .or_else(|| parameters.last().filter(|parameter| parameter.variadic))
        else {
            break;
        };
        if let Some(expected) = parameter.declared_type.as_ref() {
            if let Some(actual) = resolver.infer_ast_expression_type(argument, input.text.as_ref())
            {
                let compatibility = declared_type_compatibility(snapshot, expected, &actual);
                if compatibility == TypeCompatibility::Incompatible {
                    out.push(ByteDiagnostic {
                        range: argument.start_byte()..argument.end_byte(),
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: format!(
                            "Expected {}, found {}",
                            declared_type_label(expected),
                            declared_type_label(&actual)
                        ),
                    });
                }
            }
        }
        if !parameter.variadic {
            parameter_index += 1;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParameterArity {
    required_count: usize,
    maximum_count: usize,
    variadic: bool,
}

fn signature_counts_from_detail(detail: &str) -> ParameterArity {
    let Some(open) = detail.find('(') else {
        return ParameterArity {
            required_count: 0,
            maximum_count: 0,
            variadic: false,
        };
    };
    let Some(close) = matching_paren(detail, open) else {
        return ParameterArity {
            required_count: 0,
            maximum_count: 0,
            variadic: false,
        };
    };
    let parameters = &detail[open + 1..close];
    if parameters.trim().is_empty() {
        return ParameterArity {
            required_count: 0,
            maximum_count: 0,
            variadic: false,
        };
    }
    let mut required = 0;
    let mut maximum = 0;
    let mut variadic = false;
    for parameter in split_signature_parameters(parameters) {
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
    ParameterArity {
        required_count: required,
        maximum_count: maximum,
        variadic,
    }
}

/// Splits a PHP parameter list only at top-level commas. Types and defaults
/// may contain nested arrays/calls and quoted strings with commas.
fn split_signature_parameters(parameters: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut nesting = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (offset, ch) in parameters.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => nesting += 1,
            ')' | ']' | '}' => nesting = nesting.saturating_sub(1),
            ',' if nesting == 0 => {
                result.push(&parameters[start..offset]);
                start = offset + ch.len_utf8();
            }
            _ => {}
        }
    }
    result.push(&parameters[start..]);
    result
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
mod completion_ranking_tests {
    use super::{filter_new_completion_items, rank_new_completion_items};
    use lsp_types::{CompletionItem, CompletionItemKind};

    fn item(label: &str, detail: &str, kind: CompletionItemKind) -> CompletionItem {
        CompletionItem {
            label: label.to_owned(),
            detail: Some(detail.to_owned()),
            kind: Some(kind),
            ..Default::default()
        }
    }

    #[test]
    fn exact_new_class_is_ranked_first() {
        let mut items = vec![
            item(
                "ArrayIterator",
                "ArrayIterator • PHP Runtime",
                CompletionItemKind::CLASS,
            ),
            item(
                "AliasStatus",
                "AliasStatus • PHP Runtime",
                CompletionItemKind::CLASS,
            ),
            item(
                "ChildService",
                "App\\ChildService • Project",
                CompletionItemKind::CLASS,
            ),
            item("A", "App\\A • Project", CompletionItemKind::CLASS),
        ];
        rank_new_completion_items(&mut items, "A");
        assert_eq!(items[0].label, "A");
    }

    #[test]
    fn prefix_beats_non_matching_project_and_exact_is_case_insensitive() {
        let mut items = vec![
            item(
                "ArrayIterator",
                "ArrayIterator • Project",
                CompletionItemKind::CLASS,
            ),
            item("Base", "Base • Project", CompletionItemKind::CLASS),
            item("Baz", "Baz • Project", CompletionItemKind::CLASS),
            item(
                "ChildImplementation",
                "ChildImplementation • Runtime",
                CompletionItemKind::CLASS,
            ),
            item("CHILD", "App\\Child • Project", CompletionItemKind::CLASS),
        ];
        rank_new_completion_items(&mut items, "Child");
        assert_eq!(items[0].label, "CHILD");
        assert_eq!(items[1].label, "ChildImplementation");
        assert!(
            items[2..]
                .iter()
                .all(|candidate| { !candidate.label.starts_with("Child") })
        );
    }

    #[test]
    fn project_wins_when_textual_match_is_equivalent() {
        let mut items = vec![
            item("Alpha", "Alpha • PHP Runtime", CompletionItemKind::CLASS),
            item("Alpha", "App\\Alpha • Project", CompletionItemKind::CLASS),
        ];
        rank_new_completion_items(&mut items, "Al");
        assert_eq!(items[0].detail.as_deref(), Some("App\\Alpha • Project"));
    }

    #[test]
    fn ranking_is_stable_for_non_new_contexts() {
        let mut items = vec![
            item("Beta", "Beta • PHP Runtime", CompletionItemKind::CLASS),
            item("Alpha", "App\\Alpha • Project", CompletionItemKind::CLASS),
        ];
        let original = items.clone();
        // The helper is only invoked for `new`; other contexts retain their
        // existing provider order because no ranking pass is applied.
        assert_eq!(items, original);
        rank_new_completion_items(&mut items, "Z");
        assert_ne!(items, original);
    }

    #[test]
    fn new_prefix_filter_removes_candidates_from_previous_prefixes() {
        let mut items = vec![
            item(
                "ArrayIterator",
                "ArrayIterator • Runtime",
                CompletionItemKind::CLASS,
            ),
            item("Base", "Base • Project", CompletionItemKind::CLASS),
            item("Child", "App\\Child • Project", CompletionItemKind::CLASS),
        ];
        filter_new_completion_items(&mut items, "Chil");
        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Child"]
        );
    }
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
                    ..
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
                Some(DefinitionQuery::Name { fqn })
                    if fqn == "Omegaalfa\\FiberEventLoop\\Future"
            ),
            "unexpected imported type query: {query:?}"
        );
    }

    #[gpui::test]
    fn definition_query_does_not_treat_new_rhs_after_member_assignment_as_method(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = "<?php\nnamespace Omegaalfa\\HttpClient\\Http;\nuse Omegaalfa\\FiberEventLoop\\FiberEventLoop;\nfinal class AsyncHttpClient\n{\n    private FiberEventLoop $loop;\n    public function __construct(?FiberEventLoop $loop = null): void\n    {\n        $this->loop = $loop ?? new FiberEventLoop();\n    }\n}\n";
        let path = std::env::temp_dir().join("axiom-new-after-member-assignment.php");
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
        let offset = source.rfind("FiberEventLoop();").unwrap() + 2;
        view.update(cx, |editor, _| editor.document.move_cursor(offset));
        let query = view.update(cx, |editor, _| editor.definition_query());
        assert!(
            matches!(
                &query,
                Some(DefinitionQuery::Name { fqn })
                    if fqn == "Omegaalfa\\FiberEventLoop\\FiberEventLoop"
            ),
            "unexpected new-expression query: {query:?}"
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
        let text = "namespace App;\nuse Vendor\\FiberEventLoop\\FiberEventLoop;\nclass Client {\n private FiberEventLoop $loop;\n function f() { $this->loop->run(); }\\n}";
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

#[cfg(test)]
mod diagnostic_store_tests {
    use super::{
        ArgumentInspectionInput, ByteDiagnostic, DiagnosticStore, DuplicateClassDeclaration,
        DuplicateClassInspectionInput, ParameterArity, PersistentFileKey,
        UnknownConstantInspectionInput, compute_argument_inspections,
        compute_duplicate_class_inspections, compute_unknown_constant_inspections,
        run_native_inspection_rules, signature_counts_from_detail,
    };
    use std::cell::Cell;
    use std::fs;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn stale_native_inspection_stops_before_subsequent_rules() {
        let latest_generation = AtomicU64::new(1);
        let first_rule_ran = Cell::new(false);

        let result = run_native_inspection_rules(
            &latest_generation,
            1,
            || {
                first_rule_ran.set(true);
                latest_generation.store(2, Ordering::Release);
                Vec::new()
            },
            || panic!("unknown constant inspection must not run after staleness"),
            || panic!("duplicate class inspection must not run after staleness"),
            || panic!("argument inspection must not run after staleness"),
        );

        assert!(first_rule_ran.get());
        assert!(result.is_none());
    }

    #[test]
    fn argument_inspection_is_pure_and_send_sync() {
        assert_send_sync::<ArgumentInspectionInput>();
        let input = ArgumentInspectionInput {
            text: Arc::from("<?php function run(int $value) {} run();"),
            project_symbols: vec![axiom_index::ProjectSymbol {
                name: "run".into(),
                fully_qualified_name: "run".into(),
                kind: axiom_index::ProjectSymbolKind::Function,
                file: "test.php".into(),
                range: 0..3,
                namespace: String::new(),
                visibility: axiom_index::Visibility::Public,
                modifiers: Vec::new(),
                parameters: Some("(int $value)".into()),
                return_type: None,
            }],
            runtime_symbols: None,
            semantic_snapshot: None,
            file_key: PersistentFileKey::workspace_lexical("test.php"),
        };
        assert_eq!(compute_argument_inspections(&input).len(), 1);
    }

    #[test]
    fn argument_inspection_parses_once_and_counts_ast_arguments() {
        let input = ArgumentInspectionInput {
            text: Arc::from(
                "<?php function run(int $value) {} function pair(int $a, int $b) {}\n// fake(1, 2)\nrun(); run(1); run(\"a,b\"); pair(1, 2); run(pair(1, 2));",
            ),
            project_symbols: vec![
                axiom_index::ProjectSymbol {
                    name: "run".into(),
                    fully_qualified_name: "run".into(),
                    kind: axiom_index::ProjectSymbolKind::Function,
                    file: "test.php".into(),
                    range: 0..3,
                    namespace: String::new(),
                    visibility: axiom_index::Visibility::Public,
                    modifiers: Vec::new(),
                    parameters: Some("(int $value)".into()),
                    return_type: None,
                },
                axiom_index::ProjectSymbol {
                    name: "pair".into(),
                    fully_qualified_name: "pair".into(),
                    kind: axiom_index::ProjectSymbolKind::Function,
                    file: "test.php".into(),
                    range: 0..4,
                    namespace: String::new(),
                    visibility: axiom_index::Visibility::Public,
                    modifiers: Vec::new(),
                    parameters: Some("(int $a, int $b)".into()),
                    return_type: None,
                },
            ],
            runtime_symbols: None,
            semantic_snapshot: None,
            file_key: PersistentFileKey::workspace_lexical("test.php"),
        };
        let diagnostics = compute_argument_inspections(&input);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("Expected 1 argument"));
    }

    #[test]
    fn parameter_arity_handles_defaults_variadics_and_nested_commas() {
        assert_eq!(
            signature_counts_from_detail("(array $value)"),
            ParameterArity {
                required_count: 1,
                maximum_count: 1,
                variadic: false,
            }
        );
        assert_eq!(
            signature_counts_from_detail("(int $a, ?Foo|Bar $b, Baz&Qux $c)"),
            ParameterArity {
                required_count: 3,
                maximum_count: 3,
                variadic: false,
            }
        );
        assert_eq!(
            signature_counts_from_detail("(int $a, array $options = [1, 2, 3])"),
            ParameterArity {
                required_count: 1,
                maximum_count: 2,
                variadic: false,
            }
        );
        assert_eq!(
            signature_counts_from_detail("(?Foo $value = null, ...$rest)"),
            ParameterArity {
                required_count: 0,
                maximum_count: 2,
                variadic: true,
            }
        );
        assert_eq!(
            signature_counts_from_detail("(string $value = fn(1, 'a,b'))"),
            ParameterArity {
                required_count: 0,
                maximum_count: 1,
                variadic: false,
            }
        );
    }

    #[test]
    fn argument_inspection_uses_semantic_inherited_method_signature() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("semantic.php");
        let text = "<?php class Base { public function run(int $value): void {} } class Child extends Base {} function test(Child $child): void { $child->run(); }";
        fs::write(&path, text).unwrap();
        let mut index = axiom_index::ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = axiom_index::SemanticSnapshot::from_project_index(
            &index,
            axiom_index::SemanticRevision(1),
        );
        let input = ArgumentInspectionInput {
            text: Arc::from(text),
            project_symbols: index.symbols().to_vec(),
            runtime_symbols: None,
            semantic_snapshot: Some(Arc::new(snapshot)),
            file_key: PersistentFileKey::workspace_lexical(&path),
        };
        let diagnostics = compute_argument_inspections(&input);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("Expected 1 argument"));
    }

    #[test]
    fn argument_inspection_validates_object_creation_constructors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("constructors.php");
        let text = "<?php class Base { public function __construct(string $name) {} } class Child extends Base {} class Optional { public function __construct(?string $value = null) {} } class Variadic { public function __construct(...$values) {} } new Child(); new Child(\"ok\"); new Optional(); new Variadic();";
        fs::write(&path, text).unwrap();
        let mut index = axiom_index::ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = axiom_index::SemanticSnapshot::from_project_index(
            &index,
            axiom_index::SemanticRevision(1),
        );
        let input = ArgumentInspectionInput {
            text: Arc::from(text),
            project_symbols: index.symbols().to_vec(),
            runtime_symbols: None,
            semantic_snapshot: Some(Arc::new(snapshot)),
            file_key: PersistentFileKey::workspace_lexical(&path),
        };
        let diagnostics = compute_argument_inspections(&input);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("Expected 1 argument"));
    }

    #[test]
    fn argument_diagnostic_range_is_exact_arguments_node() {
        let text = "<?php class A { public function __construct(string $name) {} } new A();";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("arity-range.php");
        fs::write(&path, text).unwrap();
        let mut index = axiom_index::ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = axiom_index::SemanticSnapshot::from_project_index(
            &index,
            axiom_index::SemanticRevision(1),
        );
        let input = ArgumentInspectionInput {
            text: Arc::from(text),
            project_symbols: index.symbols().to_vec(),
            runtime_symbols: None,
            semantic_snapshot: Some(Arc::new(snapshot)),
            file_key: PersistentFileKey::workspace_lexical(&path),
        };
        let diagnostics = compute_argument_inspections(&input);
        let arguments_start = text.rfind("()").expect("arguments node");
        let expected = arguments_start..arguments_start + 2;
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].range, expected);
    }

    #[test]
    fn argument_type_inspection_is_conservative_and_marks_only_mismatches() {
        let text = r#"<?php
class Base {}
class Child extends Base {}
class Other {}
class Service {
    public function __construct(string $name) {}
    public function check(int|string $union, ?string $nullable, mixed $anything, Base $base, int ...$numbers): void {}
}
function test(Service $service, $unknown): void {
    new Service(22);
    $service->check(1, null, new Other(), new Child(), 2, 3);
    $service->check('ok', 'name', $unknown, new Child(), 4);
    $service->check(new Other(), null, null, new Other(), 5, 'bad');
}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("argument-types.php");
        fs::write(&path, text).unwrap();
        let mut index = axiom_index::ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = axiom_index::SemanticSnapshot::from_project_index(
            &index,
            axiom_index::SemanticRevision(1),
        );
        let input = ArgumentInspectionInput {
            text: Arc::from(text),
            project_symbols: index.symbols().to_vec(),
            runtime_symbols: None,
            semantic_snapshot: Some(Arc::new(snapshot)),
            file_key: PersistentFileKey::workspace_lexical(&path),
        };

        let diagnostics = compute_argument_inspections(&input);
        let other_ranges = text
            .match_indices("new Other()")
            .map(|(start, value)| start..start + value.len())
            .collect::<Vec<_>>();
        let number_start = text.find("new Service(22)").unwrap() + "new Service(".len();
        let bad_start = text.rfind("'bad'").unwrap();
        let expected = vec![
            number_start..number_start + 2,
            other_ranges[1].clone(),
            other_ranges[2].clone(),
            bad_start..bad_start + "'bad'".len(),
        ];
        let actual = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.range.clone())
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic.message.starts_with("Expected ") && !diagnostic.message.contains("unknown")
        }));
    }

    #[test]
    fn duplicate_class_inspection_is_pure_and_send_sync() {
        assert_send_sync::<DuplicateClassInspectionInput>();
        let a = axiom_index::PersistentFileKey::workspace("A.php");
        let b = axiom_index::PersistentFileKey::workspace("B.php");
        let input = DuplicateClassInspectionInput {
            path: a.clone(),
            declarations: vec![
                DuplicateClassDeclaration {
                    fqn: "App\\User".into(),
                    file: a,
                    range: 1..5,
                },
                DuplicateClassDeclaration {
                    fqn: "App\\User".into(),
                    file: b,
                    range: 2..6,
                },
            ],
        };
        assert_eq!(compute_duplicate_class_inspections(&input).len(), 1);
    }

    #[test]
    fn duplicate_class_inspection_distinguishes_fqn_and_self() {
        assert_send_sync::<DuplicateClassInspectionInput>();
        let file = axiom_index::PersistentFileKey::workspace("same.php");
        let other = axiom_index::PersistentFileKey::workspace("other.php");
        let input = DuplicateClassInspectionInput {
            path: file.clone(),
            declarations: vec![
                DuplicateClassDeclaration {
                    fqn: "App\\One\\User".into(),
                    file: file.clone(),
                    range: 1..2,
                },
                DuplicateClassDeclaration {
                    fqn: "App\\Two\\User".into(),
                    file: file.clone(),
                    range: 3..4,
                },
                DuplicateClassDeclaration {
                    fqn: "App\\One\\User".into(),
                    file: other,
                    range: 5..6,
                },
            ],
        };
        let diagnostics = compute_duplicate_class_inspections(&input);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].range, 1..2);
    }

    #[test]
    fn unknown_constant_inspection_is_pure_and_send_sync() {
        assert_send_sync::<UnknownConstantInspectionInput>();
        let input = UnknownConstantInspectionInput {
            text: Arc::from("<?php echo UNKNOWN_VALUE;"),
            known_constants: Vec::new(),
            runtime_symbols: None,
        };
        let diagnostics = compute_unknown_constant_inspections(&input);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "Undefined constant 'UNKNOWN_VALUE'");
    }

    #[test]
    fn unknown_constant_inspection_preserves_known_namespaces_and_literals() {
        let input = UnknownConstantInspectionInput {
            text: Arc::from(
                "<?php namespace App; const VALUE = 1; echo VALUE; echo \\App\\VALUE; echo \"FAKE\"; // FAKE\n",
            ),
            known_constants: vec![("VALUE".into(), "App\\VALUE".into())],
            runtime_symbols: None,
        };
        assert!(compute_unknown_constant_inspections(&input).is_empty());
    }

    #[test]
    fn native_and_lsp_updates_are_independent_and_combined() {
        let mut store = DiagnosticStore::default();
        store.set_native_syntax(vec![ByteDiagnostic {
            range: 0..3,
            severity: None,
            message: "native".into(),
        }]);
        store.set_lsp(vec![ByteDiagnostic {
            range: 4..7,
            severity: None,
            message: "lsp".into(),
        }]);
        let first_cache = store.combined();
        let second_cache = store.combined();
        assert!(Arc::ptr_eq(&first_cache, &second_cache));
        assert_eq!(store.combined().len(), 2);
        store.set_lsp(Vec::new());
        assert_eq!(
            store
                .combined()
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>(),
            vec!["native"]
        );
        store.set_native_syntax(Vec::new());
        store.set_lsp(vec![ByteDiagnostic {
            range: 1..2,
            severity: None,
            message: "lsp-again".into(),
        }]);
        assert_eq!(store.combined()[0].message, "lsp-again");
    }
}
