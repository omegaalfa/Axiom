use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Instant,
};

use axiom_app::commands::Keymap;
use axiom_app::shell_state::{
    RecentProjects, StartupTarget, composer_vendor_cache_path, project_symbol_cache_path,
    recent_projects_path, runtime_stubs_cache_path, runtime_stubs_default_path, unix_timestamp_now,
};
use axiom_editor::Document;
use axiom_index::{
    FindUsagesOptions, ProjectSymbolIndex, ReferenceRole, SemanticEngine, SemanticRevision,
    SemanticSnapshot, SnapshotBuilder, VendorSymbolIndex,
};
use axiom_lsp::{PositionCodec, ServerStatus, uri_to_path};
use axiom_php::{RuntimeSymbolIndex, StubProvider};
use axiom_project::{EntryKind, FileContent, Project, ProjectEntry, read_file_content};
use axiom_terminal::{TerminalLink, TerminalLinkKind, TerminalProfile, TerminalSession};
use gpui::{
    Action, App, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, KeyDownEvent,
    LayoutId, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollHandle, SharedString, Style, Timer, UTF16Selection, Window, actions, div, prelude::*, px,
    relative,
};

use crate::{
    editor_view::EditorView,
    lsp_bridge::{IdeLspEvent, LspBridge, LspRequestKind},
    terminal_view::TerminalView,
    ui::{
        components::tooltip,
        icons::{ActivityIcon, activity_icon, file_icon},
        metrics, theme,
    },
};

actions!(
    workspace,
    [
        OpenProject,
        OpenFile,
        SaveAll,
        CloseFile,
        CloseProject,
        Exit,
        ShowAbout,
        ShowFeatures,
        Find,
        ToggleProject,
        ToggleTerminal,
        OpenInTerminal,
        ImportRuntimeStubs,
        ImportRuntimeStubFiles,
        CommandPalette,
        Settings,
        PaletteUp,
        PaletteDown,
        PaletteConfirm,
        PaletteEscape,
        NavigateBack,
        NavigateForward,
        GoToClass,
        GoToSymbol,
        DebugInput,
        GoToImplementation,
        CloseFindUsages,
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-shift-o", OpenProject, None),
        KeyBinding::new("secondary-o", OpenFile, None),
        KeyBinding::new("secondary-f", Find, None),
        KeyBinding::new("secondary-`", ToggleTerminal, None),
        KeyBinding::new("ctrl-shift-p", CommandPalette, None),
        KeyBinding::new("up", PaletteUp, Some("CommandPalette")),
        KeyBinding::new("down", PaletteDown, Some("CommandPalette")),
        KeyBinding::new("enter", PaletteConfirm, Some("CommandPalette")),
        KeyBinding::new("escape", PaletteEscape, Some("CommandPalette")),
        KeyBinding::new("alt-left", NavigateBack, None),
        KeyBinding::new("alt-right", NavigateForward, None),
        KeyBinding::new("f12", DebugInput, None),
        KeyBinding::new("ctrl-alt-b", GoToImplementation, None),
        KeyBinding::new("alt-f7", crate::editor_view::References, None),
    ]
}

struct OpenTab {
    path: PathBuf,
    editor: Entity<EditorView>,
}

#[derive(Clone)]
struct NavigationLocation {
    path: PathBuf,
    position: lsp_types::Position,
}

#[derive(Clone)]
struct DefinitionTarget {
    path: PathBuf,
    position: lsp_types::Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetTextSource {
    Memory,
    Disk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FindUsageTarget {
    file: PathBuf,
    span: std::ops::Range<usize>,
    role: Option<ReferenceRole>,
    position: lsp_types::Position,
    label: String,
    snippet: String,
}

struct FindUsagesContext {
    kind: NavigationQueryKind,
    project_generation: u64,
    snapshot: Arc<axiom_index::SemanticSnapshot>,
    documents: Vec<(PathBuf, u64, u64)>,
    source_session: u64,
}

// Shared presentation for both semantic result lists; no document/query access.
fn semantic_row_colors(selected: bool, hovered: bool) -> (gpui::Rgba, gpui::Rgba) {
    let t = theme();
    (
        if selected {
            t.inactive_selection
        } else {
            t.popup_background
        },
        if selected {
            t.accent
        } else if hovered {
            t.border
        } else {
            t.popup_background
        },
    )
}

fn semantic_row_height() -> Pixels {
    (metrics().ui_font_size + metrics().spacing_xs) * 2. + metrics().spacing_xs
}

fn semantic_list_height(count: usize) -> Pixels {
    semantic_row_height() * count.min(10) as f32
}

fn valid_php_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

fn valid_php_namespace(value: &str) -> bool {
    let value = value.trim().trim_matches('\\');
    value.is_empty() || value.split('\\').all(valid_php_identifier)
}

fn relative_directory_label(root: Option<&Path>, directory: &Path) -> String {
    root.and_then(|root| directory.strip_prefix(root).ok())
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.display().to_string().replace('\\', "/"))
        .unwrap_or_else(|| ".".into())
}

fn semantic_popup_geometry(
    anchor: Point<Pixels>,
    viewport: gpui::Size<Pixels>,
    count: usize,
) -> gpui::Bounds<Pixels> {
    let margin = metrics().spacing_sm;
    let width = px(420.).min((viewport.width - margin * 2.).max(px(0.)));
    let header = metrics().ui_font_size + metrics().spacing_xs * 3. + px(1.);
    let height = (header + semantic_list_height(count.max(1)) + px(2.))
        .min((viewport.height - margin * 2.).max(px(0.)));
    let x = if anchor.x + margin + width <= viewport.width - margin {
        anchor.x + margin
    } else {
        anchor.x - width - margin
    };
    let y = if anchor.y + margin + height <= viewport.height - margin {
        anchor.y + margin
    } else {
        anchor.y - metrics().editor_line_height - height - margin
    };
    gpui::Bounds::new(
        gpui::point(
            x.max(margin).min((viewport.width - width).max(px(0.))),
            y.max(margin).min((viewport.height - height).max(px(0.))),
        ),
        gpui::size(width, height),
    )
}

fn modal_type_popup_geometry(
    anchor: Point<Pixels>,
    viewport: gpui::Size<Pixels>,
    count: usize,
    footer_top: Option<Pixels>,
) -> gpui::Bounds<Pixels> {
    let margin = metrics().spacing_sm;
    let width = px(486.).min((viewport.width - margin * 2.).max(px(0.)));
    let height =
        (px(42.) * count.clamp(1, 5) as f32).min((viewport.height - margin * 2.).max(px(0.)));
    let below = anchor.y + px(22.) + margin;
    let below_limit = footer_top
        .unwrap_or(viewport.height - margin)
        .min(viewport.height - margin);
    let above = anchor.y - margin;
    let y = if below + height <= below_limit {
        below
    } else if above - height >= margin {
        above - height
    } else {
        below.min((viewport.height - margin - height).max(margin))
    };
    let x = anchor
        .x
        .max(margin)
        .min((viewport.width - width - margin).max(margin));
    gpui::Bounds::new(gpui::point(x, y), gpui::size(width, height))
}

#[cfg(test)]
mod semantic_popup_visual_tests {
    use super::*;

    #[test]
    fn modal_type_popup_flips_and_limits_without_modal_resize() {
        let viewport = gpui::size(px(800.), px(600.));
        let below = modal_type_popup_geometry(gpui::point(px(100.), px(100.)), viewport, 4, None);
        assert!(below.origin.y > px(100.));
        let above = modal_type_popup_geometry(gpui::point(px(100.), px(560.)), viewport, 4, None);
        assert!(above.bottom() <= viewport.height);
        assert!(above.origin.y < px(560.));
        let limited =
            modal_type_popup_geometry(gpui::point(px(100.), px(300.)), viewport, 100, None);
        assert!(limited.size.height <= px(210.));
        assert!(limited.bottom() <= viewport.height - metrics().spacing_sm);
        assert_eq!(below.size.width, above.size.width);
        let footer_safe =
            modal_type_popup_geometry(gpui::point(px(100.), px(500.)), viewport, 4, Some(px(530.)));
        assert!(footer_safe.origin.y < px(500.));
    }

    #[test]
    fn hover_preserves_selection_background_and_accent() {
        let selected = semantic_row_colors(true, false);
        assert_eq!(selected, semantic_row_colors(true, true));
        assert_eq!(selected.0, theme().inactive_selection);
        assert_eq!(selected.1, theme().accent);
        assert_ne!(selected.0, theme().accent);
        let normal = semantic_row_colors(false, false);
        let hovered = semantic_row_colors(false, true);
        assert_eq!(normal.0, hovered.0);
        assert_ne!(normal.1, hovered.1);
    }

    #[test]
    fn compact_list_fits_one_two_and_caps_large_results() {
        assert_eq!(semantic_list_height(0), px(0.));
        assert_eq!(semantic_list_height(1), px(36.));
        assert_eq!(semantic_list_height(2), px(72.));
        assert_eq!(semantic_list_height(1000), px(360.));
    }

    #[test]
    fn contextual_popup_flips_and_stays_in_viewport() {
        let viewport = gpui::size(px(800.), px(600.));
        let normal = semantic_popup_geometry(gpui::point(px(100.), px(100.)), viewport, 2);
        assert_eq!(normal.origin, gpui::point(px(107.), px(107.)));
        assert_eq!(normal.size.width, px(420.));
        let right = semantic_popup_geometry(gpui::point(px(790.), px(100.)), viewport, 2);
        assert!(right.right() < px(790.));
        let bottom = semantic_popup_geometry(gpui::point(px(100.), px(590.)), viewport, 2);
        assert!(bottom.bottom() < px(590.));
        for (width, height) in [(800., 600.), (300., 150.), (8., 8.)] {
            let viewport = gpui::size(px(width), px(height));
            for (x, y) in [(0., 0.), (width, height), (-100., -100.)] {
                let bounds = semantic_popup_geometry(gpui::point(px(x), px(y)), viewport, 1000);
                assert!(bounds.left() >= px(0.) && bounds.top() >= px(0.));
                assert!(bounds.right() <= viewport.width && bounds.bottom() <= viewport.height);
            }
        }
    }

    #[test]
    fn shared_render_stays_virtualized_and_hover_has_no_state_or_io() {
        let source = include_str!("workspace_view.rs");
        let render = source
            .rsplit("    fn render_find_usages(")
            .next()
            .unwrap()
            .split("    fn project_panel_resize_start(")
            .next()
            .unwrap();
        assert!(render.contains("gpui::uniform_list("));
        assert!(render.contains("context.kind.title()"));
        assert!(render.contains(".border_color(t.border_subtle)"));
        assert!(render.contains("index + 1 < this.find_usages.len()"));
        assert!(render.contains("target.file.display().to_string()"));
        for forbidden in [
            "fs::",
            "document.content()",
            "find_usages_at(",
            "implementation_targets_at(",
            "on_mouse_move",
            "on_hover",
        ] {
            assert!(
                !render.contains(forbidden),
                "unexpected render work: {forbidden}"
            );
        }
        let compact: String = render.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("style.border_color(semantic_row_colors(selected,true).1)"));
    }

    #[test]
    fn php_creation_validation_and_psr4_directory_label_are_conservative() {
        assert!(!valid_php_identifier(""));
        assert!(valid_php_identifier("UserService"));
        assert!(valid_php_identifier("Éxample_2"));
        assert!(!valid_php_identifier("User-Service"));
        assert!(valid_php_namespace("App\\Service"));
        assert!(valid_php_namespace("\\App\\Service\\"));
        assert!(!valid_php_namespace("App\\Bad-Name"));
        assert_eq!(
            relative_directory_label(
                Some(Path::new("/workspace")),
                Path::new("/workspace/App/Service")
            ),
            "App/Service"
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavigationQueryKind {
    References,
    Implementations,
}

impl NavigationQueryKind {
    fn title(self) -> &'static str {
        match self {
            Self::References => "Find Usages",
            Self::Implementations => "Go to Implementation",
        }
    }
}

/// Runs only on the existing GPUI background executor. Each result file is read once.
fn prepare_find_usages(
    snapshot: &axiom_index::SemanticSnapshot,
    path: &Path,
    offset: usize,
    mut texts: HashMap<axiom_index::PersistentFileKey, String>,
) -> Result<(Vec<FindUsageTarget>, axiom_index::FindUsagesStatus), &'static str> {
    let source_key = axiom_index::PersistentFileKey::workspace_lexical(path);
    if !texts
        .get(&source_key)
        .is_some_and(|text| snapshot.matches_file_text(&source_key, text))
    {
        return Err("Find Usages: current buffer is not indexed yet; retry after indexing");
    }
    // Do not silently omit dirty references that have not reached the snapshot yet.
    for (key, text) in &texts {
        if (snapshot.file_id(key).is_some() || axiom_project::is_php_file(&key.normalized_path))
            && !snapshot.matches_file_text(key, text)
        {
            return Err("Find Usages: an open buffer is not indexed yet; retry after indexing");
        }
    }
    let mut result = snapshot.find_usages_at(path, offset, FindUsagesOptions::default());
    if result.usages.is_empty() && result.status != axiom_index::FindUsagesStatus::Complete {
        return Err("Find Usages: unresolved, ambiguous or unsupported symbol");
    }
    let mut targets = Vec::with_capacity(result.usages.len());
    result.usages.sort_by(|a, b| {
        a.file
            .normalized_path
            .cmp(&b.file.normalized_path)
            .then_with(|| a.span.start.cmp(&b.span.start))
            .then_with(|| a.span.end.cmp(&b.span.end))
    });
    let mut previous_file = None;
    let mut previous_offset = 0;
    let mut previous_position = lsp_types::Position::default();
    for usage in result.usages {
        let file = PathBuf::from(&usage.file.normalized_path);
        if !texts.contains_key(&usage.file) {
            let text = fs::read_to_string(&file)
                .map_err(|_| "Reference file unavailable; retry after indexing")?;
            if !snapshot.matches_file_text(&usage.file, &text) {
                return Err("Reference file changed; retry after indexing");
            }
            texts.insert(usage.file.clone(), text);
        }
        let text = &texts[&usage.file];
        if text.get(usage.span.clone()).is_none() {
            return Err("Reference range is stale; retry after indexing");
        }
        if previous_file.as_ref() != Some(&usage.file) {
            previous_offset = 0;
            previous_position = lsp_types::Position::default();
        }
        // Sorted ranges permit one forward pass per file, including UTF-16 columns.
        let delta = PositionCodec::offset_to_position(
            &text[previous_offset..],
            usage.span.start - previous_offset,
            Default::default(),
        );
        let position = lsp_types::Position::new(
            previous_position.line + delta.line,
            if delta.line == 0 {
                previous_position.character + delta.character
            } else {
                delta.character
            },
        );
        previous_file = Some(usage.file);
        previous_offset = usage.span.start;
        previous_position = position;
        let display = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document");
        let label = format!(
            "{}:{}:{}",
            display,
            position.line + 1,
            position.character + 1
        );
        let snippet = text[text[..usage.span.start].rfind('\n').map_or(0, |i| i + 1)..]
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        targets.push(FindUsageTarget {
            file,
            span: usage.span,
            role: Some(usage.role),
            position,
            label,
            snippet,
        });
    }
    targets.dedup_by(|a, b| a.file == b.file && a.span == b.span);
    Ok((targets, result.status))
}

fn prepare_implementations(
    snapshot: &axiom_index::SemanticSnapshot,
    path: &Path,
    offset: usize,
) -> Result<Vec<FindUsageTarget>, &'static str> {
    let ids = snapshot
        .implementation_targets_at(path, offset)
        .ok_or("No semantic implementation target under caret")?;
    let mut targets = Vec::with_capacity(ids.len());
    for id in ids {
        let symbol = snapshot
            .symbol(id)
            .ok_or("Implementation symbol became stale")?;
        let file = snapshot
            .file(symbol.file)
            .ok_or("Implementation file unavailable")?;
        let text = fs::read_to_string(&file.path).map_err(|_| "Implementation file unavailable")?;
        let position =
            PositionCodec::offset_to_position(&text, symbol.range.start, Default::default());
        let display = file
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document");
        targets.push(FindUsageTarget {
            file: file.path.clone(),
            span: symbol.range.clone(),
            role: None,
            position,
            label: format!(
                "{}:{}:{}",
                display,
                position.line + 1,
                position.character + 1
            ),
            snippet: text[text[..symbol.range.start].rfind('\n').map_or(0, |i| i + 1)..]
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned(),
        });
    }
    targets.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.span.start.cmp(&b.span.start))
    });
    Ok(targets)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefinitionSource {
    Semantic,
    LegacyProject,
    LegacyVendor,
    LegacyRuntime,
    Lsp,
    Unresolved,
}

#[derive(Clone, Copy)]
enum SemanticDefinitionRoute {
    Unavailable,
    Resolved,
    Outcome(axiom_index::SemanticDefinitionOutcome),
}

fn vendor_allowed_for_route(route: SemanticDefinitionRoute) -> bool {
    matches!(
        route,
        SemanticDefinitionRoute::Unavailable
            | SemanticDefinitionRoute::Outcome(
                axiom_index::SemanticDefinitionOutcome::DeferredVendor
            )
    )
}

fn definition_cache_lookup(
    cache: &HashMap<String, DefinitionTarget>,
    key: &str,
) -> Option<DefinitionTarget> {
    cache
        .get(key)
        .filter(|target| target.path.is_file())
        .cloned()
}

#[derive(Clone)]
struct ExplorerItem {
    path: PathBuf,
    name: String,
    kind: EntryKind,
    depth: usize,
}

#[derive(Clone)]
struct ExplorerContext {
    path: PathBuf,
    kind: EntryKind,
}

enum ExplorerOperation {
    NewFile(PathBuf),
    NewPhpFile(PathBuf),
    NewPhp {
        directory: PathBuf,
        keyword: &'static str,
    },
    NewDirectory(PathBuf),
    Rename(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModalField {
    Name,
    Namespace,
    File,
    Extends,
    Implements,
}

enum ExplorerFsResult {
    Create(Option<PathBuf>),
    Rename { old: PathBuf, new: PathBuf },
    Delete(PathBuf),
}

#[derive(Clone, Copy, Debug)]
enum NewItemKind {
    File,
    Directory,
    PhpFile,
    PhpClass,
    PhpInterface,
    PhpTrait,
    PhpEnum,
}

enum RuntimeStubStatus {
    Loading,
    Loaded { files: usize, symbols: usize },
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuKind {
    File,
    Edit,
    Code,
    View,
    Navigate,
    Help,
}

enum PendingOperation {
    OpenProject(PathBuf),
    CloseProject,
    Exit,
}

type ProjectLoadPayload = (Project, Vec<ProjectEntry>, Arc<LspBridge>);
type SemanticIndexPayload = (ProjectSymbolIndex, Arc<SemanticEngine>);
type RuntimeLoadResult = (
    u64,
    Result<(RuntimeStubStatus, Arc<RuntimeSymbolIndex>), String>,
);

fn semantic_update_matches(
    current_project_generation: u64,
    update_project_generation: u64,
) -> bool {
    current_project_generation == update_project_generation
}

impl RuntimeStubStatus {
    fn label(&self) -> String {
        match self {
            Self::Loading => "Loading...".to_owned(),
            Self::Loaded { files, symbols } => {
                format!("Loaded ({files} files, {symbols} symbols)")
            }
            Self::NotFound => "Not Found".to_owned(),
        }
    }
}

pub struct WorkspaceView {
    project: Option<Project>,
    explorer: Vec<ExplorerItem>,
    expanded: HashSet<PathBuf>,
    tabs: Vec<OpenTab>,
    active: Option<usize>,
    focus: FocusHandle,
    status: SharedString,
    definition_loading: bool,
    definition_loading_tick: u8,
    lsp: Option<std::sync::Arc<LspBridge>>,
    runtime_stubs: RuntimeStubStatus,
    runtime_stub_path: PathBuf,
    runtime_stub_cache_path: Option<PathBuf>,
    runtime_load_generation: u64,
    runtime_load_results: Option<Receiver<RuntimeLoadResult>>,
    runtime_watch_events: Option<Receiver<()>>,
    runtime_watch_stop: Arc<AtomicBool>,
    // Retained independently from the editor/LSP; native completion is deliberately out of scope.
    _runtime_symbols: Option<std::sync::Arc<RuntimeSymbolIndex>>,
    recent_projects: RecentProjects,
    recent_path: Option<PathBuf>,
    open_menu: Option<MenuKind>,
    menu_anchor_x: Pixels,
    pending_operation: Option<PendingOperation>,
    show_about: bool,
    startup_file: Option<PathBuf>,
    explorer_context: Option<ExplorerContext>,
    context_menu_position: Point<Pixels>,
    context_menu_selected: usize,
    context_submenu_selected: usize,
    selected_path: Option<PathBuf>,
    explorer_new_menu_open: bool,
    explorer_operation: Option<ExplorerOperation>,
    explorer_input: String,
    explorer_file: String,
    explorer_file_auto: bool,
    explorer_modal_field: ModalField,
    explorer_namespace_selection: UTF16Selection,
    explorer_file_selection: UTF16Selection,
    explorer_extends_selection: UTF16Selection,
    explorer_implements_selection: UTF16Selection,
    explorer_namespace: String,
    explorer_extends: String,
    explorer_implements: String,
    modal_inputs: [Entity<ModalInput>; 5],
    modal_type_items: Vec<lsp_types::CompletionItem>,
    modal_type_selected: usize,
    modal_type_range: std::ops::Range<usize>,
    modal_type_scroll: ScrollHandle,
    modal_field_focus: [FocusHandle; 5],
    modal_field_geometry: [crate::ui::input_line::InputGeometry; 5],
    modal_input_focus: FocusHandle,
    modal_caret_visible: bool,
    modal_caret_activity: Instant,
    modal_caret_toggle: Instant,
    modal_focus_pending: bool,
    delete_focus_pending: bool,
    explorer_selection: UTF16Selection,
    explorer_scroll: ScrollHandle,
    explorer_scroll_dragging: bool,
    explorer_scroll_drag_start_y: f32,
    explorer_scroll_drag_start_offset: f32,
    explorer_scroll_hovered: bool,
    project_panel_width: Pixels,
    project_panel_resizing: bool,
    project_panel_resize_start_x: f32,
    project_panel_resize_start_width: f32,
    explorer_undo: Vec<(String, UTF16Selection)>,
    pending_delete: Option<PathBuf>,
    pending_delete_is_directory: bool,
    project_panel_visible: bool,
    terminal_session: Option<std::sync::Arc<TerminalSession>>,
    terminal_view: Option<Entity<TerminalView>>,
    terminal_visible: bool,
    navigation_back: Vec<NavigationLocation>,
    navigation_forward: Vec<NavigationLocation>,
    definition_targets: Vec<DefinitionTarget>,
    definition_cache: HashMap<String, DefinitionTarget>,
    project_index: Option<Arc<RwLock<ProjectSymbolIndex>>>,
    index_generation: u64,
    index_results: Option<Receiver<(u64, Result<SemanticIndexPayload, String>)>>,
    semantic_engine: Option<Arc<SemanticEngine>>,
    semantic_update_sender: mpsc::Sender<(u64, PathBuf, String)>,
    semantic_update_receiver: Receiver<(u64, PathBuf, String)>,
    semantic_update_generation: u64,
    project_semantic_generation: u64,
    semantic_update_results: Option<Receiver<(u64, u64, Result<Arc<SemanticSnapshot>, String>)>>,
    pending_semantic_fs_changes: Vec<(PathBuf, Option<(PathBuf, String)>)>,
    vendor_index: Option<Arc<RwLock<VendorSymbolIndex>>>,
    vendor_index_generation: u64,
    vendor_index_results: Option<Receiver<(u64, Result<VendorSymbolIndex, String>)>>,
    indexing_phase: u8,
    keymap: Keymap,
    command_palette_visible: bool,
    command_palette_query: String,
    command_palette_selected: usize,
    command_palette_mode: Option<String>,
    features_visible: bool,
    settings_visible: bool,
    settings_query: String,
    settings_selected: Option<String>,
    shortcut_capture: bool,
    captured_shortcut: Option<String>,
    shortcut_conflict: Option<String>,
    debug_overlay_visible: bool,
    focus_active_editor: bool,
    project_dialog_open: bool,
    project_load_generation: u64,
    project_load_results: Option<Receiver<(u64, Result<ProjectLoadPayload, String>)>>,
    lsp_generations: HashMap<(lsp_types::Uri, LspRequestKind), u64>,
    explorer_fs_busy: bool,
    vendor_definition_inflight: HashSet<String>,
    find_usages: Vec<FindUsageTarget>,
    find_usages_visible: bool,
    find_usages_context: Option<Arc<FindUsagesContext>>,
    find_usages_selected: usize,
    find_usages_focus: FocusHandle,
    find_usages_focus_pending: bool,
    find_usages_scroll: gpui::UniformListScrollHandle,
    heartbeat_last_tick: Option<Instant>,
    heartbeat_summary_at: Instant,
    heartbeat_ticks: u64,
    heartbeat_over_10ms: u64,
    heartbeat_over_16ms: u64,
    heartbeat_over_50ms: u64,
    heartbeat_over_100ms: u64,
    heartbeat_over_1s: u64,
    heartbeat_worst_us: u128,
    key_event_id: u64,
    last_key_event_at: Option<Instant>,
}

impl WorkspaceView {
    fn current_text_for_path(&self, path: &Path, cx: &App) -> Option<(String, TargetTextSource)> {
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(tab) = self.tabs.iter().find(|tab| {
            let tab_path = fs::canonicalize(&tab.path).unwrap_or_else(|_| tab.path.clone());
            tab_path == canonical
        }) {
            return Some((
                tab.editor.read(cx).document_content(),
                TargetTextSource::Memory,
            ));
        }
        fs::read_to_string(path)
            .ok()
            .map(|text| (text, TargetTextSource::Disk))
    }

    fn render_command_palette(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let commands = self.palette_commands();
        let workspace = cx.entity();
        div()
            .absolute()
            .top(px(70.))
            .left(px(240.))
            .w(px(620.))
            .max_h(px(430.))
            .flex()
            .flex_col()
            .bg(t.popup_background)
            .border_1()
            .border_color(t.border)
            .rounded(m.border_radius_medium)
            .shadow_lg()
            .on_key_down(cx.listener(Self::handle_workspace_keydown))
            .key_context("CommandPalette")
            .child(
                div()
                    .h(px(38.))
                    .px_3()
                    .flex()
                    .items_center()
                    .child(if self.command_palette_query.is_empty() {
                        "Search commands...".to_owned()
                    } else {
                        self.command_palette_query.clone()
                    })
                    .child(WorkspaceInputElement {
                        workspace,
                        focus: self.focus.clone(),
                    }),
            )
            .children(commands.iter().enumerate().map(|(index, command)| {
                let selected = index == self.command_palette_selected;
                let workspace = cx.entity();
                div()
                    .id(SharedString::from(format!(
                        "palette-command-{}",
                        command.id
                    )))
                    .h(m.toolbar_height)
                    .px_3()
                    .flex()
                    .items_center()
                    .bg(if selected {
                        t.selection
                    } else {
                        t.popup_background
                    })
                    .text_color(t.text_primary)
                    .on_click(move |_, window, cx| {
                        workspace.update(cx, |this, cx| {
                            this.command_palette_selected = index;
                            this.palette_confirm(&PaletteConfirm, window, cx);
                        });
                    })
                    .child(command.title.clone())
                    .child(
                        div().ml_auto().text_color(t.text_muted).child(
                            self.keymap
                                .shortcut(&command.id)
                                .unwrap_or("None")
                                .to_owned(),
                        ),
                    )
            }))
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let close_workspace = workspace.clone();
        div()
            .absolute()
            .top(px(42.))
            .left(px(180.))
            .right(px(24.))
            .bottom(px(24.))
            .flex()
            .flex_col()
            .bg(t.window_background)
            .border_1()
            .border_color(t.border)
            .rounded(m.border_radius_medium)
            .shadow_lg()
            .child(
                div()
                    .h(px(42.))
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .child("Settings")
                    .child(div().id("close-settings").px_2().child("×").on_click(
                        move |_, window, cx| {
                            close_workspace.update(cx, |this, cx| {
                                this.settings_visible = false;
                                this.restore_editor_focus(window, cx);
                                cx.notify();
                            });
                        },
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .child(
                        div()
                            .w(px(180.))
                            .p_3()
                            .bg(t.panel_background)
                            .child("Keymap")
                            .child(div().mt_2().text_color(t.text_muted).child("PHP"))
                            .child(div().text_color(t.text_muted).child("Runtime Stubs"))
                            .child(div().text_color(t.text_muted).child("Formatter")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .p_4()
                            .child(
                                div()
                                    .h(px(34.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .bg(t.panel_background)
                                    .text_color(t.text_muted)
                                    .id("settings-search")
                                    .on_click({
                                        let workspace = workspace.clone();
                                        move |_, window, cx| {
                                            workspace.update(cx, |this, cx| {
                                                window.focus(&this.focus);
                                                cx.notify();
                                            });
                                        }
                                    })
                                    .child(if self.settings_query.is_empty() {
                                        "Search actions...".to_owned()
                                    } else {
                                        self.settings_query.clone()
                                    })
                                    .child(WorkspaceInputElement {
                                        workspace: workspace.clone(),
                                        focus: self.focus.clone(),
                                    }),
                            )
                            .child(
                                div()
                                    .mt_3()
                                    .p_2()
                                    .bg(t.panel_background)
                                    .child(format!(
                                        "Runtime Stubs Directory: {}",
                                        self.runtime_stub_path.display()
                                    ))
                                    .child(format!("Status: {}", self.runtime_stubs.label()))
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .mt_2()
                                            .child(
                                                div()
                                                    .id("runtime-stubs-open")
                                                    .px_2()
                                                    .child("Open Folder")
                                                    .on_click({
                                                        let workspace = workspace.clone();
                                                        move |_, _, cx| {
                                                            workspace.update(cx, |this, cx| {
                                                                let _ = fs::create_dir_all(
                                                                    &this.runtime_stub_path,
                                                                );
                                                                let _ = open::that(
                                                                    &this.runtime_stub_path,
                                                                );
                                                                cx.notify();
                                                            });
                                                        }
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .id("runtime-stubs-import-files")
                                                    .px_2()
                                                    .child("Import Files…")
                                                    .on_click({
                                                        let workspace = workspace.clone();
                                                        move |_, _, cx| {
                                                            workspace.update(cx, |this, cx| {
                                                                this.import_runtime_stub_files(cx)
                                                            });
                                                        }
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .id("runtime-stubs-import")
                                                    .px_2()
                                                    .child("Import…")
                                                    .on_click({
                                                        let workspace = workspace.clone();
                                                        move |_, _, cx| {
                                                            workspace.update(cx, |this, cx| {
                                                                this.import_runtime_stubs(cx)
                                                            });
                                                        }
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .id("runtime-stubs-reload")
                                                    .px_2()
                                                    .child("Reload")
                                                    .on_click({
                                                        let workspace = workspace.clone();
                                                        move |_, _, cx| {
                                                            workspace.update(cx, |this, cx| {
                                                                this.reload_runtime_stubs(cx)
                                                            });
                                                        }
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .id("runtime-stubs-clear-cache")
                                                    .px_2()
                                                    .child("Clear Cache")
                                                    .on_click({
                                                        let workspace = workspace.clone();
                                                        move |_, _, cx| {
                                                            workspace.update(cx, |this, cx| {
                                                                this.clear_runtime_stub_cache(cx)
                                                            });
                                                        }
                                                    }),
                                            ),
                                    ),
                            )
                            .children(self.keymap.search(&self.settings_query).into_iter().map(
                                |command| {
                                    let workspace = cx.entity();
                                    let selected = self.settings_selected.as_deref()
                                        == Some(command.id.as_str());
                                    let command_id = command.id.clone();
                                    div()
                                        .id(SharedString::from(format!("keymap-{}", command.id)))
                                        .h(m.toolbar_height)
                                        .flex()
                                        .items_center()
                                        .px_2()
                                        .bg(if selected {
                                            t.selection
                                        } else {
                                            t.window_background
                                        })
                                        .on_click(move |_, _, cx| {
                                            workspace.update(cx, |this, cx| {
                                                this.select_setting_command(command_id.clone(), cx)
                                            })
                                        })
                                        .child(command.title.clone())
                                        .child(
                                            div().ml_auto().text_color(t.text_muted).child(
                                                self.keymap
                                                    .shortcut(&command.id)
                                                    .unwrap_or("None")
                                                    .to_owned(),
                                            ),
                                        )
                                },
                            ))
                            .when_some(
                                self.settings_selected.as_ref().and_then(|id| {
                                    self.keymap
                                        .commands()
                                        .iter()
                                        .find(|command| &command.id == id)
                                }),
                                |this, command| this.child(self.render_keymap_details(command, cx)),
                            ),
                    ),
            )
    }

    fn render_keymap_details(
        &self,
        command: &axiom_app::commands::CommandDescriptor,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme();
        let workspace = cx.entity();
        div()
            .mt_4()
            .p_3()
            .bg(t.panel_background)
            .child(command.title.clone())
            .child(
                div()
                    .text_color(t.text_muted)
                    .child(command.description.clone()),
            )
            .child(format!("Category: {}", command.category))
            .child(format!(
                "Current Shortcut: {}",
                self.keymap.shortcut(&command.id).unwrap_or("None")
            ))
            .child(format!(
                "Default Shortcut: {}",
                command.default_shortcut.as_deref().unwrap_or("None")
            ))
            .child(
                div()
                    .mt_3()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("edit-shortcut")
                            .px_2()
                            .child("Edit Shortcut")
                            .on_click({
                                let workspace = workspace.clone();
                                move |_, _, cx| {
                                    workspace.update(cx, |this, cx| this.begin_shortcut_capture(cx))
                                }
                            }),
                    )
                    .child(
                        div()
                            .id("remove-shortcut")
                            .px_2()
                            .child("Remove Shortcut")
                            .on_click({
                                let workspace = workspace.clone();
                                move |_, _, cx| {
                                    workspace
                                        .update(cx, |this, cx| this.remove_selected_shortcut(cx))
                                }
                            }),
                    )
                    .child(
                        div()
                            .id("reset-shortcut")
                            .px_2()
                            .child("Reset to Default")
                            .on_click(move |_, _, cx| {
                                workspace.update(cx, |this, cx| this.reset_selected_shortcut(cx))
                            }),
                    ),
            )
            .when(self.shortcut_capture, |this| {
                this.child(format!(
                    "Press new keyboard shortcut: {}",
                    self.captured_shortcut.as_deref().unwrap_or("…")
                ))
            })
            .when_some(self.captured_shortcut.as_ref(), |this, _| {
                this.child(div().id("apply-shortcut").px_2().child("Apply").on_click({
                    let workspace = cx.entity();
                    move |_, _, cx| {
                        workspace.update(cx, |this, cx| this.apply_captured_shortcut(cx))
                    }
                }))
            })
            .when_some(self.shortcut_conflict.as_ref(), |this, conflict| {
                let workspace = cx.entity();
                this.child(div().text_color(t.error).child(conflict.clone()))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("replace-shortcut-conflict")
                                    .px_2()
                                    .child("Replace")
                                    .on_click({
                                        let workspace = workspace.clone();
                                        move |_, _, cx| {
                                            workspace.update(cx, |this, cx| {
                                                this.replace_conflicting_shortcut(cx)
                                            })
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .id("cancel-shortcut-conflict")
                                    .px_2()
                                    .child("Cancel")
                                    .on_click(move |_, _, cx| {
                                        workspace.update(cx, |this, cx| {
                                            this.cancel_shortcut_conflict(cx)
                                        })
                                    }),
                            ),
                    )
            })
    }

    pub fn new(startup: StartupTarget, cx: &mut Context<Self>) -> Self {
        let owner = cx.entity().downgrade();
        let modal_inputs = [
            ModalField::Name,
            ModalField::Namespace,
            ModalField::File,
            ModalField::Extends,
            ModalField::Implements,
        ]
        .map(|field| {
            cx.new(|_| ModalInput {
                owner: owner.clone(),
                field,
            })
        });
        let modal_field_focus: [FocusHandle; 5] = std::array::from_fn(|_| cx.focus_handle());
        let modal_input_focus = modal_field_focus[0].clone();
        let recent_path = recent_projects_path();
        let recent_projects = recent_path
            .as_deref()
            .map(RecentProjects::load)
            .unwrap_or_default();
        let keymap = Keymap::load_user();
        let (semantic_update_sender, semantic_update_receiver) = mpsc::channel();
        if debug_input_enabled() {
            tracing::info!(
                command = "project.rename",
                shortcut = ?keymap.shortcut("project.rename"),
                "[KEYMAP EFFECTIVE]"
            );
        }
        let mut workspace = Self {
            project: None,
            explorer: Vec::new(),
            expanded: HashSet::new(),
            tabs: Vec::new(),
            active: None,
            focus: cx.focus_handle(),
            status: "Abra um arquivo no painel Project".into(),
            definition_loading: false,
            definition_loading_tick: 0,
            lsp: None,
            runtime_stubs: RuntimeStubStatus::Loading,
            runtime_stub_path: Self::runtime_stub_path(),
            runtime_stub_cache_path: runtime_stubs_cache_path(),
            _runtime_symbols: None,
            runtime_load_generation: 0,
            runtime_load_results: None,
            runtime_watch_events: None,
            runtime_watch_stop: Arc::new(AtomicBool::new(false)),
            recent_projects,
            recent_path,
            open_menu: None,
            menu_anchor_x: px(0.),
            pending_operation: None,
            show_about: false,
            startup_file: None,
            explorer_context: None,
            context_menu_position: Point::default(),
            context_menu_selected: 0,
            context_submenu_selected: 0,
            selected_path: None,
            explorer_new_menu_open: false,
            explorer_operation: None,
            explorer_input: String::new(),
            explorer_file: String::new(),
            explorer_file_auto: true,
            explorer_modal_field: ModalField::Name,
            modal_type_items: Vec::new(),
            modal_type_selected: 0,
            modal_type_range: 0..0,
            modal_type_scroll: ScrollHandle::new(),
            explorer_namespace_selection: UTF16Selection {
                range: 0..0,
                reversed: false,
            },
            explorer_file_selection: UTF16Selection {
                range: 0..0,
                reversed: false,
            },
            explorer_extends_selection: UTF16Selection {
                range: 0..0,
                reversed: false,
            },
            explorer_implements_selection: UTF16Selection {
                range: 0..0,
                reversed: false,
            },
            explorer_namespace: String::new(),
            explorer_extends: String::new(),
            explorer_implements: String::new(),
            modal_inputs,
            modal_field_focus,
            modal_field_geometry: Default::default(),
            modal_input_focus,
            modal_caret_visible: true,
            modal_caret_activity: Instant::now(),
            modal_caret_toggle: Instant::now(),
            modal_focus_pending: false,
            delete_focus_pending: false,
            explorer_selection: UTF16Selection {
                range: 0..0,
                reversed: false,
            },
            explorer_scroll: ScrollHandle::new(),
            explorer_scroll_dragging: false,
            explorer_scroll_drag_start_y: 0.0,
            explorer_scroll_drag_start_offset: 0.0,
            explorer_scroll_hovered: false,
            project_panel_width: px(244.),
            project_panel_resizing: false,
            project_panel_resize_start_x: 0.0,
            project_panel_resize_start_width: 244.0,
            explorer_undo: Vec::new(),
            pending_delete: None,
            pending_delete_is_directory: false,
            project_panel_visible: true,
            terminal_session: None,
            terminal_view: None,
            terminal_visible: false,
            navigation_back: Vec::new(),
            navigation_forward: Vec::new(),
            definition_targets: Vec::new(),
            definition_cache: HashMap::new(),
            project_index: None,
            index_generation: 0,
            index_results: None,
            semantic_engine: None,
            semantic_update_sender,
            semantic_update_receiver,
            semantic_update_generation: 0,
            project_semantic_generation: 0,
            semantic_update_results: None,
            pending_semantic_fs_changes: Vec::new(),
            vendor_index: None,
            vendor_index_generation: 0,
            vendor_index_results: None,
            indexing_phase: 0,
            keymap,
            command_palette_visible: false,
            command_palette_query: String::new(),
            command_palette_selected: 0,
            command_palette_mode: None,
            features_visible: false,
            settings_visible: false,
            settings_query: String::new(),
            settings_selected: None,
            shortcut_capture: false,
            captured_shortcut: None,
            shortcut_conflict: None,
            debug_overlay_visible: false,
            focus_active_editor: false,
            project_dialog_open: false,
            project_load_generation: 0,
            project_load_results: None,
            lsp_generations: HashMap::new(),
            explorer_fs_busy: false,
            vendor_definition_inflight: HashSet::new(),
            find_usages: Vec::new(),
            find_usages_visible: false,
            find_usages_context: None,
            find_usages_selected: 0,
            find_usages_focus: cx.focus_handle(),
            find_usages_focus_pending: false,
            find_usages_scroll: gpui::UniformListScrollHandle::new(),
            heartbeat_last_tick: None,
            heartbeat_summary_at: Instant::now(),
            heartbeat_ticks: 0,
            heartbeat_over_10ms: 0,
            heartbeat_over_16ms: 0,
            heartbeat_over_50ms: 0,
            heartbeat_over_100ms: 0,
            heartbeat_over_1s: 0,
            heartbeat_worst_us: 0,
            key_event_id: 0,
            last_key_event_at: None,
        };
        workspace.begin_runtime_stub_load(cx, false);
        workspace.start_runtime_watcher(cx);
        if let StartupTarget::Project { root, initial_file } = startup {
            workspace.begin_open_project(root, cx);
            workspace.startup_file = initial_file;
        }
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(std::time::Duration::from_millis(100)).await;
                if this
                    .update(cx, |this, cx| {
                        let _stage = crate::editor_view::UiStageGuard::new(
                            crate::editor_view::UI_STAGE_POLL_CYCLE,
                        );
                        let cycle_started = Instant::now();
                        if this.explorer_operation.is_some()
                            && crate::ui::input_line::blink_due(
                                cycle_started,
                                this.modal_caret_activity,
                                this.modal_caret_toggle,
                            )
                        {
                            this.modal_caret_visible = !this.modal_caret_visible;
                            this.modal_caret_toggle = cycle_started;
                            cx.notify();
                        }
                        let poll_started = Instant::now();
                        this.poll_lsp(cx);
                        let lsp_us = poll_started.elapsed().as_micros();
                        if this.index_results.is_some() {
                            this.indexing_phase = this.indexing_phase.wrapping_add(6) % 100;
                            cx.notify();
                        }
                        if this.definition_loading {
                            this.definition_loading_tick =
                                this.definition_loading_tick.wrapping_add(1) % 4;
                            cx.notify();
                        }
                        let poll_started = Instant::now();
                        this.poll_index(cx);
                        let index_us = poll_started.elapsed().as_micros();
                        let poll_started = Instant::now();
                        this.poll_vendor_index(cx);
                        let vendor_us = poll_started.elapsed().as_micros();
                        let poll_started = Instant::now();
                        this.poll_project_load(cx);
                        let project_load_us = poll_started.elapsed().as_micros();
                        let poll_started = Instant::now();
                        this.poll_runtime_stub_load(cx);
                        let runtime_stub_us = poll_started.elapsed().as_micros();
                        let poll_started = Instant::now();
                        this.poll_runtime_watcher(cx);
                        let watcher_us = poll_started.elapsed().as_micros();
                        let poll_started = Instant::now();
                        this.poll_semantic_updates(cx);
                        let semantic_us = poll_started.elapsed().as_micros();
                        let total_us = cycle_started.elapsed().as_micros();
                        if debug_ui_stall_enabled()
                            && (total_us >= 5_000
                                || [
                                    lsp_us,
                                    index_us,
                                    vendor_us,
                                    project_load_us,
                                    runtime_stub_us,
                                    watcher_us,
                                    semantic_us,
                                ]
                                .into_iter()
                                .any(|us| us >= 3_000))
                        {
                            tracing::info!(target: "axiom.ui_stall",
                                total_us,
                                lsp_us,
                                index_us,
                                vendor_us,
                                project_load_us,
                                runtime_stub_us,
                                watcher_us,
                                semantic_us,
                                "[UI POLL CYCLE]"
                            );
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let expected_interval = std::time::Duration::from_millis(50);
            loop {
                Timer::after(expected_interval).await;
                if this
                    .update(cx, |this, _| {
                        let now = Instant::now();
                        crate::editor_view::LAST_UI_HEARTBEAT_NS
                            .store(crate::editor_view::ui_clock_ns(), Ordering::Relaxed);
                        let lateness_us = this
                            .heartbeat_last_tick
                            .map(|last| {
                                now.duration_since(last)
                                    .saturating_sub(expected_interval)
                                    .as_micros()
                            })
                            .unwrap_or_default();
                        this.heartbeat_last_tick = Some(now);
                        this.heartbeat_ticks = this.heartbeat_ticks.saturating_add(1);
                        this.heartbeat_worst_us = this.heartbeat_worst_us.max(lateness_us);
                        if lateness_us >= 10_000 {
                            this.heartbeat_over_10ms += 1;
                            if lateness_us >= 16_000 {
                                this.heartbeat_over_16ms += 1;
                            }
                            if lateness_us >= 50_000 {
                                this.heartbeat_over_50ms += 1;
                            }
                            if lateness_us >= 100_000 {
                                this.heartbeat_over_100ms += 1;
                            }
                            if lateness_us >= 1_000_000 {
                                this.heartbeat_over_1s += 1;
                            }
                            if debug_ui_stall_enabled() {
                                let ms_since_last_key_event = this
                                    .last_key_event_at
                                    .map(|at| now.duration_since(at).as_millis())
                                    .unwrap_or_default();
                                tracing::info!(target: "axiom.ui_stall",
                                    lateness_us,
                                    actual_interval_us = lateness_us + expected_interval.as_micros(),
                                    severity = if lateness_us >= 1_000_000 { "freeze" } else if lateness_us >= 100_000 { "major" } else { "stall" },
                                    last_key_event_id = this.key_event_id,
                                    ms_since_last_key_event,
                                    edit_generation = crate::editor_view::LAST_UI_EDIT_GENERATION.load(Ordering::Relaxed),
                                    "[UI EVENT LOOP STALL]"
                                );
                            }
                        }
                        if debug_ui_stall_enabled()
                            && now.duration_since(this.heartbeat_summary_at)
                                >= std::time::Duration::from_secs(10)
                        {
                            tracing::info!(target: "axiom.ui_stall",
                                ticks = this.heartbeat_ticks,
                                over_10ms = this.heartbeat_over_10ms,
                                over_16ms = this.heartbeat_over_16ms,
                                over_50ms = this.heartbeat_over_50ms,
                                over_100ms = this.heartbeat_over_100ms,
                                over_1s = this.heartbeat_over_1s,
                                worst_us = this.heartbeat_worst_us,
                                caret_tasks_alive = crate::editor_view::CARET_TASKS_ACTIVE.load(Ordering::Relaxed),
                                caret_tasks_started = crate::editor_view::CARET_TASKS_STARTED.load(Ordering::Relaxed),
                                "[UI EVENT LOOP SUMMARY]"
                            );
                            this.heartbeat_summary_at = now;
                            this.heartbeat_ticks = 0;
                            this.heartbeat_over_10ms = 0;
                            this.heartbeat_over_16ms = 0;
                            this.heartbeat_over_50ms = 0;
                            this.heartbeat_over_100ms = 0;
                            this.heartbeat_over_1s = 0;
                            this.heartbeat_worst_us = 0;
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        if debug_ui_stall_enabled() {
            std::thread::spawn(|| {
                let mut last_report_ns = 0u64;
                let mut last_severity = 0u8;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let now = crate::editor_view::ui_clock_ns();
                    let heartbeat =
                        crate::editor_view::LAST_UI_HEARTBEAT_NS.load(Ordering::Relaxed);
                    if heartbeat == 0 {
                        continue;
                    }
                    let blocked_for_us = now.saturating_sub(heartbeat) / 1_000;
                    let severity = if blocked_for_us >= 1_000_000 { 3 } else { 0 };
                    if severity == 0 {
                        last_severity = 0;
                        continue;
                    }
                    if severity != last_severity
                        || now.saturating_sub(last_report_ns) >= 1_000_000_000
                    {
                        let stage = crate::editor_view::UI_STAGE.load(Ordering::Relaxed);
                        let entered =
                            crate::editor_view::UI_STAGE_ENTERED_AT_NS.load(Ordering::Relaxed);
                        tracing::info!(target: "axiom.ui_stall",
                            blocked_for_us,
                            severity = if severity >= 3 { "freeze" } else { "major" },
                            ui_stage = crate::editor_view::ui_stage_name(stage),
                            stage_elapsed_us = now.saturating_sub(entered) / 1_000,
                            last_key_event_id = crate::editor_view::LAST_UI_KEY_EVENT_ID.load(Ordering::Relaxed),
                            edit_generation = crate::editor_view::LAST_UI_EDIT_GENERATION.load(Ordering::Relaxed),
                            last_rendered_generation = crate::editor_view::LAST_UI_RENDERED_GENERATION.load(Ordering::Relaxed),
                            "[UI WATCHDOG STALL]"
                        );
                        last_report_ns = now;
                        last_severity = severity;
                    }
                }
            });
        }
        workspace
    }

    #[allow(dead_code)]
    fn load_runtime_stubs() -> (
        RuntimeStubStatus,
        Option<std::sync::Arc<RuntimeSymbolIndex>>,
    ) {
        let provider = StubProvider::from_env()
            .unwrap_or_else(|| StubProvider::new(Self::runtime_stub_path()));
        let configured_path = provider.root().to_path_buf();
        let _ = fs::create_dir_all(&configured_path);
        let cache = runtime_stubs_cache_path();
        let result = cache
            .as_deref()
            .map_or_else(|| provider.load(), |cache| provider.load_incremental(cache));
        match result {
            Ok((index, report)) => {
                if debug_stubs_enabled() {
                    tracing::info!(
                        configured_path = %configured_path.display(),
                        exists = configured_path.is_dir(),
                        files = report.files_parsed,
                        symbols = report.symbols_indexed,
                        load_errors = report.errors.len(),
                        "[RUNTIME STUBS]"
                    );
                }
                (
                    RuntimeStubStatus::Loaded {
                        files: report.files_discovered,
                        symbols: report.symbols_indexed,
                    },
                    Some(std::sync::Arc::new(index)),
                )
            }
            Err(error) => {
                if debug_stubs_enabled() {
                    tracing::info!(configured_path = %configured_path.display(), exists = configured_path.is_dir(), files = 0, symbols = 0, load_errors = 1, "[RUNTIME STUBS]");
                }
                tracing::warn!(%error, "PHP runtime stubs unavailable");
                (RuntimeStubStatus::NotFound, None)
            }
        }
    }

    fn runtime_stub_path() -> PathBuf {
        if let Some(path) =
            std::env::var_os("AXIOM_PHP_STUBS").or_else(|| std::env::var_os("RUSTSTORM_PHP_STUBS"))
        {
            return PathBuf::from(path);
        }
        runtime_stubs_default_path().unwrap_or_else(|| PathBuf::from("stubs"))
    }

    fn begin_runtime_stub_load(&mut self, cx: &mut Context<Self>, updating: bool) {
        self.runtime_load_generation = self.runtime_load_generation.wrapping_add(1);
        let generation = self.runtime_load_generation;
        let path = self.runtime_stub_path.clone();
        let cache = self.runtime_stub_cache_path.clone();
        let (sender, receiver) = mpsc::channel();
        self.runtime_load_results = Some(receiver);
        self.runtime_stubs = RuntimeStubStatus::Loading;
        self.status = if updating {
            "Runtime Stubs: Updating..."
        } else {
            "Runtime Stubs: Loading..."
        }
        .into();
        thread::spawn(move || {
            let provider = StubProvider::new(path);
            let result = cache
                .as_deref()
                .map_or_else(|| provider.load(), |cache| provider.load_incremental(cache))
                .map(|(index, report)| {
                    (
                        RuntimeStubStatus::Loaded {
                            files: report.files_discovered,
                            symbols: report.symbols_indexed,
                        },
                        Arc::new(index),
                    )
                })
                .map_err(|error| error.to_string());
            let _ = sender.send((generation, result));
        });
        cx.notify();
    }

    fn poll_runtime_stub_load(&mut self, cx: &mut Context<Self>) {
        let _stage =
            crate::editor_view::UiStageGuard::new(crate::editor_view::UI_STAGE_POLL_RUNTIME_STUB);
        let Some(receiver) = self.runtime_load_results.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.runtime_load_results = None;
                return;
            }
        };
        self.runtime_load_results = None;
        let (generation, result) = result;
        if generation != self.runtime_load_generation {
            return;
        }
        match result {
            Ok((status, symbols)) => {
                let count = match status {
                    RuntimeStubStatus::Loaded { symbols, .. } => symbols,
                    _ => 0,
                };
                self.runtime_stubs = status;
                self._runtime_symbols = Some(symbols.clone());
                for tab in &self.tabs {
                    tab.editor
                        .update(cx, |editor, _| editor.set_runtime_symbols(symbols.clone()));
                }
                self.status = format!("Runtime Stubs: Ready ({count} symbols)").into();
            }
            Err(error) => {
                self.runtime_stubs = RuntimeStubStatus::NotFound;
                self.status = format!("Runtime Stubs: Error — {error}").into();
            }
        }
        cx.notify();
    }

    fn start_runtime_watcher(&mut self, cx: &mut Context<Self>) {
        let root = self.runtime_stub_path.clone();
        let stop = self.runtime_watch_stop.clone();
        let (sender, receiver) = mpsc::channel();
        self.runtime_watch_events = Some(receiver);
        thread::spawn(move || {
            let mut previous = stub_snapshot(&root).unwrap_or_default();
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(std::time::Duration::from_millis(400));
                let current = stub_snapshot(&root).unwrap_or_default();
                if current != previous {
                    previous = current;
                    let _ = sender.send(());
                }
            }
        });
        cx.notify();
    }

    fn poll_runtime_watcher(&mut self, cx: &mut Context<Self>) {
        let _stage = crate::editor_view::UiStageGuard::new(
            crate::editor_view::UI_STAGE_POLL_RUNTIME_WATCHER,
        );
        let Some(receiver) = self.runtime_watch_events.as_ref() else {
            return;
        };
        if receiver.try_recv().is_ok() && self.runtime_load_results.is_none() {
            self.begin_runtime_stub_load(cx, true);
        }
    }

    fn begin_open_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.project_load_generation = self.project_load_generation.wrapping_add(1);
        let generation = self.project_load_generation;
        let (sender, receiver) = mpsc::channel();
        self.project_load_results = Some(receiver);
        self.status = "Opening project...".into();
        if debug_input_enabled() {
            tracing::info!(path = %path.display(), generation, "[PROJECT] open path");
            tracing::info!(name = "load_project_shell", "[PROJECT STEP START]");
        }
        thread::spawn(move || {
            let started = Instant::now();
            let result = (|| {
                let project = Project::open(&path).map_err(|error| error.to_string())?;
                let root = project.root_path().to_path_buf();
                let entries = project
                    .read_directory(&root)
                    .map_err(|error| error.to_string())?;
                let lsp = LspBridge::start(project.root_path());
                Ok((project, entries, lsp))
            })();
            if debug_input_enabled() {
                tracing::info!(
                    name = "load_project_shell",
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "[PROJECT STEP END]"
                );
            }
            let _ = sender.send((generation, result));
        });
        cx.notify();
    }

    fn poll_project_load(&mut self, cx: &mut Context<Self>) {
        let _stage =
            crate::editor_view::UiStageGuard::new(crate::editor_view::UI_STAGE_POLL_PROJECT);
        let Some(receiver) = self.project_load_results.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.project_load_results = None;
                return;
            }
        };
        self.project_load_results = None;
        let (generation, result) = result;
        if generation != self.project_load_generation {
            return;
        }
        match result {
            Ok((project, entries, lsp)) => {
                let started = Instant::now();
                self.finish_project_load(project, entries, lsp, cx);
                let elapsed_ms = started.elapsed().as_millis() as u64;
                if debug_input_enabled() && elapsed_ms > 50 {
                    tracing::warn!(
                        operation = "publish_project_shell",
                        elapsed_ms,
                        "[UI BLOCK WARNING]"
                    );
                }
                if debug_input_enabled() {
                    tracing::info!("[PROJECT] ready");
                }
            }
            Err(error) => {
                self.status = format!("Falha ao abrir projeto: {error}").into();
                if debug_input_enabled() {
                    tracing::warn!(%error, "[PROJECT] open failed");
                }
                cx.notify();
            }
        }
    }

    fn finish_project_load(
        &mut self,
        project: Project,
        entries: Vec<ProjectEntry>,
        lsp: Arc<LspBridge>,
        cx: &mut Context<Self>,
    ) {
        self.navigation_back.clear();
        self.navigation_forward.clear();
        self.project_semantic_generation = self.project_semantic_generation.wrapping_add(1);
        self.semantic_update_generation = 0;
        self.semantic_update_results = None;
        self.pending_semantic_fs_changes.clear();
        let (semantic_update_sender, semantic_update_receiver) = mpsc::channel();
        self.semantic_update_sender = semantic_update_sender;
        self.semantic_update_receiver = semantic_update_receiver;
        let root = project.root_path().to_path_buf();
        self.index_generation = self.index_generation.wrapping_add(1);
        let generation = self.index_generation;
        let (sender, receiver) = mpsc::channel();
        self.index_results = Some(receiver);
        self.project_index = None;
        self.semantic_engine = None;
        self.vendor_index = None;
        self.status = "Project opened — indexing...".into();
        let index_root = root.clone();
        thread::spawn(move || {
            let mut index = ProjectSymbolIndex::new();
            let result = project_symbol_cache_path(&index_root)
                .map(|cache| index.index_project_cached(&index_root, cache))
                .unwrap_or_else(|| index.index_project(&index_root))
                .map(|_| {
                    let snapshot = axiom_index::SemanticSnapshot::from_project_index(
                        &index,
                        SemanticRevision(generation),
                    );
                    (index, Arc::new(SemanticEngine::from_snapshot(snapshot)))
                })
                .map_err(|error| error.to_string());
            let _ = sender.send((generation, result));
        });
        self.vendor_index_generation = self.vendor_index_generation.wrapping_add(1);
        let vendor_generation = self.vendor_index_generation;
        let (vendor_sender, vendor_receiver) = mpsc::channel();
        self.vendor_index_results = Some(vendor_receiver);
        let vendor_root = root.clone();
        thread::spawn(move || {
            let result = composer_vendor_cache_path(&vendor_root)
                .map(|cache| VendorSymbolIndex::load_cached(&vendor_root, cache))
                .unwrap_or_else(|| VendorSymbolIndex::load(&vendor_root))
                .map_err(|error| error.to_string());
            let _ = vendor_sender.send((vendor_generation, result));
        });
        self.explorer = entries
            .into_iter()
            .map(|entry| ExplorerItem {
                path: entry.path,
                name: entry.name,
                kind: entry.kind,
                depth: 0,
            })
            .collect();
        self.expanded.clear();
        self.status = "Project opened — indexing...".into();
        self.project = Some(project);
        self.recent_projects.add(&root, unix_timestamp_now());
        if let Some(path) = &self.recent_path
            && let Err(error) = self.recent_projects.save(path)
        {
            tracing::warn!("failed to persist recent projects: {error}");
        }
        self.lsp = Some(lsp);
        cx.notify();
    }

    fn poll_vendor_index(&mut self, cx: &mut Context<Self>) {
        let _stage =
            crate::editor_view::UiStageGuard::new(crate::editor_view::UI_STAGE_POLL_VENDOR);
        let Some(receiver) = self.vendor_index_results.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.vendor_index_results = None;
                return;
            }
        };
        self.vendor_index_results = None;
        let (generation, result) = result;
        if generation != self.vendor_index_generation || self.project.is_none() {
            return;
        }
        match result {
            Ok(index) => {
                let shared = Arc::new(RwLock::new(index));
                self.vendor_index = Some(shared.clone());
                for tab in &self.tabs {
                    tab.editor
                        .update(cx, |editor, _| editor.set_vendor_symbols(shared.clone()));
                }
            }
            Err(error) => self.status = format!("Composer metadata unavailable: {error}").into(),
        }
        cx.notify();
    }

    fn poll_index(&mut self, cx: &mut Context<Self>) {
        let _stage = crate::editor_view::UiStageGuard::new(crate::editor_view::UI_STAGE_POLL_INDEX);
        let Some(receiver) = self.index_results.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.index_results = None;
                return;
            }
        };
        self.index_results = None;
        let (generation, result) = result;
        if generation != self.index_generation || self.project.is_none() {
            return;
        }
        match result {
            Ok(index) => {
                let publish_started = Instant::now();
                let (index, semantic_engine) = index;
                let report = index.report();
                let shared = Arc::new(RwLock::new(index));
                self.project_index = Some(shared.clone());
                self.semantic_engine = Some(semantic_engine);
                if let Some(engine) = &self.semantic_engine {
                    for tab in &self.tabs {
                        tab.editor.update(cx, |editor, editor_cx| {
                            let root = self.project.as_ref().unwrap().root_path().to_path_buf();
                            let workspace_source =
                                axiom_index::is_workspace_source_lexical(&tab.path, &root);
                            editor.set_workspace_root(root, workspace_source);
                            editor.set_semantic_engine(engine.clone(), editor_cx);
                        });
                    }
                }
                for tab in &self.tabs {
                    let updates = self.semantic_update_sender.clone();
                    tab.editor.update(cx, |editor, editor_cx| {
                        let root = self.project.as_ref().unwrap().root_path().to_path_buf();
                        let workspace_source =
                            axiom_index::is_workspace_source_lexical(&tab.path, &root);
                        editor.set_workspace_root(root, workspace_source);
                        editor.set_project_symbols(shared.clone(), editor_cx);
                        editor
                            .set_semantic_update_sender(updates, self.project_semantic_generation);
                    });
                }
                self.status = format!("PHP • {} symbols", report.symbols).into();
                if debug_input_enabled() {
                    tracing::debug!(
                        publish_ui_us = publish_started.elapsed().as_micros(),
                        tabs = self.tabs.len(),
                        symbols = report.symbols,
                        "[PROJECT STARTUP UI PROFILE]"
                    );
                }
            }
            Err(error) => self.status = format!("PHP Index Failed: {error}").into(),
        }
        cx.notify();
    }

    fn poll_semantic_updates(&mut self, cx: &mut Context<Self>) {
        let _stage =
            crate::editor_view::UiStageGuard::new(crate::editor_view::UI_STAGE_POLL_SEMANTIC);
        let ui_started = Instant::now();
        let mut receive_us = 0u128;
        let mut publish_us = 0u128;
        let mut tabs_update_us = 0u128;
        let mut tabs_count = 0usize;
        if self.semantic_update_results.is_none() && !self.pending_semantic_fs_changes.is_empty() {
            let changes = std::mem::take(&mut self.pending_semantic_fs_changes);
            self.schedule_semantic_fs_batch(changes, cx);
        }
        if self.semantic_update_results.is_none() {
            let mut updates = HashMap::new();
            let semantic_poll_started = Instant::now();
            let mut items_drained = 0usize;
            let workspace_checks = 0usize;
            let workspace_check_us = 0u128;
            let max_workspace_check_us = 0u128;
            let receive_started = Instant::now();
            while let Ok((project_generation, path, text)) =
                self.semantic_update_receiver.try_recv()
            {
                items_drained += 1;
                if semantic_update_matches(self.project_semantic_generation, project_generation) {
                    updates.insert(path, text);
                } else if debug_input_enabled() {
                    tracing::debug!(path = %path.display(), reason = "generation_mismatch", "[SEMANTIC UPDATE REJECTED]");
                }
            }
            receive_us = receive_started.elapsed().as_micros();
            let semantic_poll_total_us = semantic_poll_started.elapsed().as_micros();
            if debug_ui_stall_enabled()
                && (semantic_poll_total_us >= 3_000
                    || workspace_check_us >= 3_000
                    || items_drained > 1)
            {
                tracing::info!(target: "axiom.ui_stall",
                    items_drained,
                    workspace_checks,
                    workspace_check_us,
                    max_workspace_check_us,
                    remainder_us = semantic_poll_total_us.saturating_sub(workspace_check_us),
                    total_us = semantic_poll_total_us,
                    "[UI SEMANTIC POLL]"
                );
            }
            if !updates.is_empty()
                && let Some(engine) = self.semantic_engine.clone()
            {
                self.semantic_update_generation = self.semantic_update_generation.wrapping_add(1);
                let generation = self.semantic_update_generation;
                let project_generation = self.project_semantic_generation;
                let base = engine.snapshot();
                let (sender, receiver) = mpsc::channel();
                self.semantic_update_results = Some(receiver);
                let changed_files = updates.keys().cloned().collect::<Vec<_>>();
                thread::spawn(move || {
                    let base_revision = base.revision;
                    let mut builder = SnapshotBuilder::from_snapshot(&base);
                    for (path, text) in updates {
                        builder.replace_workspace_file(path, text);
                    }
                    let snapshot = Arc::new(builder.finish());
                    let _ = sender.send((project_generation, generation, Ok(snapshot)));
                    if debug_input_enabled() {
                        tracing::info!(files = ?changed_files, document_version = generation, ?base_revision, new_revision = base_revision.0 + 1, publish = true, "[SEMANTIC UPDATE]");
                    }
                });
            }
        }
        let Some(receiver) = self.semantic_update_results.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.semantic_update_results = None;
                return;
            }
        };
        self.semantic_update_results = None;
        let (project_generation, generation, result) = result;
        if !semantic_update_matches(self.project_semantic_generation, project_generation)
            || generation != self.semantic_update_generation
        {
            if debug_input_enabled() {
                tracing::info!(
                    project_generation,
                    current = self.project_semantic_generation,
                    generation,
                    "[SEMANTIC PROJECT] discard=project_generation_mismatch"
                );
            }
            return;
        }
        if let Ok(snapshot) = result {
            if let Some(engine) = &self.semantic_engine {
                let _stage = crate::editor_view::UiStageGuard::new(
                    crate::editor_view::UI_STAGE_SEMANTIC_PUBLISH,
                );
                let publish_started = Instant::now();
                let published = engine.publish(snapshot);
                publish_us = publish_started.elapsed().as_micros();
                let revision = engine.snapshot().revision.0;
                if published {
                    let _stage = crate::editor_view::UiStageGuard::new(
                        crate::editor_view::UI_STAGE_TAB_UPDATE,
                    );
                    let tabs_started = Instant::now();
                    tabs_count = self.tabs.len();
                    for tab in &self.tabs {
                        tab.editor.update(cx, |editor, editor_cx| {
                            editor.set_semantic_engine(engine.clone(), editor_cx);
                        });
                    }
                    tabs_update_us = tabs_started.elapsed().as_micros();
                    self.status = format!("PHP • semantic revision {revision}").into();
                }
                if debug_input_enabled() {
                    tracing::info!(
                        new_revision = revision,
                        publish = published,
                        "[SEMANTIC UPDATE]"
                    );
                }
            }
            let notify_started = Instant::now();
            cx.notify();
            let notify_us = notify_started.elapsed().as_micros();
            let total_us = ui_started.elapsed().as_micros();
            if debug_ui_stall_enabled() && total_us >= 3_000 {
                tracing::info!(target: "axiom.ui_stall",
                    receive_us,
                    publish_us,
                    tabs_update_us,
                    notify_us,
                    total_us,
                    tabs_count,
                    "[UI SEMANTIC APPLY]"
                );
            }
        }
    }

    /// Applies Explorer filesystem changes to both mutable project indexes and
    /// the published semantic snapshot as one batch. Each tuple is
    /// `(old_path, replacement)`; `None` represents deletion.
    fn schedule_semantic_fs_batch(
        &mut self,
        changes: Vec<(PathBuf, Option<(PathBuf, String)>)>,
        cx: &mut Context<Self>,
    ) {
        if changes.is_empty() || self.semantic_engine.is_none() {
            return;
        }
        if self.semantic_update_results.is_some() {
            self.pending_semantic_fs_changes.extend(changes);
            return;
        }
        let Some(project_index) = self.project_index.clone() else {
            return;
        };
        self.semantic_update_generation = self.semantic_update_generation.wrapping_add(1);
        let generation = self.semantic_update_generation;
        let project_generation = self.project_semantic_generation;
        let base = self.semantic_engine.as_ref().unwrap().snapshot();
        let (sender, receiver) = mpsc::channel();
        self.semantic_update_results = Some(receiver);
        if debug_input_enabled() {
            tracing::info!(
                generation,
                project_generation,
                files_affected = changes.len(),
                "[FS SEMANTIC] batch_start"
            );
        }
        thread::spawn(move || {
            let mut index = project_index.write().expect("project index lock poisoned");
            let mut builder = SnapshotBuilder::from_snapshot(&base);
            for (old_path, replacement) in &changes {
                index.remove_file(old_path);
                builder.remove_file(old_path);
                if let Some((new_path, text)) = replacement {
                    let _ = index.index_file_text_with_source(
                        new_path,
                        text.clone(),
                        "ExplorerFilesystemUpdate",
                    );
                    builder.replace_workspace_file(new_path, text.clone());
                }
            }
            let snapshot = Arc::new(builder.finish());
            let _ = sender.send((project_generation, generation, Ok(snapshot)));
            if debug_input_enabled() {
                tracing::info!(
                    generation,
                    project_generation,
                    files_affected = changes.len(),
                    publish = true,
                    "[FS SEMANTIC] batch_ready"
                );
            }
        });
        cx.notify();
    }

    fn has_dirty_tabs(&self, cx: &App) -> bool {
        self.tabs.iter().any(|tab| tab.editor.read(cx).is_dirty())
    }

    fn request_operation(&mut self, operation: PendingOperation, cx: &mut Context<Self>) {
        if self.has_dirty_tabs(cx) {
            self.pending_operation = Some(operation);
            self.status = "You have unsaved changes".into();
        } else {
            self.perform_operation(operation, cx);
        }
        cx.notify();
    }

    fn perform_operation(&mut self, operation: PendingOperation, cx: &mut Context<Self>) {
        match operation {
            PendingOperation::OpenProject(path) => {
                self.clear_project(cx);
                self.begin_open_project(path, cx);
            }
            PendingOperation::CloseProject => self.clear_project(cx),
            PendingOperation::Exit => {
                self.clear_project(cx);
                cx.quit();
            }
        }
    }

    fn clear_project(&mut self, cx: &mut Context<Self>) {
        self.project_load_generation = self.project_load_generation.wrapping_add(1);
        self.project_load_results = None;
        self.project_semantic_generation = self.project_semantic_generation.wrapping_add(1);
        self.semantic_update_generation = 0;
        self.semantic_update_results = None;
        self.pending_semantic_fs_changes.clear();
        let (semantic_update_sender, semantic_update_receiver) = mpsc::channel();
        self.semantic_update_sender = semantic_update_sender;
        self.semantic_update_receiver = semantic_update_receiver;
        self.navigation_back.clear();
        self.navigation_forward.clear();
        for tab in &self.tabs {
            tab.editor.read(cx).close_lsp_document();
        }
        self.tabs.clear();
        self.active = None;
        self.explorer.clear();
        self.expanded.clear();
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_operation = None;
        self.explorer_input.clear();
        self.explorer_namespace.clear();
        self.explorer_extends.clear();
        self.explorer_implements.clear();
        self.pending_delete = None;
        self.selected_path = None;
        self.context_menu_selected = 0;
        self.context_submenu_selected = 0;
        self.project = None;
        self.project_index = None;
        self.semantic_engine = None;
        self.lsp = None;
        if let Some(session) = self.terminal_session.take() {
            let _ = session.terminate();
        }
        self.terminal_view = None;
        self.terminal_visible = false;
        self.status = "No project".into();
    }

    fn open_project(&mut self, _: &OpenProject, _: &mut Window, cx: &mut Context<Self>) {
        if debug_input_enabled() {
            tracing::info!(id = "project.open_project", "[COMMAND]");
            tracing::info!(received = true, "[OPEN PROJECT COMMAND]");
        }
        self.open_project_picker(cx);
    }

    fn open_project_picker(&mut self, cx: &mut Context<Self>) {
        if debug_input_enabled() {
            tracing::info!(
                before = if self.project_dialog_open {
                    "Opening"
                } else {
                    "Idle"
                },
                "[PICKER STATE]"
            );
        }
        if self.project_dialog_open {
            return;
        }
        self.project_dialog_open = true;
        self.open_menu = None;
        self.status = "Opening project...".into();
        cx.notify();
        if debug_input_enabled() {
            tracing::info!(spawned = true, "[DIALOG TASK]");
            tracing::info!(kind = "folder", "[DIALOG]");
        }
        let workspace = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let path = rfd::AsyncFileDialog::new()
                .pick_folder()
                .await
                .map(|handle| handle.path().to_path_buf());
            if let Some(path) = path {
                if !path.is_dir() {
                    let _ = workspace.update(cx, |this, cx| {
                        this.project_dialog_open = false;
                        this.status = "Open Project requires a directory".into();
                        if debug_input_enabled() {
                            tracing::info!(after = "Idle", "[PICKER STATE]");
                        }
                        cx.notify();
                    });
                    return;
                }
                if debug_input_enabled() {
                    tracing::info!(
                        kind = "folder",
                        selected = %path.display(),
                        type = "directory",
                        "[DIALOG RESULT]"
                    );
                }
                let _ = workspace.update(cx, |this, cx| {
                    this.project_dialog_open = false;
                    if debug_input_enabled() {
                        tracing::info!(after = "Idle", "[PICKER STATE]");
                    }
                    if debug_input_enabled() {
                        tracing::info!(path = %path.display(), "[PROJECT] opening");
                    }
                    this.request_operation(PendingOperation::OpenProject(path), cx);
                });
            } else {
                let _ = workspace.update(cx, |this, cx| {
                    this.project_dialog_open = false;
                    if debug_input_enabled() {
                        tracing::info!(cancelled = true, "[DIALOG RESULT]");
                        tracing::info!(after = "Idle", "[PICKER STATE]");
                    }
                    this.status = "Project selection cancelled".into();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn open_file_dialog(&mut self, _: &OpenFile, _: &mut Window, cx: &mut Context<Self>) {
        if debug_input_enabled() {
            tracing::info!(id = "project.open_file", "[COMMAND]");
            tracing::info!(kind = "file", "[DIALOG]");
        }
        self.open_menu = None;
        let directory = self
            .project
            .as_ref()
            .map(|project| project.root_path().to_path_buf());
        let workspace = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let mut dialog = rfd::AsyncFileDialog::new();
            if let Some(directory) = directory {
                dialog = dialog.set_directory(directory);
            }
            let path = dialog
                .pick_file()
                .await
                .map(|handle| handle.path().to_path_buf());
            if let Some(path) = path {
                let _ = workspace.update(cx, |this, cx| {
                    if !path.is_file() {
                        this.status = "Open File requires a file".into();
                        cx.notify();
                        return;
                    }
                    if debug_input_enabled() {
                        tracing::info!(
                            kind = "file",
                            path = %path.display(),
                            type = "file",
                            "[DIALOG RESULT]"
                        );
                        tracing::info!(path = %path.display(), "[EDITOR] open_file");
                    }
                    this.open_file_background(path, cx);
                });
            } else if debug_input_enabled() {
                tracing::info!(cancelled = true, "[DIALOG RESULT]");
            }
        })
        .detach();
    }

    fn save_all(&mut self, _: &SaveAll, _: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.save_all_now(cx);
    }

    fn save_all_now(&mut self, cx: &mut Context<Self>) -> bool {
        let mut errors = Vec::new();
        for tab in &self.tabs {
            tab.editor.update(cx, |editor, _| {
                if editor.is_dirty()
                    && let Err(error) = editor.save_now()
                {
                    errors.push(format!("{}: {error}", editor.title()));
                }
            });
            // Dirty editors already enqueue a debounced background index
            // update. Never parse a saved file while holding the shared
            // ProjectSymbolIndex lock on the UI thread.
        }
        self.status = if errors.is_empty() {
            "All files saved".into()
        } else {
            format!("Save All failed: {}", errors.join("; ")).into()
        };
        self.definition_cache.clear();
        cx.notify();
        errors.is_empty()
    }

    fn close_active_file(&mut self, _: &CloseFile, _: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        if let Some(index) = self.active {
            self.close_tab(index, cx);
        }
    }

    fn close_project_action(&mut self, _: &CloseProject, _: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.request_operation(PendingOperation::CloseProject, cx);
    }

    fn exit(&mut self, _: &Exit, _: &mut Window, cx: &mut Context<Self>) {
        self.request_operation(PendingOperation::Exit, cx);
    }

    fn show_about(&mut self, _: &ShowAbout, _: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.show_about = true;
        cx.notify();
    }

    fn show_features(&mut self, _: &ShowFeatures, _: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.features_visible = true;
        cx.notify();
    }

    fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.dispatch_editor_action(crate::editor_view::Find, window, cx);
        cx.notify();
    }

    fn command_palette(&mut self, _: &CommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        self.command_palette_visible = true;
        self.command_palette_query.clear();
        self.command_palette_selected = 0;
        self.command_palette_mode = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn palette_commands(&self) -> Vec<axiom_app::commands::CommandDescriptor> {
        if let Some(mode) = &self.command_palette_mode {
            let query = self.command_palette_query.to_ascii_lowercase();
            if let Some(index) = &self.project_index
                && let Ok(index) = index.try_read()
            {
                return index
                    .symbols()
                    .iter()
                    .filter(|symbol| {
                        let is_class = matches!(
                            symbol.kind,
                            axiom_index::ProjectSymbolKind::Class
                                | axiom_index::ProjectSymbolKind::Interface
                                | axiom_index::ProjectSymbolKind::Trait
                                | axiom_index::ProjectSymbolKind::Enum
                        );
                        (mode == "class" && is_class || mode == "symbol")
                            && (query.is_empty()
                                || symbol.name.to_ascii_lowercase().contains(&query)
                                || symbol
                                    .fully_qualified_name
                                    .to_ascii_lowercase()
                                    .contains(&query))
                    })
                    .take(80)
                    .map(|symbol| axiom_app::commands::CommandDescriptor {
                        id: format!(
                            "{}:{}:{}:{}",
                            mode,
                            symbol.file.display(),
                            symbol.range.start,
                            symbol.range.end
                        ),
                        title: symbol.name.clone(),
                        description: format!(
                            "{} • {}",
                            symbol.fully_qualified_name,
                            symbol.file.display()
                        ),
                        category: "Navigate".into(),
                        default_shortcut: None,
                        context: "project".into(),
                    })
                    .collect();
            }
            return Vec::new();
        }
        self.keymap
            .search(&self.command_palette_query)
            .into_iter()
            .cloned()
            .collect()
    }

    fn palette_up(&mut self, _: &PaletteUp, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.palette_commands().len();
        if count > 0 {
            self.command_palette_selected = self.command_palette_selected.saturating_sub(1);
        }
        cx.notify();
    }

    fn palette_down(&mut self, _: &PaletteDown, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.palette_commands().len();
        if count > 0 {
            self.command_palette_selected = (self.command_palette_selected + 1).min(count - 1);
        }
        cx.notify();
    }

    fn palette_escape(&mut self, _: &PaletteEscape, window: &mut Window, cx: &mut Context<Self>) {
        self.command_palette_visible = false;
        self.restore_editor_focus(window, cx);
        cx.notify();
    }

    fn palette_confirm(&mut self, _: &PaletteConfirm, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = self
            .palette_commands()
            .into_iter()
            .nth(self.command_palette_selected)
        else {
            return;
        };
        self.command_palette_visible = false;
        self.execute_command(&command.id, window, cx);
        cx.notify();
    }

    fn execute_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if debug_input_enabled() {
            tracing::info!(id = %id, "[COMMAND DISPATCH]");
            tracing::info!(
                command = %id,
                palette = self.command_palette_visible,
                features = self.features_visible,
                settings = self.settings_visible,
                terminal = self.terminal_visible,
                "[COMMAND] state before"
            );
        }
        match id {
            "help.features" => self.show_features(&ShowFeatures, window, cx),
            "settings.open" => self.settings(&Settings, window, cx),
            "workspace.commands" => self.command_palette(&CommandPalette, window, cx),
            "terminal.toggle" => self.toggle_terminal(&ToggleTerminal, window, cx),
            "project.open_project" => self.open_project(&OpenProject, window, cx),
            "project.open_file" => self.open_file_dialog(&OpenFile, window, cx),
            "project.rename" => {
                if let Some(path) = self.selected_path.clone() {
                    if debug_input_enabled() {
                        tracing::info!(path = %path.display(), "[RENAME]");
                        tracing::info!(popup_open = true, "[RENAME DIALOG]");
                    }
                    window.focus(&self.focus);
                    self.rename_entry(path, cx);
                    if debug_input_enabled() {
                        tracing::info!(
                            command = "project.rename",
                            executed = true,
                            "[COMMAND RESULT]"
                        );
                    }
                } else {
                    self.status = "No project item selected".into();
                    cx.notify();
                }
            }
            "project.new" => {
                if let Some(directory) = self.selected_path.clone().or_else(|| {
                    self.project
                        .as_ref()
                        .map(|project| project.root_path().to_path_buf())
                }) {
                    let directory = if directory.is_dir() {
                        directory
                    } else {
                        directory.parent().unwrap_or(&directory).to_path_buf()
                    };
                    self.open_new_menu(directory, cx);
                }
            }
            "navigate.back" => self.navigate_back(&NavigateBack, window, cx),
            "navigate.forward" => self.navigate_forward(&NavigateForward, window, cx),
            "editor.reformat" => {
                self.dispatch_editor_action(crate::editor_view::Reformat, window, cx)
            }
            "editor.undo" => self.dispatch_editor_action(crate::editor_view::Undo, window, cx),
            "editor.redo" => self.dispatch_editor_action(crate::editor_view::Redo, window, cx),
            "editor.select_all" => {
                self.dispatch_editor_action(crate::editor_view::SelectAll, window, cx)
            }
            "editor.save" => self.dispatch_editor_action(crate::editor_view::Save, window, cx),
            "editor.find" => self.find(&Find, window, cx),
            "code.completion" => {
                self.dispatch_editor_action(crate::editor_view::Complete, window, cx)
            }
            "editor.complete_statement" => {
                self.dispatch_editor_action(crate::editor_view::CompleteStatement, window, cx)
            }
            "navigate.definition" => {
                let has_lsp = self
                    .active
                    .and_then(|index| self.tabs.get(index))
                    .and_then(|tab| tab.editor.read(cx).lsp_uri())
                    .is_some();
                if has_lsp {
                    self.dispatch_editor_action(crate::editor_view::Definition, window, cx)
                } else {
                    self.navigate_native_definition(cx);
                }
            }
            "navigate.class" => {
                self.command_palette_mode = Some("class".into());
                self.command_palette_query.clear();
                self.command_palette_selected = 0;
                self.command_palette_visible = true;
            }
            "navigate.symbol" => {
                self.command_palette_mode = Some("symbol".into());
                self.command_palette_query.clear();
                self.command_palette_selected = 0;
                self.command_palette_visible = true;
            }
            id if id.starts_with("class:") || id.starts_with("symbol:") => {
                let kind = id.split(':').next().unwrap_or("symbol");
                let payload = id.get(kind.len() + 1..).unwrap_or_default();
                let mut parts = payload.rsplitn(3, ':');
                let end = parts.next().and_then(|value| value.parse::<usize>().ok());
                let start = parts.next().and_then(|value| value.parse::<usize>().ok());
                let path = parts.next();
                if let (Some(end), Some(start), Some(path)) = (end, start, path) {
                    if debug_input_enabled() {
                        tracing::info!(kind, path, start, end, "[NAVIGATION TARGET]");
                    }
                    if let Some(active) = self.active.and_then(|index| self.tabs.get(index))
                        && let Some(position) = active.editor.read(cx).current_lsp_position()
                    {
                        self.navigation_back.push(NavigationLocation {
                            path: active.path.clone(),
                            position,
                        });
                        self.navigation_forward.clear();
                    }
                    self.command_palette_mode = None;
                    self.open_file(PathBuf::from(path), window, cx);
                    if let Some(tab) = self.active.and_then(|index| self.tabs.get(index)) {
                        tab.editor
                            .update(cx, |editor, cx| editor.reveal_byte_range(start..end, cx));
                    }
                    if debug_input_enabled() {
                        tracing::info!(success = true, "[NAVIGATION RESULT]");
                    }
                }
            }
            "editor.copy" => self.dispatch_editor_action(crate::editor_view::Copy, window, cx),
            "editor.cut" => self.dispatch_editor_action(crate::editor_view::Cut, window, cx),
            "editor.paste" => self.dispatch_editor_action(crate::editor_view::Paste, window, cx),
            _ => self.status = format!("Command {id} is not available in this context").into(),
        }
        if debug_input_enabled() {
            tracing::info!(
                command = %id,
                palette = self.command_palette_visible,
                features = self.features_visible,
                settings = self.settings_visible,
                terminal = self.terminal_visible,
                "[COMMAND] state after; notify=true"
            );
        }
    }

    fn go_to_class(&mut self, _: &GoToClass, window: &mut Window, cx: &mut Context<Self>) {
        self.execute_command("navigate.class", window, cx);
    }

    fn go_to_symbol(&mut self, _: &GoToSymbol, window: &mut Window, cx: &mut Context<Self>) {
        self.execute_command("navigate.symbol", window, cx);
    }

    fn navigate_native_definition(&mut self, cx: &mut Context<Self>) -> bool {
        let native = self
            .active
            .and_then(|index| self.tabs.get(index))
            .and_then(|tab| tab.editor.read(cx).native_definition_location());
        if let Some((path, position)) = native {
            if debug_input_enabled() {
                tracing::info!(path = %path.display(), line = position.line, character = position.character, "[NAVIGATION TARGET]");
            }
            self.navigate_to_definition(DefinitionTarget { path, position }, cx);
            return true;
        } else {
            self.status = "Definition não encontrada".into();
            if debug_input_enabled() {
                tracing::info!(
                    success = false,
                    reason = "no_definition",
                    "[NAVIGATION RESULT]"
                );
            }
        }
        false
    }

    fn navigate_semantic_definition(&mut self, cx: &mut Context<Self>) -> SemanticDefinitionRoute {
        let profile = std::env::var_os("AXIOM_DEBUG_DEFINITION_PROFILE").is_some();
        let navigation_started = Instant::now();
        let Some(engine) = self.semantic_engine.clone() else {
            return SemanticDefinitionRoute::Unavailable;
        };
        let Some(tab_index) = self.active else {
            return SemanticDefinitionRoute::Unavailable;
        };
        let Some(tab) = self.tabs.get(tab_index) else {
            return SemanticDefinitionRoute::Unavailable;
        };
        let editor = tab.editor.read(cx);
        let path = tab.path.clone();
        let offset = editor.current_cursor_offset();
        let text = editor.document_content();
        let syntax_context = editor.definition_syntax_context(offset);
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            let token = axiom_syntax::PhpSyntax::parse(text.clone())
                .ok()
                .and_then(|syntax| syntax.token_at_byte(offset));
            tracing::info!(
                offset,
                token_text = ?token.as_ref().map(|token| token.text.as_str()),
                token_kind = ?token.as_ref().map(|token| token.kind.as_str()),
                token_range = ?token.as_ref().map(|token| token.range.clone()),
                "\n\n\n[DEFINITION INPUT]"
            );
        }
        let snapshot = engine.snapshot();
        let context = axiom_index::DefinitionQueryContext {
            document_version: None,
            semantic_revision: snapshot.revision,
        };
        let semantic_started = Instant::now();
        let detailed = snapshot.definition_at_detailed_with_syntax(
            &path,
            &text,
            offset,
            syntax_context,
            context,
        );
        let semantic_us = semantic_started.elapsed().as_micros();
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            tracing::info!(outcome = ?detailed.outcome, result = ?detailed.result, "\n\n\n[DEFINITION RESULT]");
        }
        let axiom_index::DefinitionResult::Resolved(candidate) = &detailed.result else {
            if debug_input_enabled() {
                tracing::info!(
                    source = ?DefinitionSource::Unresolved,
                    outcome = ?detailed.outcome,
                    result = ?detailed.result,
                    "[DEFINITION SEMANTIC]"
                );
            }
            return SemanticDefinitionRoute::Outcome(detailed.outcome);
        };
        let target_path = candidate.location.file.clone();
        let target_lookup_started = Instant::now();
        let Some((target_text, text_source)) = self.current_text_for_path(&target_path, cx) else {
            return SemanticDefinitionRoute::Outcome(detailed.outcome);
        };
        let target_text_lookup_us = target_lookup_started.elapsed().as_micros();
        if debug_input_enabled() {
            let span = &candidate.location.span;
            let text_at_span = target_text.get(span.clone()).unwrap_or("");
            tracing::info!(
                file = %target_path.display(),
                span = ?span,
                text_source = ?text_source,
                text_len = target_text.len(),
                text_at_span,
                "[NAVIGATION TARGET]"
            );
        }
        let position_started = Instant::now();
        let position = PositionCodec::offset_to_position(
            &target_text,
            candidate.location.span.start,
            Default::default(),
        );
        if profile {
            tracing::debug!(semantic_us, target_text_lookup_us, position_codec_us = position_started.elapsed().as_micros(), total_navigation_us = navigation_started.elapsed().as_micros(), target_text_source = ?text_source, "[NAVIGATION PROFILE]");
        }
        if debug_input_enabled() {
            tracing::info!(
                source = ?DefinitionSource::Semantic,
                outcome = ?detailed.outcome,
                path = %target_path.display(),
                span_start = candidate.location.span.start,
                "[DEFINITION SEMANTIC]"
            );
        }
        self.navigate_to_definition(
            DefinitionTarget {
                path: target_path,
                position,
            },
            cx,
        );
        SemanticDefinitionRoute::Resolved
    }

    fn native_definition_action(
        &mut self,
        _: &crate::editor_view::NativeDefinition,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if debug_input_enabled() {
            tracing::info!(provider = "native", "[DEFINITION REQUEST]");
        }
        let semantic_route = self.navigate_semantic_definition(cx);
        let semantic_result = matches!(semantic_route, SemanticDefinitionRoute::Resolved);
        let vendor_allowed = vendor_allowed_for_route(semantic_route);
        if debug_input_enabled() {
            tracing::info!(
                called = true,
                result = semantic_result,
                "[DEFINITION SEMANTIC PATH]"
            );
            tracing::info!(
                semantic_outcome = ?match semantic_route {
                    SemanticDefinitionRoute::Outcome(outcome) => Some(outcome),
                    SemanticDefinitionRoute::Resolved => Some(axiom_index::SemanticDefinitionOutcome::Resolved),
                    SemanticDefinitionRoute::Unavailable => None,
                },
                vendor_allowed,
                legacy_allowed = true,
                lsp_allowed = true,
                reason = if vendor_allowed { "deferred-vendor-or-no-semantic-engine" } else { "semantic-outcome-disallows-vendor" },
                "[DEFINITION ROUTING]"
            );
        }
        if semantic_result {
            return;
        }
        let vendor_result = vendor_allowed && self.start_vendor_definition(cx);
        if debug_input_enabled() {
            tracing::info!(
                called = true,
                result = vendor_result,
                "[DEFINITION VENDOR PATH]"
            );
        }
        if vendor_result {
            return;
        }
        let native_result = self.navigate_native_definition(cx);
        if debug_input_enabled() {
            tracing::info!(
                called = true,
                result = native_result,
                "[DEFINITION LEGACY PATH]"
            );
        }
        if native_result {
            if debug_input_enabled() {
                tracing::info!(source = ?DefinitionSource::LegacyRuntime, "[DEFINITION SOURCE]");
            }
            return;
        }
        if let (Some(lsp), Some(uri), Some(position)) = (
            &self.lsp,
            self.active
                .and_then(|index| self.tabs.get(index))
                .and_then(|tab| tab.editor.read(cx).lsp_uri())
                .cloned(),
            self.active
                .and_then(|index| self.tabs.get(index))
                .and_then(|tab| tab.editor.read(cx).current_lsp_position()),
        ) {
            if debug_input_enabled() {
                tracing::info!(source = ?DefinitionSource::Lsp, "[DEFINITION SOURCE]");
            }
            lsp.request_definition(uri, position);
        } else if debug_input_enabled() {
            tracing::info!(source = ?DefinitionSource::Unresolved, "[DEFINITION SOURCE]");
        }
    }

    fn start_vendor_definition(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(tab_index) = self.active else {
            return false;
        };
        let Some(tab) = self.tabs.get(tab_index) else {
            return false;
        };
        let query_key = tab
            .editor
            .read(cx)
            .definition_query()
            .map(|query| format!("{}::{query:?}", tab.path.display()));
        if let Some(key) = query_key.as_deref()
            && let Some(target) = definition_cache_lookup(&self.definition_cache, key)
        {
            if debug_input_enabled() {
                tracing::info!(
                    kind = "definition",
                    symbol = %key,
                    source = ?DefinitionSource::LegacyProject,
                    hit = true,
                    "[DEFINITION CACHE]"
                );
            }
            self.navigate_to_definition(target, cx);
            return true;
        }
        // Workspace project definitions are resolved exclusively by the
        // SemanticEngine. This path is retained only for deferred Vendor
        // symbols, whose files are loaded asynchronously below.
        self.definition_loading = true;
        self.definition_loading_tick = 0;
        self.status = "Resolving definition…".into();
        cx.notify();
        let Some(request) = tab.editor.read(cx).vendor_definition_request() else {
            self.definition_loading = false;
            cx.notify();
            return false;
        };
        if !self.vendor_definition_inflight.insert(request.fqn.clone()) {
            if debug_input_enabled() {
                tracing::info!(fqn = %request.fqn, deduplicated = true, "[VENDOR REQUEST]");
            }
            return true;
        }
        if debug_input_enabled() {
            tracing::info!(
                attempted = true,
                fqn = %request.fqn,
                source = ?DefinitionSource::LegacyVendor,
                "[DEFINITION VENDOR]"
            );
        }
        let weak = cx.entity().downgrade();
        let request_fqn = request.fqn.clone();
        let cache_key = query_key.clone();
        if debug_input_enabled() {
            tracing::info!(fqn = %request.fqn, "[DEFINITION NATIVE START]");
        }
        cx.spawn(async move |_, cx| {
            let started = std::time::Instant::now();
            let result = gpui::background_executor()
                .spawn(async move {
                    // Clone metadata/cache under a short lock. Parsing and all
                    // filesystem work happen on the private clone, never while
                    // the shared Vendor RwLock is held.
                    let mut parser = request
                        .index
                        .read()
                        .map_err(|_| "vendor index lock poisoned")?
                        .clone();
                    let mut owner_fqn = request.fqn.clone();
                    for member in &request.chain {
                        let Some(method) =
                            parser.symbols_of(&owner_fqn).into_iter().find(|symbol| {
                                symbol.name == *member
                                    && symbol.kind == axiom_index::ProjectSymbolKind::Method
                            })
                        else {
                            return Err("vendor chain member not found");
                        };
                        let Some(next) = method.return_type else {
                            return Err("vendor chain return type unavailable");
                        };
                        let next = next
                            .trim_start_matches('?')
                            .split(['|', '&'])
                            .next()
                            .unwrap_or(&next)
                            .trim();
                        owner_fqn = if matches!(next, "self" | "static") {
                            owner_fqn
                        } else {
                            next.to_owned()
                        };
                    }
                    let symbols = parser.symbols_of(&owner_fqn);
                    let symbol = symbols.into_iter().find(|symbol| match &request.member {
                        Some(member) => {
                            symbol.name == *member
                                && (request.is_static
                                    == symbol.modifiers.iter().any(|m| m == "static"))
                        }
                        None => matches!(
                            symbol.kind,
                            axiom_index::ProjectSymbolKind::Class
                                | axiom_index::ProjectSymbolKind::Interface
                                | axiom_index::ProjectSymbolKind::Trait
                                | axiom_index::ProjectSymbolKind::Enum
                        ),
                    });
                    let Some(symbol) = symbol else {
                        return Err("vendor symbol not found");
                    };
                    let path = symbol.file.clone();
                    let offset = symbol.range.start;
                    if let Ok(mut index) = request.index.write() {
                        index.merge_parsed_cache(&parser);
                    }
                    let content =
                        std::fs::read_to_string(&path).map_err(|_| "vendor file unreadable")?;
                    Ok::<_, &'static str>((path, offset, content))
                })
                .await;
            let _ = weak.update(cx, |workspace, cx| {
                workspace.vendor_definition_inflight.remove(&request_fqn);
                if debug_input_enabled() {
                    tracing::info!(
                        elapsed_ms = started.elapsed().as_millis(),
                        result = result.is_ok(),
                        "[DEFINITION NATIVE END]"
                    );
                }
                match result {
                    Ok((path, offset, content)) => {
                        if let Some(key) = cache_key.as_ref() {
                            let position = axiom_lsp::PositionCodec::offset_to_position(
                                &content,
                                offset,
                                axiom_lsp::PositionEncoding::Utf8,
                            );
                            workspace.definition_cache.insert(
                                key.clone(),
                                DefinitionTarget {
                                    path: path.clone(),
                                    position,
                                },
                            );
                        }
                        workspace.open_vendor_definition(path, offset, content, cx);
                        workspace.definition_loading = false;
                        cx.notify();
                    }
                    Err(error) => {
                        workspace.definition_loading = false;
                        workspace.status = format!("Definition not found: {error}").into();
                        if debug_input_enabled() {
                            tracing::info!(success = false, "[NAVIGATION RESULT]");
                        }
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        true
    }

    fn open_vendor_definition(
        &mut self,
        path: PathBuf,
        offset: usize,
        content: String,
        cx: &mut Context<Self>,
    ) {
        let path = fs::canonicalize(&path).unwrap_or(path);
        let position = axiom_lsp::PositionCodec::offset_to_position(
            &content,
            offset,
            axiom_lsp::PositionEncoding::Utf8,
        );
        if let Some(index) = self.tabs.iter().position(|tab| tab.path == path) {
            self.active = Some(index);
            self.focus_active_editor = true;
            self.tabs[index]
                .editor
                .update(cx, |editor, cx| editor.reveal_lsp_position(position, cx));
        } else {
            let mut document = Document::from_content(&content);
            document.set_file_path(path.clone());
            let editor = cx
                .new(|cx| EditorView::from_document(path.clone(), document, self.lsp.clone(), cx));
            if let Some(symbols) = &self._runtime_symbols {
                editor.update(cx, |editor, _| editor.set_runtime_symbols(symbols.clone()));
            }
            if let Some(index) = &self.project_index {
                editor.update(cx, |editor, _| {
                    let root = self
                        .project
                        .as_ref()
                        .map(|p| p.root_path().to_path_buf())
                        .unwrap_or_default();
                    let workspace_source = axiom_index::is_workspace_source_lexical(&path, &root);
                    editor.set_workspace_root(root, workspace_source)
                });
                editor.update(cx, |editor, editor_cx| {
                    editor.set_project_symbols(index.clone(), editor_cx)
                });
                editor.update(cx, |editor, _| {
                    editor.set_semantic_update_sender(
                        self.semantic_update_sender.clone(),
                        self.project_semantic_generation,
                    )
                });
            }
            if let Some(engine) = &self.semantic_engine {
                editor.update(cx, |editor, editor_cx| {
                    editor.set_semantic_engine(engine.clone(), editor_cx);
                });
            }
            if let Some(vendor) = &self.vendor_index {
                editor.update(cx, |editor, _| editor.set_vendor_symbols(vendor.clone()));
            }
            self.tabs.push(OpenTab {
                path: path.clone(),
                editor,
            });
            self.active = Some(self.tabs.len() - 1);
            self.focus_active_editor = true;
            let index = self.active.unwrap();
            self.tabs[index]
                .editor
                .update(cx, |editor, cx| editor.reveal_lsp_position(position, cx));
        }
        self.status = "Definition resolved".into();
        if debug_input_enabled() {
            tracing::info!(success = true, "[DEFINITION TARGET]");
            tracing::info!(success = true, "[NAVIGATION RESULT]");
        }
        cx.notify();
    }

    fn find_usages_action(
        &mut self,
        _: &crate::editor_view::References,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_usages.clear();
        self.find_usages_visible = false;
        self.find_usages_context = None;
        let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) else {
            return;
        };
        let editor = tab.editor.read(cx);
        let path = tab.path.clone();
        let offset = editor.current_cursor_offset();
        let source_session = editor.document_session();
        let Some(snapshot) = self
            .semantic_engine
            .as_ref()
            .and_then(|engine| engine.try_snapshot())
        else {
            self.status =
                "Find Usages: semantic snapshot unavailable; retry when indexing finishes".into();
            cx.notify();
            return;
        };
        // Rope clones retain resident text without materializing buffers on the UI.
        let buffers = self
            .tabs
            .iter()
            .map(|tab| {
                let editor = tab.editor.read(cx);
                (
                    axiom_index::PersistentFileKey::workspace_lexical(&tab.path),
                    editor.references_text_snapshot(),
                )
            })
            .collect::<Vec<_>>();
        let context = Arc::new(FindUsagesContext {
            kind: NavigationQueryKind::References,
            project_generation: self.project_semantic_generation,
            snapshot,
            documents: self.references_document_stamps(cx),
            source_session,
        });
        self.find_usages_context = Some(context.clone());
        self.status = "Finding usages…".into();
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let query_context = context.clone();
            let result = executor
                .spawn(async move {
                    let buffers = buffers
                        .into_iter()
                        .map(|(key, rope)| (key, rope.to_string()))
                        .collect();
                    prepare_find_usages(&query_context.snapshot, &path, offset, buffers)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.references_context_current(&context, cx) {
                    return;
                }
                match result {
                    Ok((targets, status)) => {
                        this.find_usages = targets;
                        this.find_usages_selected = 0;
                        this.find_usages_visible = true;
                        this.status =
                            format!("{} referência(s); {:?}", this.find_usages.len(), status)
                                .into();
                        this.find_usages_focus_pending = true;
                    }
                    Err(message) => {
                        this.status = message.into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn go_to_implementation(
        &mut self,
        _: &GoToImplementation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) else {
            return;
        };
        let path = tab.path.clone();
        let offset = tab.editor.read(cx).current_cursor_offset();
        let source_text = tab.editor.read(cx).references_text_snapshot().to_string();
        let Some(snapshot) = self.semantic_engine.as_ref().and_then(|e| e.try_snapshot()) else {
            self.status = "Go to Implementation: semantic snapshot unavailable".into();
            cx.notify();
            return;
        };
        let session = tab.editor.read(cx).document_session();
        let context = Arc::new(FindUsagesContext {
            kind: NavigationQueryKind::Implementations,
            project_generation: self.project_semantic_generation,
            snapshot,
            documents: self.references_document_stamps(cx),
            source_session: session,
        });
        self.find_usages_context = Some(context.clone());
        self.status = "Finding implementations…".into();
        let source_key = axiom_index::PersistentFileKey::workspace_lexical(&path);
        if !context
            .snapshot
            .matches_file_text(&source_key, &source_text)
        {
            self.status = "Go to Implementation: current buffer is not indexed yet".into();
            cx.notify();
            return;
        }
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let query_context = context.clone();
            let result = executor
                .spawn(
                    async move { prepare_implementations(&query_context.snapshot, &path, offset) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.references_context_current(&context, cx) {
                    return;
                }
                match result {
                    Ok(mut targets) if targets.len() == 1 => {
                        let target = targets.remove(0);
                        this.navigate_to_definition_preloaded(
                            DefinitionTarget {
                                path: target.file,
                                position: target.position,
                            },
                            None,
                            cx,
                        );
                    }
                    Ok(targets) => {
                        this.find_usages = targets;
                        this.find_usages_selected = 0;
                        this.find_usages_visible = !this.find_usages.is_empty();
                        this.find_usages_focus_pending = this.find_usages_visible;
                        this.status = if this.find_usages_visible {
                            format!("{} implementação(ões)", this.find_usages.len()).into()
                        } else {
                            "No implementations found".into()
                        };
                    }
                    Err(message) => {
                        this.status = message.into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_find_usages(
        &mut self,
        _: &CloseFindUsages,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_usages_visible = false;
        self.find_usages.clear();
        self.find_usages_context = None;
        self.restore_editor_focus(window, cx);
        cx.notify();
    }

    fn references_document_stamps(&self, cx: &App) -> Vec<(PathBuf, u64, u64)> {
        self.tabs
            .iter()
            .map(|tab| {
                let editor = tab.editor.read(cx);
                (
                    tab.path.clone(),
                    editor.document_session(),
                    editor.references_revision(),
                )
            })
            .collect()
    }

    fn references_context_current(&self, context: &Arc<FindUsagesContext>, cx: &App) -> bool {
        self.find_usages_context
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, context))
            && context.project_generation == self.project_semantic_generation
            && self.references_document_stamps(cx) == context.documents
            && self
                .active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|tab| tab.editor.read(cx).document_session() == context.source_session)
            && self
                .semantic_engine
                .as_ref()
                .and_then(|engine| engine.try_snapshot())
                .is_some_and(|snapshot| Arc::ptr_eq(&snapshot, &context.snapshot))
    }

    fn open_find_usage(&mut self, target: FindUsageTarget, cx: &mut Context<Self>) {
        let Some(context) = self.find_usages_context.clone() else {
            return;
        };
        if !self.references_context_current(&context, cx) {
            self.find_usages_visible = false;
            self.status = "Find Usages changed; run the query again".into();
            cx.notify();
            return;
        }
        let target_key = axiom_index::PersistentFileKey::workspace_lexical(&target.file);
        if let Some(index) = self.tabs.iter().position(|tab| {
            axiom_index::PersistentFileKey::workspace_lexical(&tab.path) == target_key
        }) {
            // The query fingerprint and unchanged buffer revision already validate
            // this target. Open/unsaved buffers must never be reopened from disk.
            if let Some(origin) = self.active.and_then(|i| self.tabs.get(i)).and_then(|tab| {
                tab.editor
                    .read(cx)
                    .current_lsp_position()
                    .map(|position| NavigationLocation {
                        path: tab.path.clone(),
                        position,
                    })
            }) {
                self.navigation_back.push(origin);
                self.navigation_forward.clear();
            }
            self.active = Some(index);
            self.focus_active_editor = true;
            self.find_usages_visible = false;
            self.tabs[index].editor.update(cx, |editor, cx| {
                editor.reveal_lsp_position(target.position, cx)
            });
            cx.notify();
            return;
        }
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let query_context = context.clone();
            let result = executor
                .spawn(async move {
                    let path = fs::canonicalize(&target.file).ok()?;
                    let document = Document::from_file(&path).ok()?;
                    let text = document.content();
                    let key = axiom_index::PersistentFileKey::workspace_lexical(&target.file);
                    query_context
                        .snapshot
                        .matches_file_text(&key, &text)
                        .then_some((
                            DefinitionTarget {
                                path,
                                position: target.position,
                            },
                            document,
                        ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.references_context_current(&context, cx) {
                    return;
                }
                this.find_usages_visible = false;
                if let Some((target, document)) = result {
                    this.navigate_to_definition_preloaded(target, Some(document), cx);
                } else {
                    this.status =
                        "Reference file changed or was removed; run Find Usages again".into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn dispatch_editor_action<A: Action + Clone + 'static>(
        &mut self,
        action: A,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.active.and_then(|index| self.tabs.get(index)) {
            window.focus(&tab.editor.read(cx).focus_handle(cx));
            window.dispatch_action(action.boxed_clone(), cx);
        } else {
            self.status = "No editor document is active".into();
        }
    }

    fn restore_editor_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.active.and_then(|index| self.tabs.get(index)) {
            window.focus(&tab.editor.read(cx).focus_handle(cx));
        } else {
            window.focus(&self.focus);
        }
    }

    fn settings(&mut self, _: &Settings, _: &mut Window, cx: &mut Context<Self>) {
        self.settings_visible = true;
        self.settings_query.clear();
        self.settings_selected = None;
        cx.notify();
    }

    fn import_runtime_stubs_action(
        &mut self,
        _: &ImportRuntimeStubs,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_menu = None;
        self.import_runtime_stubs(cx);
    }

    fn import_runtime_stub_files_action(
        &mut self,
        _: &ImportRuntimeStubFiles,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_menu = None;
        if debug_stubs_enabled() {
            tracing::info!("[STUB IMPORT ACTION] files");
        }
        self.import_runtime_stub_files(cx);
    }

    fn import_runtime_stubs(&mut self, cx: &mut Context<Self>) {
        let target = self.runtime_stub_path.clone();
        let workspace = cx.entity().downgrade();
        self.status = "Runtime Stubs: Choose a folder to import...".into();
        if debug_stubs_enabled() {
            tracing::info!(target = %target.display(), "[STUB IMPORT PICKER] open folder");
        }
        cx.spawn(async move |_, cx| {
            let selected = rfd::AsyncFileDialog::new()
                .pick_folder()
                .await
                .map(|handle| handle.path().to_path_buf());
            let result = match selected {
                None => Ok(None),
                Some(source) if source == target => Ok(Some(StubImportReport::default())),
                Some(source) => gpui::background_executor()
                    .spawn(async move { copy_stub_tree(&source, &target) })
                    .await
                    .map(Some),
            };
            if debug_stubs_enabled() {
                match &result {
                    Ok(Some(report)) => tracing::info!(
                        copied = report.copied,
                        conflicts = report.conflicts,
                        "[STUB IMPORT COPY] folder complete"
                    ),
                    Ok(None) => tracing::info!("[STUB IMPORT PICKER] cancelled"),
                    Err(error) => tracing::warn!(%error, "[STUB IMPORT COPY] failed"),
                }
            }
            let _ = workspace.update(cx, |this, cx| {
                match result {
                    Ok(Some(report)) => {
                        this.status = format!(
                            "Imported {} stub files ({} conflicts skipped)",
                            report.copied, report.conflicts
                        )
                        .into();
                        this.reload_runtime_stubs(cx);
                    }
                    Ok(None) => this.status = "Runtime stubs import cancelled".into(),
                    Err(error) => {
                        this.status = format!("Runtime stubs import failed: {error}").into()
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn import_runtime_stub_files(&mut self, cx: &mut Context<Self>) {
        let target = self.runtime_stub_path.clone();
        let workspace = cx.entity().downgrade();
        self.status = "Runtime Stubs: Choose PHP files to import...".into();
        if debug_stubs_enabled() {
            tracing::info!(target = %target.display(), "[STUB IMPORT PICKER] open files");
        }
        cx.spawn(async move |_, cx| {
            let selected = rfd::AsyncFileDialog::new()
                .add_filter("PHP stubs", &["php"])
                .pick_files()
                .await
                .map(|files| {
                    files
                        .into_iter()
                        .map(|file| file.path().to_path_buf())
                        .collect::<Vec<_>>()
                });
            let result = match selected {
                None => Ok(None),
                Some(files) => gpui::background_executor()
                    .spawn(async move { copy_stub_files(&files, &target) })
                    .await
                    .map(Some),
            };
            if debug_stubs_enabled() {
                match &result {
                    Ok(Some(report)) => tracing::info!(
                        copied = report.copied,
                        conflicts = report.conflicts,
                        "[STUB IMPORT COPY] files complete"
                    ),
                    Ok(None) => tracing::info!("[STUB IMPORT PICKER] cancelled"),
                    Err(error) => tracing::warn!(%error, "[STUB IMPORT COPY] failed"),
                }
            }
            let _ = workspace.update(cx, |this, cx| {
                match result {
                    Ok(Some(report)) => {
                        this.status = format!(
                            "Imported {} stub files ({} conflicts skipped)",
                            report.copied, report.conflicts
                        )
                        .into();
                        this.reload_runtime_stubs(cx);
                    }
                    Ok(None) => this.status = "Runtime stubs import cancelled".into(),
                    Err(error) => {
                        this.status = format!("Runtime stubs import failed: {error}").into()
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[allow(dead_code)]
    fn reload_runtime_stubs_sync(&mut self, cx: &mut Context<Self>) {
        self.status = "Runtime Stubs: Updating...".into();
        let provider = StubProvider::new(self.runtime_stub_path.clone());
        let _ = fs::create_dir_all(&self.runtime_stub_path);
        let result = self
            .runtime_stub_cache_path
            .as_deref()
            .map_or_else(|| provider.load(), |cache| provider.load_incremental(cache));
        match result {
            Ok((index, report)) => {
                self.runtime_stubs = RuntimeStubStatus::Loaded {
                    files: report.files_discovered,
                    symbols: report.symbols_indexed,
                };
                let shared = Arc::new(index);
                self._runtime_symbols = Some(shared.clone());
                for tab in &self.tabs {
                    tab.editor
                        .update(cx, |editor, _| editor.set_runtime_symbols(shared.clone()));
                }
                self.status =
                    format!("Runtime Stubs: Ready ({} symbols)", report.symbols_indexed).into();
            }
            Err(error) => self.status = format!("Runtime Stubs: Error — {error}").into(),
        }
        cx.notify();
    }

    fn reload_runtime_stubs(&mut self, cx: &mut Context<Self>) {
        let _ = fs::create_dir_all(&self.runtime_stub_path);
        self.begin_runtime_stub_load(cx, true);
    }

    fn clear_runtime_stub_cache(&mut self, cx: &mut Context<Self>) {
        if let Some(cache) = &self.runtime_stub_cache_path {
            let _ = fs::remove_file(cache);
        }
        self.reload_runtime_stubs(cx);
    }

    fn debug_input(&mut self, _: &DebugInput, _: &mut Window, cx: &mut Context<Self>) {
        self.debug_overlay_visible = !self.debug_overlay_visible;
        self.status = "Input key received (F12)".into();
        tracing::info!("[RESULT] debug F12 executed");
        cx.notify();
    }

    fn select_setting_command(&mut self, id: String, cx: &mut Context<Self>) {
        self.settings_selected = Some(id);
        cx.notify();
    }

    fn remove_selected_shortcut(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.settings_selected.clone() {
            let _ = self.keymap.set_shortcut(&id, None);
            let _ = self.keymap.persist_user();
        }
        cx.notify();
    }

    fn reset_selected_shortcut(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.settings_selected.clone() {
            self.keymap.reset(&id);
            let _ = self.keymap.persist_user();
        }
        cx.notify();
    }

    fn begin_shortcut_capture(&mut self, cx: &mut Context<Self>) {
        self.shortcut_capture = true;
        self.captured_shortcut = None;
        self.shortcut_conflict = None;
        cx.notify();
    }

    fn capture_shortcut(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.shortcut_capture {
            return;
        }
        if event.keystroke.modifiers.control
            && event.keystroke.modifiers.alt
            && event.keystroke.key_char.is_some()
        {
            return;
        }
        let key = event.keystroke.key.to_ascii_lowercase();
        if key == "escape" {
            self.shortcut_capture = false;
            self.captured_shortcut = None;
            cx.notify();
            return;
        }
        if matches!(
            key.as_str(),
            "control" | "shift" | "alt" | "command" | "super"
        ) {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        let mut value = String::new();
        if modifiers.control {
            value.push_str("ctrl-");
        }
        if modifiers.shift {
            value.push_str("shift-");
        }
        if modifiers.alt {
            value.push_str("alt-");
        }
        value.push_str(&key);
        self.captured_shortcut = Some(value);
        cx.notify();
    }

    fn handle_workspace_keydown(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _stage =
            crate::editor_view::UiStageGuard::new(crate::editor_view::UI_STAGE_KEY_CALLBACK);
        self.key_event_id = self.key_event_id.wrapping_add(1);
        self.last_key_event_at = Some(Instant::now());
        crate::editor_view::LAST_UI_KEY_EVENT_ID.store(self.key_event_id, Ordering::Relaxed);
        let _probe = UiKeyProbe::new(event.keystroke.key.clone());
        if self.explorer_operation.is_none()
            && self.pending_delete.is_none()
            && (debug_keys_enabled() || debug_input_enabled())
        {
            tracing::info!(
                key = %event.keystroke.key,
                ctrl = event.keystroke.modifiers.control,
                shift = event.keystroke.modifiers.shift,
                alt = event.keystroke.modifiers.alt,
                context = "workspace-root",
                "[KEY RAW]"
            );
        }
        if self.shortcut_capture {
            self.capture_shortcut(event, window, cx);
            return;
        }
        if event.keystroke.modifiers.control
            && event.keystroke.modifiers.alt
            && event.keystroke.key_char.is_some()
        {
            return;
        }
        let key = event.keystroke.key.to_ascii_lowercase();
        if self.pending_delete.is_some() {
            if debug_input_enabled() {
                tracing::info!(key = %key, "[DELETE MODAL KEY]");
            }
            if key == "escape" {
                self.pending_delete = None;
                self.pending_delete_is_directory = false;
                self.delete_focus_pending = false;
                self.status = "Deletion cancelled".into();
                cx.notify();
            }
            return;
        }
        if self.focus.is_focused(window) && key == "delete" {
            if let Some(path) = self.selected_path.clone() {
                self.request_delete(path, cx);
            }
            return;
        }
        if self.explorer_operation.is_some()
            && self
                .modal_field_focus
                .iter()
                .any(|focus| focus.is_focused(window))
        {
            if debug_input_enabled() && !matches!(key.as_str(), "escape" | "enter") {
                tracing::info!(key = %key, "[MODAL INPUT]");
            }
            match key.as_str() {
                _ if self.modal_completion_key(&key, cx) => {
                    cx.stop_propagation();
                    window.prevent_default();
                }
                "escape" => self.cancel_explorer_operation(cx),
                "enter" => self.confirm_explorer_operation(cx),
                "tab" => {
                    self.cycle_modal_field(event.keystroke.modifiers.shift);
                    window.focus(&self.modal_field_focus[self.explorer_modal_field as usize]);
                    cx.notify();
                    cx.stop_propagation();
                    window.prevent_default();
                }
                _ if self.modal_key_edit(&key, event.keystroke.modifiers, cx) => {
                    cx.stop_propagation();
                    window.prevent_default();
                }
                _ => {}
            }
            return;
        }
        if self.explorer_context.is_some() {
            if key == "escape" {
                if self.explorer_new_menu_open {
                    self.explorer_new_menu_open = false;
                    if debug_input_enabled() {
                        tracing::info!(selected_path = ?self.selected_path, reason = "escape", "[SUBMENU CLOSE]");
                    }
                    cx.notify();
                } else {
                    if debug_input_enabled() {
                        tracing::info!(selected_path = ?self.selected_path, reason = "escape", "[CONTEXT MENU ESCAPE]");
                    }
                    self.close_context_menu("escape", cx);
                }
                return;
            }
            let submenu_count = 7;
            let new_index = if self
                .explorer_context
                .as_ref()
                .is_some_and(|context| context.kind == EntryKind::File)
            {
                2
            } else {
                1
            };
            if self.explorer_new_menu_open {
                match key.as_str() {
                    "left" => {
                        self.explorer_new_menu_open = false;
                        if debug_input_enabled() {
                            tracing::info!(selected_path = ?self.selected_path, reason = "left", "[SUBMENU CLOSE]");
                        }
                        cx.notify();
                    }
                    "up" => {
                        self.context_submenu_selected =
                            self.context_submenu_selected.saturating_sub(1);
                        cx.notify();
                    }
                    "down" => {
                        self.context_submenu_selected =
                            (self.context_submenu_selected + 1).min(submenu_count - 1);
                        cx.notify();
                    }
                    "enter" => self.execute_new_submenu_item(window, cx),
                    _ => {}
                }
                return;
            }
            match key.as_str() {
                "up" => {
                    self.context_menu_selected = self.context_menu_selected.saturating_sub(1);
                    cx.notify();
                }
                "down" => {
                    self.context_menu_selected = (self.context_menu_selected + 1).min(7);
                    cx.notify();
                }
                "right" if self.context_menu_selected == new_index => {
                    self.open_context_submenu(cx);
                }
                "enter" if self.context_menu_selected == new_index => {
                    self.open_context_submenu(cx);
                }
                _ => {}
            }
            return;
        }
        if key == "escape" && self.open_menu.is_some() {
            let before = self.open_menu;
            self.open_menu = None;
            if debug_input_enabled() {
                tracing::info!(menu_before = ?before, menu_after = ?self.open_menu, "[MENU ESCAPE]");
            }
            cx.notify();
            return;
        }
        let (control, shift, alt) = normalize_modifiers(event.keystroke.modifiers);
        let mut stroke = String::new();
        if control {
            stroke.push_str("ctrl-");
        }
        if shift {
            stroke.push_str("shift-");
        }
        if alt {
            stroke.push_str("alt-");
        }
        stroke.push_str(&key);
        if debug_input_enabled() {
            tracing::info!(raw = %event.keystroke.key, normalized = %stroke, "[KEY NORMALIZE]");
        }
        if self.command_palette_visible {
            match key.as_str() {
                "down" => self.palette_down(&PaletteDown, window, cx),
                "up" => self.palette_up(&PaletteUp, window, cx),
                "enter" => self.palette_confirm(&PaletteConfirm, window, cx),
                "escape" => self.palette_escape(&PaletteEscape, window, cx),
                "home" => {
                    self.command_palette_selected = 0;
                    cx.notify();
                }
                "end" => {
                    self.command_palette_selected = self.palette_commands().len().saturating_sub(1);
                    cx.notify();
                }
                "backspace" => {
                    self.command_palette_query.pop();
                    self.command_palette_selected = 0;
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        if stroke == "f12" {
            self.debug_input(&DebugInput, window, cx);
            return;
        }
        if stroke == "shift-tab" {
            self.dispatch_editor_action(crate::editor_view::Outdent, window, cx);
            return;
        }
        if let Some(command) = self
            .keymap
            .commands()
            .iter()
            .find(|command| self.keymap.shortcut(&command.id) == Some(stroke.as_str()))
        {
            let id = command.id.clone();
            if debug_keys_enabled() || debug_input_enabled() {
                tracing::info!(shortcut = %stroke, result = %id, "[KEYMAP LOOKUP]");
                tracing::info!(key = %stroke, matched = %id, "[KEYMAP]");
                tracing::info!(key = %stroke, matched = %id, context = "workspace", executed = true, "Axiom key event");
            }
            self.execute_command(&id, window, cx);
        } else if debug_keys_enabled() || debug_input_enabled() {
            if debug_input_enabled() {
                tracing::info!(shortcut = %stroke, result = "none", "[KEYMAP LOOKUP]");
            }
            tracing::debug!(key = %stroke, matched = "", context = "workspace", executed = false, "Axiom key event");
        }
    }

    fn apply_captured_shortcut(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(shortcut)) = (
            self.settings_selected.clone(),
            self.captured_shortcut.clone(),
        ) else {
            return;
        };
        if let Err(conflict) = self.keymap.set_shortcut(&id, Some(shortcut.clone())) {
            self.shortcut_conflict = Some(conflict);
            cx.notify();
            return;
        }
        let _ = self.keymap.persist_user();
        self.shortcut_capture = false;
        cx.notify();
    }

    fn replace_conflicting_shortcut(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(shortcut)) = (
            self.settings_selected.clone(),
            self.captured_shortcut.clone(),
        ) else {
            return;
        };
        self.keymap.replace_shortcut(&id, Some(shortcut));
        let _ = self.keymap.persist_user();
        self.shortcut_conflict = None;
        self.shortcut_capture = false;
        cx.notify();
    }

    fn cancel_shortcut_conflict(&mut self, cx: &mut Context<Self>) {
        self.shortcut_conflict = None;
        self.captured_shortcut = None;
        cx.notify();
    }

    fn toggle_project(&mut self, _: &ToggleProject, _: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        let before = self.project_panel_visible;
        self.project_panel_visible = !self.project_panel_visible;
        if debug_input_enabled() {
            tracing::info!(
                before,
                after = self.project_panel_visible,
                "[PROJECT PANEL]"
            );
        }
        cx.notify();
    }

    fn toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        if self.terminal_visible {
            self.terminal_visible = false;
            cx.notify();
            return;
        }
        if self.terminal_view.is_none() {
            let Some(project) = &self.project else {
                self.status = "Open a project before starting a terminal".into();
                cx.notify();
                return;
            };
            match TerminalSession::spawn(project.root_path(), TerminalProfile::platform_default()) {
                Ok(session) => {
                    let session = std::sync::Arc::new(session);
                    let workspace = cx.entity().downgrade();
                    let view = cx.new(|cx| TerminalView::new(session.clone(), workspace, cx));
                    self.terminal_session = Some(session);
                    self.terminal_view = Some(view);
                }
                Err(error) => {
                    self.status = format!("Terminal failed to start: {error}").into();
                    cx.notify();
                    return;
                }
            }
        }
        self.terminal_visible = true;
        if let Some(terminal) = &self.terminal_view {
            window.focus(&terminal.read(cx).focus_handle());
        }
        cx.notify();
    }

    fn open_in_terminal(
        &mut self,
        _: &OpenInTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(context) = self.explorer_context.take() else {
            return;
        };
        let directory = Self::context_directory(&context.path, context.kind).to_path_buf();
        match TerminalSession::spawn(&directory, TerminalProfile::platform_default()) {
            Ok(session) => {
                let session = std::sync::Arc::new(session);
                let workspace = cx.entity().downgrade();
                let view = cx.new(|cx| TerminalView::new(session.clone(), workspace, cx));
                self.terminal_session = Some(session);
                self.terminal_view = Some(view);
                self.terminal_visible = true;
                self.status = format!("Terminal opened in {}", directory.display()).into();
                if let Some(terminal) = &self.terminal_view {
                    window.focus(&terminal.read(cx).focus_handle());
                }
            }
            Err(error) => self.status = format!("Terminal failed to start: {error}").into(),
        }
        cx.notify();
    }

    pub(crate) fn open_terminal_link(
        &mut self,
        link: TerminalLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match link.kind {
            TerminalLinkKind::Url => {
                if link.target.starts_with("http://") || link.target.starts_with("https://") {
                    if let Err(error) = open::that(&link.target) {
                        self.status = format!("Unable to open link: {error}").into();
                    }
                }
            }
            TerminalLinkKind::File | TerminalLinkKind::FileLine { .. } => {
                let Some(path) = link.path else { return };
                if !path.is_file() {
                    self.status = format!("File not found: {}", path.display()).into();
                    cx.notify();
                    return;
                }
                self.open_file(path.clone(), window, cx);
                if let Some(line) = match link.kind {
                    TerminalLinkKind::FileLine { line, .. } => Some(line),
                    TerminalLinkKind::File => None,
                    TerminalLinkKind::Url => None,
                } {
                    let column = match link.kind {
                        TerminalLinkKind::FileLine { column, .. } => column,
                        _ => None,
                    };
                    if let Some(index) = self.active.and_then(|index| self.tabs.get(index)) {
                        index.editor.update(cx, |editor, cx| {
                            editor.reveal_lsp_position(
                                lsp_types::Position {
                                    line: line.saturating_sub(1),
                                    character: column.unwrap_or(1).saturating_sub(1),
                                },
                                cx,
                            )
                        });
                    }
                }
            }
        }
        cx.notify();
    }

    fn toggle_directory(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.explorer.get(index).cloned() else {
            return;
        };
        if item.kind != EntryKind::Directory {
            return;
        }
        if self.expanded.remove(&item.path) {
            let prefix = item.path;
            self.explorer.retain(|candidate| {
                candidate.path == prefix || !candidate.path.starts_with(&prefix)
            });
        } else if let Some(project) = self.project.clone() {
            let directory = item.path.clone();
            let depth = item.depth + 1;
            let workspace = cx.entity().downgrade();
            cx.spawn(async move |_, cx| {
                let result = project.read_directory(&directory);
                let _ = workspace.update(cx, |this, cx| {
                    match result {
                        Ok(entries) => {
                            let children = entries.into_iter().map(|entry| ExplorerItem {
                                path: entry.path,
                                name: entry.name,
                                kind: entry.kind,
                                depth,
                            });
                            this.explorer.splice(index + 1..index + 1, children);
                            this.expanded.insert(directory.clone());
                        }
                        Err(error) => this.status = format!("Falha ao abrir pasta: {error}").into(),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn refresh_explorer(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let root = project.root_path().to_path_buf();
        let workspace = cx.entity().downgrade();
        self.status = "Refreshing Project Explorer...".into();
        self.explorer_context = None;
        cx.notify();
        cx.spawn(async move |_, cx| {
            let result = project.read_directory(&root);
            let _ = workspace.update(cx, |this, cx| {
                match result {
                    Ok(entries) => {
                        this.explorer = entries
                            .into_iter()
                            .map(|entry| ExplorerItem {
                                path: entry.path,
                                name: entry.name,
                                kind: entry.kind,
                                depth: 0,
                            })
                            .collect();
                        this.expanded.clear();
                        this.status = "Project Explorer refreshed".into();
                    }
                    Err(error) => {
                        this.status = format!("Falha ao atualizar projeto: {error}").into()
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn context_directory(path: &Path, kind: EntryKind) -> &Path {
        if kind == EntryKind::Directory {
            path
        } else {
            path.parent().unwrap_or(path)
        }
    }

    fn close_context_menu(&mut self, reason: &'static str, cx: &mut Context<Self>) {
        if self.explorer_context.take().is_some() {
            self.explorer_new_menu_open = false;
            if debug_input_enabled() {
                tracing::info!(
                    selected_path = ?self.selected_path,
                    reason,
                    "[CONTEXT MENU CLOSE]"
                );
            }
            cx.notify();
        }
    }

    fn open_context_menu(
        &mut self,
        path: PathBuf,
        kind: EntryKind,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.selected_path = Some(path.clone());
        self.context_menu_position = position;
        self.open_menu = None;
        self.context_menu_selected = 0;
        self.context_submenu_selected = 0;
        self.explorer_new_menu_open = false;
        self.explorer_context = Some(ExplorerContext { path, kind });
        if debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, "[CONTEXT MENU OPEN]");
        }
        cx.notify();
    }

    fn open_context_submenu(&mut self, cx: &mut Context<Self>) {
        if self.explorer_context.is_some() {
            self.explorer_new_menu_open = true;
            self.context_submenu_selected = 0;
            if debug_input_enabled() {
                tracing::info!(selected_path = ?self.selected_path, "[SUBMENU OPEN]");
            }
            cx.notify();
        }
    }

    fn execute_new_submenu_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.explorer_context.as_ref() else {
            return;
        };
        let directory = Self::context_directory(&context.path, context.kind).to_path_buf();
        if debug_input_enabled() {
            tracing::info!(
                selected_path = ?self.selected_path,
                index = self.context_submenu_selected,
                "[CONTEXT MENU ACTION]"
            );
            let kind = [
                "file",
                "directory",
                "php_file",
                "php_class",
                "php_interface",
                "php_trait",
                "php_enum",
            ]
            .get(self.context_submenu_selected)
            .copied()
            .unwrap_or("unknown");
            tracing::info!(kind, selected_path = ?self.selected_path, target_directory = %directory.display(), "[NEW ITEM ACTION]");
        }
        let kind = match self.context_submenu_selected {
            0 => NewItemKind::File,
            1 => NewItemKind::Directory,
            2 => NewItemKind::PhpFile,
            3 => NewItemKind::PhpClass,
            4 => NewItemKind::PhpInterface,
            5 => NewItemKind::PhpTrait,
            6 => NewItemKind::PhpEnum,
            _ => return,
        };
        self.begin_new_item(kind, directory, window, cx);
    }

    fn new_file(&mut self, directory: PathBuf, _: &mut Window, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if self.explorer_context.is_some() && debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, reason = "action", "[CONTEXT MENU CLOSE]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_undo.clear();
        self.explorer_input = "untitled".into();
        self.explorer_selection = UTF16Selection {
            range: 0..self.explorer_input.encode_utf16().count(),
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::NewFile(directory));
        cx.notify();
    }

    fn begin_new_item(
        &mut self,
        kind: NewItemKind,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if debug_input_enabled() {
            tracing::info!(?kind, target_directory = %directory.display(), "[NEW ITEM ACTION]");
        }
        match kind {
            NewItemKind::File => self.new_file(directory, window, cx),
            NewItemKind::Directory => self.new_directory(directory, cx),
            NewItemKind::PhpFile => self.new_php_file(directory, cx),
            NewItemKind::PhpClass => self.new_php_item(directory, "class", cx),
            NewItemKind::PhpInterface => self.new_php_item(directory, "interface", cx),
            NewItemKind::PhpTrait => self.new_php_item(directory, "trait", cx),
            NewItemKind::PhpEnum => self.new_php_item(directory, "enum", cx),
        }
    }

    fn open_new_menu(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.selected_path = Some(directory.clone());
        self.context_menu_position = Point::new(px(34.), px(70.));
        self.explorer_context = Some(ExplorerContext {
            path: directory,
            kind: EntryKind::Directory,
        });
        self.explorer_new_menu_open = true;
        if debug_input_enabled() {
            tracing::info!("[EXPLORER TOOLBAR CLICK] button=new");
            tracing::info!(command = "project.new", "[ACTION]");
        }
        cx.notify();
    }

    fn new_php_file(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if self.explorer_context.is_some() && debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, reason = "action", "[CONTEXT MENU CLOSE]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_undo.clear();
        self.explorer_input = "untitled.php".into();
        self.explorer_selection = UTF16Selection {
            range: 0..self.explorer_input.encode_utf16().count(),
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::NewPhpFile(directory));
        cx.notify();
    }

    fn new_directory(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if self.explorer_context.is_some() && debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, reason = "action", "[CONTEXT MENU CLOSE]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_undo.clear();
        self.explorer_input = "New Folder".into();
        self.explorer_selection = UTF16Selection {
            range: 0..self.explorer_input.encode_utf16().count(),
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::NewDirectory(directory));
        cx.notify();
    }

    fn new_php_item(&mut self, directory: PathBuf, keyword: &'static str, cx: &mut Context<Self>) {
        self.modal_type_items.clear();
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if self.explorer_context.is_some() && debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, reason = "action", "[CONTEXT MENU CLOSE]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_undo.clear();
        self.explorer_input = "NewItem".into();
        self.explorer_file = "NewItem.php".into();
        self.explorer_file_auto = true;
        self.explorer_selection = UTF16Selection {
            range: 0..self.explorer_input.encode_utf16().count(),
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace = self
            .project
            .as_ref()
            .and_then(|project| project.path_to_namespace(&directory))
            .unwrap_or_default();
        self.explorer_extends.clear();
        self.explorer_implements.clear();
        self.explorer_operation = Some(ExplorerOperation::NewPhp { directory, keyword });
        cx.notify();
    }

    fn select_php_type(&mut self, keyword: &'static str, cx: &mut Context<Self>) {
        self.modal_type_items.clear();
        if let Some(ExplorerOperation::NewPhp {
            keyword: current, ..
        }) = self.explorer_operation.as_mut()
        {
            *current = keyword;
            if !matches!(keyword, "class" | "interface") {
                self.explorer_extends.clear();
                self.explorer_implements.clear();
            }
            if (self.explorer_modal_field == ModalField::Implements && keyword != "class")
                || (self.explorer_modal_field == ModalField::Extends
                    && !matches!(keyword, "class" | "interface"))
            {
                self.set_modal_field(ModalField::Name);
            }
            cx.notify();
        }
    }

    fn modal_type_context(&self) -> Option<crate::editor_view::TypeCompletionContext> {
        use crate::editor_view::TypeCompletionContext::*;
        match (&self.explorer_operation, self.explorer_modal_field) {
            (
                Some(ExplorerOperation::NewPhp {
                    keyword: "class", ..
                }),
                ModalField::Extends,
            ) => Some(ClassExtends),
            (
                Some(ExplorerOperation::NewPhp {
                    keyword: "class", ..
                }),
                ModalField::Implements,
            ) => Some(ClassImplements),
            (
                Some(ExplorerOperation::NewPhp {
                    keyword: "interface",
                    ..
                }),
                ModalField::Extends,
            ) => Some(InterfaceExtends),
            _ => None,
        }
    }

    fn refresh_modal_types(&mut self) {
        self.modal_type_items.clear();
        let Some(context) = self.modal_type_context() else {
            return;
        };
        let field = self.explorer_modal_field;
        let text = self.modal_field_text(field);
        let caret = utf16_to_byte_offset(text, self.modal_field_selection(field).range.end);
        let (range, prefix) = crate::modal_type_completion::active_token(text, caret);
        // A busy index is skipped; never wait on the UI thread.
        let project = self
            .project_index
            .as_ref()
            .and_then(|index| index.try_read().ok());
        let vendor = self
            .vendor_index
            .as_ref()
            .and_then(|index| index.try_read().ok());
        let items = crate::modal_type_completion::lookup(
            context,
            prefix,
            project.as_deref(),
            vendor.as_deref(),
            self._runtime_symbols.as_deref(),
        );
        self.modal_type_items = items;
        self.modal_type_range = range;
        self.modal_type_selected = 0;
        self.modal_type_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
    }

    fn accept_modal_type(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.modal_type_context().is_none() {
            return;
        }
        let Some(fqn) = self
            .modal_type_items
            .get(index)
            .and_then(|item| item.insert_text.clone())
        else {
            return;
        };
        let field = self.explorer_modal_field;
        let text = self.modal_field_text(field);
        let range = byte_to_utf16_offset(text, self.modal_type_range.start)
            ..byte_to_utf16_offset(text, self.modal_type_range.end);
        self.modal_replace_field_range(field, range, &fqn, cx);
        self.modal_type_items.clear();
        cx.notify();
    }

    fn modal_completion_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if self.modal_type_items.is_empty() {
            return false;
        }
        match key {
            "up" => {
                self.modal_type_selected = (self.modal_type_selected + self.modal_type_items.len()
                    - 1)
                    % self.modal_type_items.len()
            }
            "down" => {
                self.modal_type_selected =
                    (self.modal_type_selected + 1) % self.modal_type_items.len()
            }
            "enter" => {
                self.accept_modal_type(self.modal_type_selected, cx);
                return true;
            }
            "escape" => self.modal_type_items.clear(),
            "tab" => {
                self.modal_type_items.clear();
                return false;
            }
            _ => return false,
        }
        self.modal_type_scroll
            .scroll_to_item(self.modal_type_selected);
        cx.notify();
        true
    }

    fn cycle_modal_field(&mut self, backwards: bool) {
        let fields = match self.explorer_operation {
            Some(ExplorerOperation::NewPhp {
                keyword: "class", ..
            }) => vec![
                ModalField::Name,
                ModalField::Namespace,
                ModalField::File,
                ModalField::Extends,
                ModalField::Implements,
            ],
            Some(ExplorerOperation::NewPhp {
                keyword: "interface",
                ..
            }) => vec![
                ModalField::Name,
                ModalField::Namespace,
                ModalField::File,
                ModalField::Extends,
            ],
            Some(ExplorerOperation::NewPhp { .. }) => {
                vec![ModalField::Name, ModalField::Namespace, ModalField::File]
            }
            _ => vec![ModalField::Name],
        };
        let index = fields
            .iter()
            .position(|field| *field == self.explorer_modal_field)
            .unwrap_or(0);
        let next = if backwards {
            index.checked_sub(1).unwrap_or(fields.len() - 1)
        } else {
            (index + 1) % fields.len()
        };
        self.set_modal_field(fields[next]);
    }

    fn rename_entry(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if debug_input_enabled() {
            tracing::info!(selected_path = %path.display(), popup_open = true, "[RENAME DIALOG]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        let Some(current_name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
        else {
            return;
        };
        self.explorer_input = current_name;
        self.explorer_undo.clear();
        let basename_len = self
            .explorer_input
            .rsplit_once('.')
            .filter(|(_, extension)| !extension.is_empty())
            .map(|(basename, _)| basename.encode_utf16().count())
            .unwrap_or_else(|| self.explorer_input.encode_utf16().count());
        self.explorer_selection = UTF16Selection {
            range: 0..basename_len,
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::Rename(path));
        cx.notify();
    }

    fn cancel_explorer_operation(&mut self, cx: &mut Context<Self>) {
        self.modal_type_items.clear();
        self.explorer_operation = None;
        self.explorer_new_menu_open = false;
        self.modal_focus_pending = false;
        self.explorer_input.clear();
        self.explorer_file.clear();
        self.explorer_file_auto = true;
        self.explorer_undo.clear();
        self.explorer_namespace.clear();
        self.explorer_extends.clear();
        self.explorer_implements.clear();
        cx.notify();
    }

    fn modal_field_text(&self, field: ModalField) -> &str {
        match field {
            ModalField::Name => &self.explorer_input,
            ModalField::Namespace => &self.explorer_namespace,
            ModalField::File => &self.explorer_file,
            ModalField::Extends => &self.explorer_extends,
            ModalField::Implements => &self.explorer_implements,
        }
    }

    fn modal_field_text_mut(&mut self, field: ModalField) -> &mut String {
        match field {
            ModalField::Name => &mut self.explorer_input,
            ModalField::Namespace => &mut self.explorer_namespace,
            ModalField::File => &mut self.explorer_file,
            ModalField::Extends => &mut self.explorer_extends,
            ModalField::Implements => &mut self.explorer_implements,
        }
    }

    fn modal_field_selection(&self, field: ModalField) -> &UTF16Selection {
        match field {
            ModalField::Name => &self.explorer_selection,
            ModalField::Namespace => &self.explorer_namespace_selection,
            ModalField::File => &self.explorer_file_selection,
            ModalField::Extends => &self.explorer_extends_selection,
            ModalField::Implements => &self.explorer_implements_selection,
        }
    }

    fn modal_field_selection_mut(&mut self, field: ModalField) -> &mut UTF16Selection {
        match field {
            ModalField::Name => &mut self.explorer_selection,
            ModalField::Namespace => &mut self.explorer_namespace_selection,
            ModalField::File => &mut self.explorer_file_selection,
            ModalField::Extends => &mut self.explorer_extends_selection,
            ModalField::Implements => &mut self.explorer_implements_selection,
        }
    }

    fn set_modal_field(&mut self, field: ModalField) {
        self.modal_type_items.clear();
        self.explorer_modal_field = field;
        self.reset_modal_caret();
    }

    fn modal_caret_for(&self, field: ModalField, window: &Window) -> bool {
        self.modal_caret_visible
            && self.explorer_modal_field == field
            && self.modal_field_focus[field as usize].is_focused(window)
    }

    fn set_modal_field_at(&mut self, field: ModalField, byte: usize) {
        self.set_modal_field(field);
        let caret = byte_to_utf16_offset(
            self.modal_field_text(field),
            byte.min(self.modal_field_text(field).len()),
        );
        *self.modal_field_selection_mut(field) = UTF16Selection {
            range: caret..caret,
            reversed: false,
        };
    }

    fn modal_pointer_down(
        &mut self,
        field: ModalField,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let byte = self.modal_field_geometry[field as usize].hit_test(event.position.x);
        self.set_modal_field_at(field, byte);
        if event.click_count == 2 {
            let text = self.modal_field_text(field);
            let range = axiom_app::interaction::word_range_at(text, byte);
            let range =
                byte_to_utf16_offset(text, range.start)..byte_to_utf16_offset(text, range.end);
            self.modal_field_selection_mut(field).range = range;
        }
        window.focus(&self.modal_field_focus[field as usize]);
        cx.notify();
    }

    fn modal_pointer_move(
        &mut self,
        field: ModalField,
        event: &gpui::MouseMoveEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging()
            || self.explorer_modal_field != field
            || !self.modal_field_focus[field as usize].is_focused(window)
        {
            return;
        }
        let byte = self.modal_field_geometry[field as usize].hit_test(event.position.x);
        let caret = byte_to_utf16_offset(self.modal_field_text(field), byte);
        let selection = self.modal_field_selection_mut(field);
        let anchor = if selection.reversed {
            selection.range.end
        } else {
            selection.range.start
        };
        *selection = UTF16Selection {
            range: anchor.min(caret)..anchor.max(caret),
            reversed: caret < anchor,
        };
        self.reset_modal_caret();
        cx.notify();
    }
    fn modal_replace_range(
        &mut self,
        range: std::ops::Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let field = self.explorer_modal_field;
        self.modal_replace_field_range(field, range, text, cx);
    }

    fn modal_replace_field_range(
        &mut self,
        field: ModalField,
        range: std::ops::Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if field != ModalField::Name {
            let current = self.modal_field_text(field).to_owned();
            let start = range.start.min(current.encode_utf16().count());
            let end = range.end.min(current.encode_utf16().count());
            let (updated, caret) = replace_utf16_range(&current, start..end, text);
            *self.modal_field_text_mut(field) = updated;
            *self.modal_field_selection_mut(field) = UTF16Selection {
                range: caret..caret,
                reversed: false,
            };

            if field == ModalField::File {
                self.explorer_file_auto = false;
            }
            if matches!(field, ModalField::Extends | ModalField::Implements)
                && field == self.explorer_modal_field
            {
                self.refresh_modal_types();
            }
            self.reset_modal_caret();
            cx.notify();
            return;
        }
        self.reset_modal_caret();
        if self
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| matches!(operation, ExplorerOperation::Rename(_)))
        {
            self.explorer_undo.push((
                self.explorer_input.clone(),
                UTF16Selection {
                    range: self.explorer_selection.range.clone(),
                    reversed: self.explorer_selection.reversed,
                },
            ));
        }
        let before = self.explorer_input.encode_utf16().count();
        let (updated, caret) = replace_utf16_range(&self.explorer_input, range.clone(), text);
        self.explorer_input = updated;
        if self.explorer_file_auto
            && self
                .explorer_operation
                .as_ref()
                .is_some_and(|op| matches!(op, ExplorerOperation::NewPhp { .. }))
        {
            self.explorer_file = format!("{}.php", self.explorer_input.trim_end_matches(".php"));
        }
        self.explorer_selection = UTF16Selection {
            range: caret..caret,
            reversed: false,
        };
        if debug_input_enabled() {
            tracing::info!(
                kind = if self
                    .explorer_operation
                    .as_ref()
                    .is_some_and(|op| matches!(op, ExplorerOperation::Rename(_)))
                {
                    "rename"
                } else {
                    "explorer"
                },
                range_start = range.start,
                range_end = range.end,
                inserted_len = text.encode_utf16().count(),
                "[MODAL REPLACE TEXT]"
            );
            if text.is_empty() {
                tracing::info!(
                    range_start = range.start,
                    range_end = range.end,
                    "[MODAL DELETE]"
                );
            }
            tracing::info!(
                value_len_before = before,
                value_len_after = self.explorer_input.encode_utf16().count(),
                changed = before != self.explorer_input.encode_utf16().count(),
                "[MODAL STATE]"
            );
        }
        cx.notify();
    }

    fn reset_modal_caret(&mut self) {
        self.modal_caret_visible = true;
        let now = Instant::now();
        self.modal_caret_activity = now;
        self.modal_caret_toggle = now;
    }

    fn modal_key_edit(&mut self, key: &str, modifiers: Modifiers, cx: &mut Context<Self>) -> bool {
        if matches!(key, "left" | "right" | "home" | "end") || (modifiers.control && key == "a") {
            self.modal_type_items.clear();
        }
        let field = self.explorer_modal_field;
        self.reset_modal_caret();
        let length = self.modal_field_text(field).encode_utf16().count();
        let start = self
            .modal_field_selection(field)
            .range
            .start
            .min(self.modal_field_selection(field).range.end);
        let end = self
            .modal_field_selection(field)
            .range
            .start
            .max(self.modal_field_selection(field).range.end);
        if modifiers.control
            && key == "z"
            && self
                .explorer_operation
                .as_ref()
                .is_some_and(|operation| matches!(operation, ExplorerOperation::Rename(_)))
        {
            if let Some((value, selection)) = self.explorer_undo.pop() {
                self.explorer_input = value;
                *self.modal_field_selection_mut(field) = selection;
                if debug_input_enabled() {
                    tracing::info!(
                        selection = ?self.modal_field_selection(field).range,
                        "[RENAME UNDO]"
                    );
                }
                cx.notify();
            }
            return true;
        }
        if modifiers.control && key == "a" {
            *self.modal_field_selection_mut(field) = UTF16Selection {
                range: 0..length,
                reversed: false,
            };
            cx.notify();
            return true;
        }
        if modifiers.control && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.modal_replace_range(start..end, &text, cx);
            }
            return true;
        }
        if modifiers.control && matches!(key, "c" | "x") {
            if start != end {
                let query = self.modal_field_text(field);
                let text = query
                    [utf16_to_byte_offset(query, start)..utf16_to_byte_offset(query, end)]
                    .to_owned();
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                if key == "x" {
                    self.modal_replace_range(start..end, "", cx);
                }
            }
            return true;
        }
        if key == "backspace" {
            if start != end {
                self.modal_replace_range(start..end, "", cx);
            } else if start > 0 {
                let previous = crate::ui::input_line::adjacent_utf16(
                    self.modal_field_text(field),
                    start,
                    false,
                );
                self.modal_replace_range(previous..start, "", cx);
            }
            return true;
        }
        if key == "delete" {
            if start != end {
                self.modal_replace_range(start..end, "", cx);
            } else if end < length {
                let next =
                    crate::ui::input_line::adjacent_utf16(self.modal_field_text(field), end, true);
                self.modal_replace_range(end..next, "", cx);
            }
            return true;
        }
        if matches!(key, "left" | "right") {
            let next = if key == "left" {
                if start != end {
                    start
                } else {
                    crate::ui::input_line::adjacent_utf16(
                        self.modal_field_text(field),
                        start,
                        false,
                    )
                }
            } else {
                if start != end {
                    end
                } else {
                    crate::ui::input_line::adjacent_utf16(self.modal_field_text(field), end, true)
                }
            };
            *self.modal_field_selection_mut(field) = UTF16Selection {
                range: next..next,
                reversed: false,
            };
            cx.notify();
            return true;
        }
        false
    }

    fn confirm_explorer_operation(&mut self, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        let Some(operation) = self.explorer_operation.as_ref() else {
            return;
        };
        let name = self.explorer_input.trim().to_owned();
        let is_php = matches!(
            operation,
            ExplorerOperation::NewPhp { .. } | ExplorerOperation::NewPhpFile(_)
        );
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
            self.status = "Name must be a valid PHP identifier".into();
            cx.notify();
            return;
        }
        if is_php
            && (!valid_php_identifier(name.trim_end_matches(".php"))
                || !valid_php_namespace(&self.explorer_namespace))
        {
            self.status = "Invalid PHP name or namespace".into();
            cx.notify();
            return;
        }
        let Some(operation) = self.explorer_operation.take() else {
            return;
        };
        self.modal_focus_pending = false;
        self.explorer_input.clear();
        self.explorer_undo.clear();
        let Some(project) = self.project.clone() else {
            return;
        };
        let operation = match operation {
            ExplorerOperation::NewFile(directory) => (directory, name, None),
            ExplorerOperation::NewPhpFile(directory) => {
                let name = if self.explorer_file.ends_with(".php") {
                    self.explorer_file.trim().to_owned()
                } else {
                    format!("{}.php", self.explorer_file.trim())
                };
                (directory, name, None)
            }
            ExplorerOperation::NewPhp { directory, keyword } => {
                let name = if name.ends_with(".php") {
                    name
                } else {
                    format!("{name}.php")
                };
                let symbol = name.trim_end_matches(".php");
                let body = crate::php_type_template::render(
                    keyword,
                    symbol,
                    &self.explorer_namespace,
                    &self.explorer_extends,
                    &self.explorer_implements,
                );
                (directory, name, Some(body))
            }
            ExplorerOperation::NewDirectory(directory) => {
                self.explorer_fs_busy = true;
                cx.spawn(async move |this, cx| {
                    let result = gpui::background_executor()
                        .spawn(async move {
                            axiom_index::trace_path(
                                "explorer_create_directory",
                                "Create",
                                &directory,
                            );
                            project
                                .create_directory(&directory, &name)
                                .map(|_| ExplorerFsResult::Create(None))
                                .map_err(|e| e.to_string())
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.explorer_fs_busy = false;
                        match result {
                            Ok(_) => {
                                this.refresh_explorer(cx);
                                this.status = "Directory created".into();
                            }
                            Err(error) => this.status = format!("Operation failed: {error}").into(),
                        }
                        cx.notify();
                    });
                })
                .detach();
                return;
            }
            ExplorerOperation::Rename(path) => {
                self.explorer_fs_busy = true;
                cx.spawn(async move |this, cx| {
                    let result = gpui::background_executor()
                        .spawn(async move {
                            axiom_index::trace_path("explorer_rename", "Rename", &path);
                            project
                                .rename(&path, &name)
                                .map(|new| ExplorerFsResult::Rename { old: path, new })
                                .map_err(|e| e.to_string())
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.explorer_fs_busy = false;
                        match result {
                            Ok(ExplorerFsResult::Rename { old, new }) => {
                                let dirty_contents: HashMap<PathBuf, String> = this
                                    .tabs
                                    .iter()
                                    .filter(|tab| tab.path.starts_with(&old))
                                    .filter_map(|tab| {
                                        tab.editor.read(cx).is_dirty().then(|| {
                                            (
                                                tab.path.clone(),
                                                tab.editor.read(cx).document_content(),
                                            )
                                        })
                                    })
                                    .collect();
                                let semantic_changes = this
                                    .project_index
                                    .as_ref()
                                    .map(|index| {
                                        let paths = index
                                            .read()
                                            .map(|index| {
                                                index
                                                    .indexed_files()
                                                    .map(Path::to_path_buf)
                                                    .collect::<Vec<_>>()
                                            })
                                            .unwrap_or_default();
                                        paths
                                            .into_iter()
                                            .filter(|path| path.starts_with(&old))
                                            .filter_map(|path| {
                                                let relative = path.strip_prefix(&old).ok()?;
                                                let new_path = new.join(relative);
                                                let text =
                                                    dirty_contents.get(&path).cloned().or_else(
                                                        || fs::read_to_string(&new_path).ok(),
                                                    )?;
                                                Some((path, Some((new_path, text))))
                                            })
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                for tab in &mut this.tabs {
                                    if let Ok(relative) = tab.path.strip_prefix(&old) {
                                        tab.path = new.join(relative);
                                        tab.editor.update(cx, |editor, _| {
                                            editor.relocate_path(&old, &new)
                                        });
                                    }
                                }
                                this.schedule_semantic_fs_batch(semantic_changes, cx);
                                this.refresh_explorer(cx);
                                this.status = "Renamed".into();
                            }
                            _ => this.status = "Operation failed".into(),
                        }
                        cx.notify();
                    });
                })
                .detach();
                return;
            }
        };
        self.explorer_fs_busy = true;
        cx.spawn(async move |this, cx| {
            let result = gpui::background_executor()
                .spawn(async move {
                    axiom_index::trace_path("explorer_create_file", "Create", &operation.0);
                    let path = project
                        .create_file(&operation.0, &operation.1)
                        .map_err(|e| e.to_string())?;
                    if let Some(body) = operation.2 {
                        fs::write(&path, body).map_err(|e| e.to_string())?;
                    }
                    Ok::<_, String>(ExplorerFsResult::Create(Some(path)))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.explorer_fs_busy = false;
                match result {
                    Ok(ExplorerFsResult::Create(Some(path))) => {
                        this.refresh_explorer(cx);
                        this.open_file_background(path, cx);
                    }
                    Ok(_) => this.refresh_explorer(cx),
                    Err(error) => this.status = format!("Operation failed: {error}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn request_delete(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if debug_input_enabled() {
            tracing::info!(path = %path.display(), "[EXPLORER ACTION] action=delete");
        }
        self.explorer_context = None;
        if self
            .tabs
            .iter()
            .any(|tab| tab.path.starts_with(&path) && tab.editor.read(cx).is_dirty())
        {
            self.status = "Save or close modified files before deleting".into();
        } else {
            self.pending_delete_is_directory = fs::metadata(&path)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            self.pending_delete = Some(path);
            self.delete_focus_pending = true;
            self.status = "Confirm deletion".into();
            if debug_input_enabled() {
                tracing::info!(confirmation_open = true, "[DELETE]");
            }
        }
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        let Some(path) = self.pending_delete.take() else {
            return;
        };
        self.delete_focus_pending = false;
        self.pending_delete_is_directory = false;
        let Some(project) = self.project.clone() else {
            self.status = "No project open".into();
            cx.notify();
            return;
        };
        self.explorer_fs_busy = true;
        cx.spawn(async move |this, cx| {
            let result = gpui::background_executor()
                .spawn(async move {
                    axiom_index::trace_path("explorer_delete", "Other", &path);
                    project
                        .delete(&path)
                        .map(|_| ExplorerFsResult::Delete(path))
                        .map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.explorer_fs_busy = false;
                match result {
                    Ok(ExplorerFsResult::Delete(path)) => {
                        if debug_input_enabled() {
                            tracing::info!(success = true, "[DELETE RESULT]");
                        }
                        for tab in this.tabs.iter().filter(|tab| tab.path.starts_with(&path)) {
                            tab.editor.read(cx).close_lsp_document();
                        }
                        let semantic_changes = this
                            .project_index
                            .as_ref()
                            .map(|index| {
                                let paths = index
                                    .read()
                                    .map(|index| {
                                        index
                                            .indexed_files()
                                            .map(Path::to_path_buf)
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                paths
                                    .into_iter()
                                    .filter(|candidate| candidate.starts_with(&path))
                                    .map(|candidate| (candidate, None))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        this.schedule_semantic_fs_batch(semantic_changes, cx);
                        this.tabs.retain(|tab| !tab.path.starts_with(&path));
                        this.active = if this.tabs.is_empty() {
                            None
                        } else {
                            Some(this.active.unwrap_or(0).min(this.tabs.len() - 1))
                        };
                        this.refresh_explorer(cx);
                        this.status = "Entry deleted".into();
                    }
                    Err(error) => {
                        if debug_input_enabled() {
                            tracing::info!(success = false, error = %error, "[DELETE RESULT]");
                        }
                        this.status = format!("Falha ao excluir: {error}").into();
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn copy_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
        self.explorer_context = None;
        self.status = "Path copied".into();
        cx.notify();
    }

    fn open_file_background(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let path = match fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) => {
                self.status = format!("Falha ao normalizar arquivo: {error}").into();
                cx.notify();
                return;
            }
        };
        if let Some(index) = self.tabs.iter().position(|tab| tab.path == path) {
            self.active = Some(index);
            self.focus_active_editor = true;
            cx.notify();
            return;
        }
        axiom_index::trace_path("document_load_request", "Other", &path);
        let document = match Document::from_file(&path) {
            Ok(document) => document,
            Err(error) => {
                self.status = format!("Falha ao abrir arquivo: {error}").into();
                cx.notify();
                return;
            }
        };
        let lsp = self.lsp.clone();
        let editor = cx.new(|cx| EditorView::from_document(path.clone(), document, lsp, cx));
        if let Some(symbols) = &self._runtime_symbols {
            editor.update(cx, |editor, _| editor.set_runtime_symbols(symbols.clone()));
        }
        if let Some(index) = &self.project_index {
            editor.update(cx, |editor, _| {
                let root = self
                    .project
                    .as_ref()
                    .map(|p| p.root_path().to_path_buf())
                    .unwrap_or_default();
                let workspace_source = axiom_index::is_workspace_source_lexical(&path, &root);
                editor.set_workspace_root(root, workspace_source)
            });
            editor.update(cx, |editor, editor_cx| {
                editor.set_project_symbols(index.clone(), editor_cx)
            });
            editor.update(cx, |editor, _| {
                editor.set_semantic_update_sender(
                    self.semantic_update_sender.clone(),
                    self.project_semantic_generation,
                )
            });
            if let Some(vendor) = &self.vendor_index {
                editor.update(cx, |editor, _| editor.set_vendor_symbols(vendor.clone()));
            }
        }
        if let Some(engine) = &self.semantic_engine {
            editor.update(cx, |editor, editor_cx| {
                editor.set_semantic_engine(engine.clone(), editor_cx);
            });
        }
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        self.tabs.push(OpenTab { path, editor });
        self.active = Some(self.tabs.len() - 1);
        self.focus_active_editor = true;
        cx.notify();
    }

    fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(message) = match read_file_content(&path) {
            Ok(FileContent::Text(_)) => Ok(()),
            Ok(FileContent::Binary) => Err("Binary file — preview not supported".to_owned()),
            Ok(FileContent::UnsupportedEncoding) => {
                Err("Unsupported text encoding — file not opened".to_owned())
            }
            Err(error) => Err(format!("Falha ao ler arquivo: {error}")),
        } {
            self.status = message.into();
            cx.notify();
            return;
        }
        let path = match fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) => {
                self.status = format!("Falha ao normalizar arquivo: {error}").into();
                cx.notify();
                return;
            }
        };
        if let Some(index) = self.tabs.iter().position(|tab| tab.path == path) {
            self.activate(index, window, cx);
            return;
        }
        axiom_index::trace_path("document_load_request", "Other", &path);
        let document = match Document::from_file(&path) {
            Ok(document) => document,
            Err(error) => {
                self.status = format!("Falha ao abrir arquivo: {error}").into();
                cx.notify();
                return;
            }
        };
        let lsp = self.lsp.clone();
        let editor = cx.new(|cx| EditorView::from_document(path.clone(), document, lsp, cx));
        if let Some(symbols) = &self._runtime_symbols {
            editor.update(cx, |editor, _| editor.set_runtime_symbols(symbols.clone()));
        }
        if let Some(index) = &self.project_index {
            editor.update(cx, |editor, _| {
                let root = self
                    .project
                    .as_ref()
                    .map(|p| p.root_path().to_path_buf())
                    .unwrap_or_default();
                let workspace_source = axiom_index::is_workspace_source_lexical(&path, &root);
                editor.set_workspace_root(root, workspace_source)
            });
            editor.update(cx, |editor, editor_cx| {
                editor.set_project_symbols(index.clone(), editor_cx)
            });
            editor.update(cx, |editor, _| {
                editor.set_semantic_update_sender(
                    self.semantic_update_sender.clone(),
                    self.project_semantic_generation,
                )
            });
            if let Some(vendor) = &self.vendor_index {
                editor.update(cx, |editor, _| editor.set_vendor_symbols(vendor.clone()));
            }
        }
        if let Some(engine) = &self.semantic_engine {
            editor.update(cx, |editor, editor_cx| {
                editor.set_semantic_engine(engine.clone(), editor_cx);
            });
        }
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        self.tabs.push(OpenTab { path, editor });
        self.activate(self.tabs.len() - 1, window, cx);
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let started = std::time::Instant::now();
        if let Some(tab) = self.tabs.get(index) {
            let title = tab.editor.read(cx).title();
            self.active = Some(index);
            let focus_before = tab.editor.read(cx).focus_handle(cx).is_focused(window);
            window.focus(&tab.editor.read(cx).focus_handle(cx));
            let focus_after = tab.editor.read(cx).focus_handle(cx).is_focused(window);
            if debug_input_enabled() {
                tracing::info!(
                    path = %tab.path.display(),
                    focus_before,
                    focus_after,
                    ready_for_input = focus_after,
                    "[EDITOR ACTIVATE]"
                );
            }
            cx.notify();
            let activation = started.elapsed();
            window.on_next_frame(move |_, _| {
                tracing::debug!(
                    target: "axiom::tab_switch",
                    file = %title,
                    activation_us = activation.as_micros(),
                    first_frame_us = started.elapsed().as_micros(),
                    syntax_us = 0_u64,
                    lsp_us = 0_u64,
                    "resident tab activated"
                );
            });
        }
    }

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if tab.editor.read(cx).is_dirty() {
            self.status = format!(
                "{} possui alterações; salve antes de fechar",
                tab.editor.read(cx).title()
            )
            .into();
            cx.notify();
            return;
        }
        tab.editor.read(cx).close_lsp_document();
        self.tabs.remove(index);
        self.active = match self.active {
            None => None,
            Some(_) if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(index.min(self.tabs.len() - 1)),
            active => active,
        };
        cx.notify();
    }

    fn poll_lsp(&mut self, cx: &mut Context<Self>) {
        let _stage = crate::editor_view::UiStageGuard::new(crate::editor_view::UI_STAGE_POLL_LSP);
        let Some(lsp) = self.lsp.clone() else { return };
        let events = lsp.drain_events();
        if events.is_empty() {
            return;
        }
        for event in events {
            match event {
                IdeLspEvent::Diagnostics { params, session } => {
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&params.uri))
                    {
                        tab.editor.update(cx, |editor, cx| {
                            if editor.document_session() == session {
                                editor.set_diagnostics(params.version, params.diagnostics, cx)
                            }
                        });
                    }
                }
                IdeLspEvent::Completion {
                    uri,
                    items,
                    generation,
                } => {
                    if !self.accept_lsp_generation(&uri, LspRequestKind::Completion, generation) {
                        continue;
                    }
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&uri))
                    {
                        tab.editor
                            .update(cx, |editor, cx| editor.set_completions(items, cx));
                    }
                }
                IdeLspEvent::Formatting {
                    uri,
                    edits,
                    generation,
                    document_session,
                    document_revision,
                } => {
                    if !self.accept_lsp_generation(&uri, LspRequestKind::Formatting, generation) {
                        continue;
                    }
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&uri))
                    {
                        tab.editor.update(cx, |editor, cx| {
                            editor.apply_formatting(&edits, document_session, document_revision, cx)
                        });
                    }
                }
                IdeLspEvent::SignatureHelp {
                    uri,
                    text,
                    generation,
                } => {
                    if !self.accept_lsp_generation(&uri, LspRequestKind::SignatureHelp, generation)
                    {
                        continue;
                    }
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&uri))
                    {
                        tab.editor
                            .update(cx, |editor, cx| editor.set_signature_help(text, cx));
                    }
                }
                IdeLspEvent::Hover {
                    uri,
                    text,
                    generation,
                } => {
                    if !self.accept_lsp_generation(&uri, LspRequestKind::Hover, generation) {
                        continue;
                    }
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&uri))
                    {
                        tab.editor
                            .update(cx, |editor, cx| editor.set_hover(text, cx));
                    }
                }
                IdeLspEvent::Definition {
                    uri,
                    locations,
                    generation,
                } => {
                    if !self.accept_lsp_generation(&uri, LspRequestKind::Definition, generation) {
                        continue;
                    }
                    self.definition_targets = locations
                        .iter()
                        .filter_map(|location| {
                            uri_to_path(&location.uri)
                                .ok()
                                .map(|path| DefinitionTarget {
                                    path,
                                    position: location.range.start,
                                })
                        })
                        .collect();
                    if let Some(location) = locations.into_iter().next() {
                        match uri_to_path(&location.uri) {
                            Ok(path) => self.navigate_to_definition(
                                DefinitionTarget {
                                    path,
                                    position: location.range.start,
                                },
                                cx,
                            ),
                            Err(error) => {
                                self.status = format!("Definition inválida: {error}").into()
                            }
                        }
                    } else {
                        let native = self
                            .active
                            .and_then(|index| self.tabs.get(index))
                            .and_then(|tab| tab.editor.read(cx).native_definition_location());
                        if let Some((path, position)) = native {
                            self.navigate_to_definition(DefinitionTarget { path, position }, cx);
                        } else {
                            self.status = "Definition não encontrada".into();
                        }
                    }
                }
                IdeLspEvent::Error(error) => {
                    self.status = format!("Language Server: {error}").into();
                }
                IdeLspEvent::Stopped => self.status = "Language Server: Stopped".into(),
            }
        }
        cx.notify();
    }

    fn navigate_to_definition(&mut self, target: DefinitionTarget, cx: &mut Context<Self>) {
        self.navigate_to_definition_preloaded(target, None, cx);
    }

    fn navigate_to_definition_preloaded(
        &mut self,
        target: DefinitionTarget,
        preloaded: Option<Document>,
        cx: &mut Context<Self>,
    ) {
        let path = match preloaded
            .as_ref()
            .map(|_| target.path.clone())
            .map(Ok)
            .unwrap_or_else(|| fs::canonicalize(&target.path))
        {
            Ok(path) => path,
            Err(error) => {
                self.status = format!("Falha ao abrir definition: {error}").into();
                return;
            }
        };
        let origin = self
            .active
            .and_then(|index| self.tabs.get(index))
            .and_then(|tab| {
                tab.editor
                    .read(cx)
                    .current_lsp_position()
                    .map(|position| NavigationLocation {
                        path: tab.path.clone(),
                        position,
                    })
            });
        let same_location = origin.as_ref().is_some_and(|origin| {
            fs::canonicalize(&origin.path).ok().as_ref() == Some(&path)
                && origin.position == target.position
        });
        if let Some(origin) = origin
            && !same_location
        {
            self.navigation_back.push(origin);
            self.navigation_forward.clear();
        }
        let existing = self.tabs.iter().position(|tab| tab.path == path);
        if debug_input_enabled() {
            tracing::info!(existing = existing.is_some(), path = %path.display(), "[NAVIGATION TAB]");
        }
        let index = if let Some(index) = existing {
            index
        } else {
            let open_started = Instant::now();
            axiom_index::trace_path("document_load_request", "Other", &path);
            let disk_started = Instant::now();
            let document = match preloaded
                .map(Ok)
                .unwrap_or_else(|| Document::from_file(&path))
            {
                Ok(document) => document,
                Err(error) => {
                    self.status = format!("Falha ao abrir definition: {error}").into();
                    return;
                }
            };
            let disk_read_us = disk_started.elapsed().as_micros();
            let lsp = self.lsp.clone();
            let editor_started = Instant::now();
            let editor = cx.new(|cx| EditorView::from_document(path.clone(), document, lsp, cx));
            let editor_create_us = editor_started.elapsed().as_micros();
            if let Some(symbols) = &self._runtime_symbols {
                editor.update(cx, |editor, _| editor.set_runtime_symbols(symbols.clone()));
            }
            if let Some(index) = &self.project_index {
                editor.update(cx, |editor, _| {
                    let root = self
                        .project
                        .as_ref()
                        .map(|p| p.root_path().to_path_buf())
                        .unwrap_or_default();
                    let workspace_source = axiom_index::is_workspace_source_lexical(&path, &root);
                    editor.set_workspace_root(root, workspace_source)
                });
                editor.update(cx, |editor, editor_cx| {
                    editor.set_project_symbols(index.clone(), editor_cx)
                });
                editor.update(cx, |editor, _| {
                    editor.set_semantic_update_sender(
                        self.semantic_update_sender.clone(),
                        self.project_semantic_generation,
                    )
                });
                if let Some(vendor) = &self.vendor_index {
                    editor.update(cx, |editor, _| editor.set_vendor_symbols(vendor.clone()));
                }
            }
            if let Some(engine) = &self.semantic_engine {
                editor.update(cx, |editor, editor_cx| {
                    editor.set_semantic_engine(engine.clone(), editor_cx);
                });
            }
            cx.observe(&editor, |_, _, cx| cx.notify()).detach();
            self.tabs.push(OpenTab {
                path: path.clone(),
                editor,
            });
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
                    disk_read_us,
                    editor_create_us,
                    total_us = open_started.elapsed().as_micros(),
                    "[EDITOR OPEN PERF]"
                );
            }
            self.tabs.len() - 1
        };
        self.active = Some(index);
        self.focus_active_editor = true;
        self.tabs[index].editor.update(cx, |editor, cx| {
            editor.reveal_lsp_position(target.position, cx)
        });
        if debug_input_enabled() {
            tracing::info!(
                line = target.position.line,
                character = target.position.character,
                "[NAVIGATION CARET]"
            );
            tracing::info!(success = true, "[NAVIGATION RESULT]");
        }
    }

    fn accept_lsp_generation(
        &mut self,
        uri: &lsp_types::Uri,
        kind: LspRequestKind,
        generation: u64,
    ) -> bool {
        let last = self.lsp_generations.entry((uri.clone(), kind)).or_insert(0);
        if generation < *last {
            return false;
        }
        *last = generation;
        true
    }

    fn navigate_back(&mut self, _: &NavigateBack, _: &mut Window, cx: &mut Context<Self>) {
        let Some(location) = self.navigation_back.pop() else {
            self.status = "No earlier navigation location".into();
            cx.notify();
            return;
        };
        if let Some(active) = self.active.and_then(|index| self.tabs.get(index)) {
            if let Some(position) = active.editor.read(cx).current_lsp_position() {
                self.navigation_forward.push(NavigationLocation {
                    path: active.path.clone(),
                    position,
                });
            }
        }
        self.open_definition_without_history(location.path, location.position, cx);
    }

    fn navigate_forward(&mut self, _: &NavigateForward, _: &mut Window, cx: &mut Context<Self>) {
        let Some(location) = self.navigation_forward.pop() else {
            self.status = "No later navigation location".into();
            cx.notify();
            return;
        };
        if let Some(active) = self.active.and_then(|index| self.tabs.get(index)) {
            if let Some(position) = active.editor.read(cx).current_lsp_position() {
                self.navigation_back.push(NavigationLocation {
                    path: active.path.clone(),
                    position,
                });
            }
        }
        self.open_definition_without_history(location.path, location.position, cx);
    }

    fn open_definition_without_history(
        &mut self,
        path: PathBuf,
        position: lsp_types::Position,
        cx: &mut Context<Self>,
    ) {
        let path = match fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) => {
                self.status = format!("Navigation target unavailable: {error}").into();
                return;
            }
        };
        let index = if let Some(index) = self.tabs.iter().position(|tab| tab.path == path) {
            index
        } else {
            axiom_index::trace_path("document_load_request", "Other", &path);
            let Ok(document) = Document::from_file(&path) else {
                self.status = "Navigation target could not be opened".into();
                return;
            };
            let editor = cx
                .new(|cx| EditorView::from_document(path.clone(), document, self.lsp.clone(), cx));
            if let Some(symbols) = &self._runtime_symbols {
                editor.update(cx, |editor, _| editor.set_runtime_symbols(symbols.clone()));
            }
            if let Some(index) = &self.project_index {
                editor.update(cx, |editor, _| {
                    let root = self
                        .project
                        .as_ref()
                        .map(|p| p.root_path().to_path_buf())
                        .unwrap_or_default();
                    let workspace_source = axiom_index::is_workspace_source_lexical(&path, &root);
                    editor.set_workspace_root(root, workspace_source)
                });
                editor.update(cx, |editor, editor_cx| {
                    editor.set_project_symbols(index.clone(), editor_cx)
                });
                editor.update(cx, |editor, _| {
                    editor.set_semantic_update_sender(
                        self.semantic_update_sender.clone(),
                        self.project_semantic_generation,
                    )
                });
                if let Some(vendor) = &self.vendor_index {
                    editor.update(cx, |editor, _| editor.set_vendor_symbols(vendor.clone()));
                }
            }
            if let Some(engine) = &self.semantic_engine {
                editor.update(cx, |editor, editor_cx| {
                    editor.set_semantic_engine(engine.clone(), editor_cx);
                });
            }
            cx.observe(&editor, |_, _, cx| cx.notify()).detach();
            self.tabs.push(OpenTab { path, editor });
            self.tabs.len() - 1
        };
        self.active = Some(index);
        self.focus_active_editor = true;
        self.tabs[index]
            .editor
            .update(cx, |editor, cx| editor.reveal_lsp_position(position, cx));
        cx.notify();
    }

    fn render_activity_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let project_workspace = workspace.clone();
        let project_active = self.project_panel_visible;
        div()
            .w(m.activity_bar_width)
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .py_1()
            .gap_1()
            .bg(t.window_background)
            .border_r_1()
            .border_color(t.border_subtle)
            .child(
                div()
                    .id("activity-project")
                    .relative()
                    .w(m.activity_bar_width)
                    .h(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(|_, cx| tooltip("Project", cx))
                    .hover(move |style| style.bg(t.hover))
                    .on_click(move |_, _, cx| {
                        project_workspace.update(cx, |this, cx| {
                            let before = this.project_panel_visible;
                            this.project_panel_visible = !this.project_panel_visible;
                            if debug_input_enabled() {
                                tracing::info!(
                                    before,
                                    after = this.project_panel_visible,
                                    "[PROJECT PANEL]"
                                );
                            }
                            cx.notify();
                        });
                    })
                    .when(project_active, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(0.))
                                .h(px(20.))
                                .w(px(2.))
                                .rounded_r(m.border_radius_small)
                                .bg(t.accent),
                        )
                    })
                    .child(activity_icon(
                        ActivityIcon::Project,
                        if project_active {
                            t.accent
                        } else {
                            t.text_muted
                        },
                    )),
            )
            .child(
                div()
                    .id("activity-search-disabled")
                    .w(m.toolbar_height)
                    .h(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(|_, cx| tooltip("Search — not available yet", cx))
                    .child(activity_icon(ActivityIcon::Search, t.text_muted)),
            )
            .child(
                div()
                    .id("activity-problems-disabled")
                    .w(m.toolbar_height)
                    .h(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(|_, cx| tooltip("Problems — not available yet", cx))
                    .child(activity_icon(ActivityIcon::Problems, t.text_muted)),
            )
            .child({
                let workspace = workspace.clone();
                let active = self.terminal_visible;
                div()
                    .id("activity-terminal")
                    .relative()
                    .w(m.activity_bar_width)
                    .h(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(|_, cx| tooltip("Terminal (Ctrl+`)", cx))
                    .hover(move |style| style.bg(t.hover))
                    .on_click(move |_, window, cx| {
                        workspace.update(cx, |this, cx| {
                            this.toggle_terminal(&ToggleTerminal, window, cx);
                        });
                    })
                    .when(active, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(0.))
                                .h(px(20.))
                                .w(px(2.))
                                .rounded_r(m.border_radius_small)
                                .bg(t.accent),
                        )
                    })
                    .child(activity_icon(
                        ActivityIcon::Terminal,
                        if active { t.accent } else { t.text_secondary },
                    ))
            })
            .child(div().flex_1())
    }

    fn render_find_usages(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let title = self
            .find_usages_context
            .as_ref()
            .map_or("Find Usages", |context| context.kind.title());
        let anchor = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|tab| tab.editor.read(cx).semantic_popup_anchor(window))
            .unwrap_or(gpui::point(m.spacing_lg, m.panel_header_height));
        let geometry =
            semantic_popup_geometry(anchor, window.viewport_size(), self.find_usages.len());
        let list_height = semantic_list_height(self.find_usages.len()).min(
            (geometry.size.height - (m.ui_font_size + m.spacing_xs * 3. + px(3.))).max(px(0.)),
        );
        div()
            .id("find-usages-popup")
            .absolute()
            .top(geometry.origin.y)
            .left(geometry.origin.x)
            .w(geometry.size.width)
            .max_h(geometry.size.height)
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(t.popup_background)
            .border_1()
            .border_color(t.border)
            .rounded(m.border_radius_medium)
            .shadow_lg()
            .occlude()
            .track_focus(&self.find_usages_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        this.find_usages_visible = false;
                        this.find_usages_context = None;
                        if let Some(tab) = this.active.and_then(|i| this.tabs.get(i)) {
                            window.focus(&tab.editor.read(cx).focus_handle(cx));
                        }
                    }
                    "enter" => {
                        if let Some(target) =
                            this.find_usages.get(this.find_usages_selected).cloned()
                        {
                            this.open_find_usage(target, cx);
                        }
                    }
                    "up" | "down" if !this.find_usages.is_empty() => {
                        let len = this.find_usages.len();
                        this.find_usages_selected = if event.keystroke.key == "up" {
                            this.find_usages_selected.saturating_sub(1)
                        } else {
                            (this.find_usages_selected + 1).min(len - 1)
                        };
                        this.find_usages_scroll
                            .scroll_to_item(this.find_usages_selected, gpui::ScrollStrategy::Top);
                    }
                    "home" if !this.find_usages.is_empty() => this.find_usages_selected = 0,
                    "end" if !this.find_usages.is_empty() => {
                        this.find_usages_selected = this.find_usages.len() - 1
                    }
                    _ => return,
                }
                cx.stop_propagation();
                window.prevent_default();
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("find-usages-header")
                    .flex_none()
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py(m.spacing_xs)
                    .text_size(m.ui_font_size)
                    .line_height(m.ui_font_size + m.spacing_xs)
                    .child(format!(
                        "{} · {} result{}",
                        title,
                        self.find_usages.len(),
                        if self.find_usages.len() == 1 { "" } else { "s" }
                    ))
                    .child(
                        div()
                            .id("find-usages-close")
                            .px_2()
                            .cursor(CursorStyle::PointingHand)
                            .hover(|style| style.bg(t.hover))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_find_usages(&CloseFindUsages, window, cx);
                            }))
                            .child("×"),
                    ),
            )
            .when(self.find_usages.is_empty(), |this| {
                this.child(div().px_3().py_2().child("No usages found"))
            })
            .when(!self.find_usages.is_empty(), |this| {
                this.child(
                    gpui::uniform_list(
                        "find-usages-rows",
                        self.find_usages.len(),
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|index| {
                                    let target = this.find_usages[index].clone();
                                    let filename = target
                                        .file
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("document")
                                        .to_owned();
                                    let position = target.position;
                                    let snippet = target.snippet.clone();
                                    let click_target = target.clone();
                                    let selected = index == this.find_usages_selected;
                                    let (background, border) = semantic_row_colors(selected, false);
                                    div()
                                        .id(("find-usage", index))
                                        .w_full()
                                        .relative()
                                        .h(semantic_row_height())
                                        .py(m.spacing_xs / 2.)
                                        .px_3()
                                        .flex()
                                        .flex_col()
                                        .text_size(m.ui_font_size)
                                        .line_height(m.ui_font_size + m.spacing_xs)
                                        .overflow_hidden()
                                        .cursor(CursorStyle::PointingHand)
                                        .text_color(t.text_primary)
                                        .bg(background)
                                        .border_l_2()
                                        .border_color(border)
                                        .hover(move |style| {
                                            style
                                                .border_color(semantic_row_colors(selected, true).1)
                                        })
                                        .tooltip({
                                            let full_path = target.file.display().to_string();
                                            move |_, cx| tooltip(full_path.clone(), cx)
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.open_find_usage(click_target.clone(), cx);
                                        }))
                                        .child(
                                            div()
                                                .w_full()
                                                .flex()
                                                .justify_between()
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .overflow_hidden()
                                                        .child(filename),
                                                )
                                                .child(
                                                    div()
                                                        .flex_none()
                                                        .text_size(px(11.))
                                                        .text_color(t.text_muted)
                                                        .child(format!(
                                                            "{}:{}",
                                                            position.line + 1,
                                                            position.character + 1
                                                        )),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_color(t.text_secondary)
                                                .text_size(px(11.))
                                                .overflow_hidden()
                                                .child(snippet),
                                        )
                                        .when(index + 1 < this.find_usages.len(), |row| {
                                            row.child(
                                                div()
                                                    .absolute()
                                                    .bottom(px(0.))
                                                    .left(m.spacing_md)
                                                    .right(m.spacing_md)
                                                    .h(px(1.))
                                                    .bg(t.border_subtle),
                                            )
                                        })
                                })
                                .collect()
                        }),
                    )
                    .h(list_height)
                    .track_scroll(self.find_usages_scroll.clone()),
                )
            })
    }

    fn project_panel_resize_start(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_panel_resizing = true;
        self.project_panel_resize_start_x = event.position.x.into();
        self.project_panel_resize_start_width = self.project_panel_width.into();
        cx.notify();
    }

    fn project_panel_resize_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.project_panel_resizing {
            return;
        }
        let x: f32 = event.position.x.into();
        let width = (self.project_panel_resize_start_width + x - self.project_panel_resize_start_x)
            .clamp(180.0, 520.0);
        self.project_panel_width = px(width);
        cx.notify();
    }

    fn project_panel_resize_end(
        &mut self,
        _: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_panel_resizing = false;
        cx.notify();
    }

    fn render_explorer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let project_root = self
            .project
            .as_ref()
            .map(|project| project.root_path().to_path_buf());
        let root_name = self
            .project
            .as_ref()
            .map(Project::name)
            .unwrap_or("No project");
        let active_path = self
            .active
            .and_then(|index| self.tabs.get(index))
            .map(|tab| tab.path.clone());
        // Keep the tree viewport constrained by the sidebar while giving its
        // content a natural width. This enables GPUI's horizontal scrollbar
        // for deep trees and long names instead of shrinking/clipping rows.
        let tree_content_width = self
            .explorer
            .iter()
            .map(|item| {
                12.0 + item.depth as f32 * 16.0 + 28.0 + item.name.chars().count() as f32 * 8.0
            })
            .fold(180.0_f32, f32::max);
        let scroll_max: f32 = self.explorer_scroll.max_offset().height.into();
        let viewport_height: f32 = self.explorer_scroll.bounds().size.height.into();
        let content_height = (viewport_height + scroll_max).max(1.0);
        let thumb_height = (viewport_height * viewport_height / content_height)
            .max(24.0)
            .min(viewport_height.max(24.0));
        let offset_y: f32 = self.explorer_scroll.offset().y.into();
        let thumb_top = if scroll_max > 0.0 {
            (offset_y / scroll_max) * (viewport_height - thumb_height).max(0.0)
        } else {
            0.0
        };
        div()
            .id("explorer-sidebar")
            .w(self.project_panel_width)
            .min_w(px(180.))
            .h_full()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .relative()
            .on_hover({
                let workspace = workspace.clone();
                move |hovered, _, cx| {
                    workspace.update(cx, |this, cx| {
                        this.explorer_scroll_hovered = *hovered;
                        cx.notify();
                    });
                }
            })
            .bg(t.sidebar_background)
            .border_r_1()
            .border_color(t.border_subtle)
            .child(
                div()
                    .h(m.panel_header_height)
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(m.ui_font_size)
                    .text_color(t.text_secondary)
                    .child("PROJECT")
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .when_some(project_root.clone(), |this, root| {
                                let new_file_workspace = workspace.clone();
                                let new_directory_workspace = workspace.clone();
                                let refresh_workspace = workspace.clone();
                                this.child(
                                    div()
                                        .id("explorer-new-file")
                                        .tooltip(|_, cx| tooltip("New", cx))
                                        .px_1()
                                        .w(m.toolbar_height)
                                        .h(m.toolbar_height)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(m.border_radius_small)
                                        .hover(move |s| s.bg(t.hover))
                                        .on_mouse_down(MouseButton::Left, |_, _, _| {
                                            if debug_input_enabled() {
                                                tracing::info!(
                                                    "[EXPLORER TOOLBAR MOUSE DOWN] button=new"
                                                );
                                            }
                                        })
                                        .on_click({
                                            let root = root.clone();
                                            move |_, window, cx| {
                                                new_file_workspace.update(cx, |this, cx| {
                                                    this.open_new_menu(root.clone(), cx);
                                                    let _ = window;
                                                })
                                            }
                                        })
                                        .child("+"),
                                )
                                .child(
                                    div()
                                        .id("explorer-new-directory")
                                        .tooltip(|_, cx| tooltip("New Directory", cx))
                                        .px_1()
                                        .w(m.toolbar_height)
                                        .h(m.toolbar_height)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(m.border_radius_small)
                                        .hover(move |s| s.bg(t.hover))
                                        .on_click({
                                            let root = root.clone();
                                            move |_, _, cx| {
                                                new_directory_workspace.update(cx, |this, cx| {
                                                    this.new_directory(root.clone(), cx)
                                                })
                                            }
                                        })
                                        .child("□+"),
                                )
                                .child(
                                    div()
                                        .id("explorer-refresh")
                                        .tooltip(|_, cx| tooltip("Refresh", cx))
                                        .px_1()
                                        .w(m.toolbar_height)
                                        .h(m.toolbar_height)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(m.border_radius_small)
                                        .hover(move |s| s.bg(t.hover))
                                        .on_click(move |_, _, cx| {
                                            refresh_workspace
                                                .update(cx, |this, cx| this.refresh_explorer(cx));
                                        })
                                        .child("↻"),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .h(m.toolbar_height)
                    .px_2()
                    .text_color(t.text_primary)
                    .when_some(project_root.clone(), |this, root| {
                        let workspace = workspace.clone();
                        this.on_mouse_down(MouseButton::Right, move |event, _, cx| {
                            workspace.update(cx, |this, cx| {
                                this.open_context_menu(
                                    root.clone(),
                                    EntryKind::Directory,
                                    event.position,
                                    cx,
                                );
                            });
                            cx.stop_propagation();
                        })
                    })
                    .child(format!("▾ {root_name}")),
            )
            .child(
                div()
                    .id("explorer-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .min_w(px(0.))
                    .overflow_scroll()
                    .scrollbar_width(px(8.))
                    .track_scroll(&self.explorer_scroll)
                    .on_mouse_move(cx.listener(Self::explorer_scroll_drag_move))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(Self::explorer_scroll_drag_end),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(Self::explorer_scroll_drag_end),
                    )
                    .child(div().w(px(tree_content_width)).flex().flex_col().children(
                        self.explorer.iter().enumerate().map(|(index, item)| {
                            let item = item.clone();
                            let workspace = workspace.clone();
                            let context_workspace = workspace.clone();
                            let context_item = item.clone();
                            let is_expanded = self.expanded.contains(&item.path);
                            let is_active = active_path.as_ref() == Some(&item.path);
                            let icon = file_icon(
                                &item.path,
                                item.kind == EntryKind::Directory,
                                is_expanded,
                            )
                            .glyph();
                            div()
                                .id(("explorer", index))
                                .w_full()
                                .h(px(24.))
                                .pl(px(12. + item.depth as f32 * 16.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(m.ui_font_size)
                                .text_color(if is_active {
                                    t.text_primary
                                } else {
                                    t.text_secondary
                                })
                                .bg(if is_active {
                                    t.pressed
                                } else {
                                    t.sidebar_background
                                })
                                .hover(move |style| style.bg(t.hover))
                                .on_click(move |_, window, cx| {
                                    workspace.update(cx, |this, cx| {
                                        window.focus(&this.focus);
                                        if item.kind == EntryKind::Directory {
                                            this.selected_path = Some(item.path.clone());
                                            this.toggle_directory(index, cx);
                                        } else {
                                            this.selected_path = Some(item.path.clone());
                                            this.open_file(item.path.clone(), window, cx);
                                        }
                                    });
                                })
                                .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                                    context_workspace.update(cx, |this, cx| {
                                        window.focus(&this.focus);
                                        this.open_context_menu(
                                            context_item.path.clone(),
                                            context_item.kind,
                                            event.position,
                                            cx,
                                        );
                                    });
                                    cx.stop_propagation();
                                })
                                .child(
                                    div()
                                        .w(m.icon_size)
                                        .text_color(if is_active { t.accent } else { t.text_muted })
                                        .child(icon),
                                )
                                .child(item.name)
                        }),
                    ))
                    .when(scroll_max > 0.0 && self.explorer_scroll_hovered, |this| {
                        this.child(
                            div()
                                .absolute()
                                .right(px(1.))
                                .top(px(1.))
                                .bottom(px(1.))
                                .w(px(6.))
                                .opacity(0.0)
                                .hover(|style| style.opacity(1.0))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(Self::explorer_scroll_drag_start),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(0.))
                                        .right(px(0.))
                                        .top(px(thumb_top))
                                        .h(px(thumb_height))
                                        .rounded(px(3.))
                                        .bg(t.scrollbar_hover),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .id("explorer-resize-divider")
                    .absolute()
                    .right(px(-2.))
                    .top(px(0.))
                    .bottom(px(0.))
                    .w(px(4.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .hover(|style| style.bg(theme().accent))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(Self::project_panel_resize_start),
                    ),
            )
    }

    fn explorer_scroll_drag_start(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.explorer_scroll_dragging = true;
        self.explorer_scroll_drag_start_y = event.position.y.into();
        let offset: f32 = self.explorer_scroll.offset().y.into();
        self.explorer_scroll_drag_start_offset = -offset;
        cx.notify();
    }

    fn explorer_scroll_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if !self.explorer_scroll_dragging {
            return;
        }
        let viewport: f32 = self.explorer_scroll.bounds().size.height.into();
        let max: f32 = self.explorer_scroll.max_offset().height.into();
        let thumb = (viewport * viewport / (viewport + max).max(1.0)).max(24.0);
        let track = (viewport - thumb).max(1.0);
        let delta: f32 = event.position.y.into();
        let delta = delta - self.explorer_scroll_drag_start_y;
        let position =
            (self.explorer_scroll_drag_start_offset + delta * max / track).clamp(0.0, max);
        self.explorer_scroll
            .set_offset(Point::new(px(0.), px(-position)));
    }

    fn explorer_scroll_drag_end(
        &mut self,
        _: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.explorer_scroll_dragging = false;
        cx.notify();
    }

    fn render_explorer_context(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let Some(context) = self.explorer_context.clone() else {
            return div();
        };
        let Some(project) = self.project.as_ref() else {
            return div();
        };
        let root = project.root_path().to_path_buf();
        let directory = Self::context_directory(&context.path, context.kind).to_path_buf();
        let is_root = context.path == root;
        let menu_width = px(300.);
        let submenu_width = px(250.);
        let opens_left = self.context_menu_position.x + menu_width + submenu_width
            > window.viewport_size().width;
        let menu_left = self
            .context_menu_position
            .x
            .max(px(0.))
            .min((window.viewport_size().width - menu_width).max(px(0.)));
        let menu_top = self
            .context_menu_position
            .y
            .max(px(0.))
            .min((window.viewport_size().height - px(360.)).max(px(0.)));
        div()
            .absolute()
            .left(menu_left)
            .top(menu_top)
            .w(menu_width)
            .p_1()
            .rounded(m.border_radius_medium)
            .border_1()
            .border_color(t.border)
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .bg(t.menu_background)
            .text_color(t.text_primary)
            .when(context.kind == EntryKind::File, |this| {
                let open_workspace = workspace.clone();
                let path = context.path.clone();
                this.child(Self::explorer_menu_item("Open", move |window, cx| {
                    open_workspace.update(cx, |this, cx| this.open_file(path.clone(), window, cx));
                }))
                .child({
                    let workspace = workspace.clone();
                    Self::explorer_menu_item(
                        "Open Containing Folder in Terminal",
                        move |window, cx| {
                            workspace.update(cx, |this, cx| {
                                this.open_in_terminal(&OpenInTerminal, window, cx)
                            });
                        },
                    )
                })
            })
            .when(context.kind == EntryKind::Directory, |this| {
                let workspace = workspace.clone();
                this.child(Self::explorer_menu_item(
                    "Open in Terminal",
                    move |window, cx| {
                        workspace.update(cx, |this, cx| {
                            this.open_in_terminal(&OpenInTerminal, window, cx)
                        });
                    },
                ))
            })
            .child({
                let workspace = workspace.clone();
                let click_workspace = workspace.clone();
                let directory = directory.clone();
                let submenu = self.render_new_submenu(directory.clone(), opens_left, cx);
                div()
                    .relative()
                    .id("explorer-new-submenu-trigger")
                    .h(metrics().toolbar_height)
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(metrics().border_radius_small)
                    .hover(move |style| style.bg(theme().hover))
                    .on_mouse_move(move |_, _, cx| {
                        workspace.update(cx, |this, cx| {
                            this.explorer_new_menu_open = true;
                            if debug_input_enabled() {
                                tracing::info!("[SUBMENU] open=New");
                            }
                            cx.notify();
                        });
                    })
                    .on_click({
                        move |_, _, cx| {
                            click_workspace.update(cx, |this, cx| this.open_context_submenu(cx));
                        }
                    })
                    .child("New")
                    .child("▶")
                    .when(self.explorer_new_menu_open, |this| this.child(submenu))
            })
            .when(!is_root, |this| {
                let rename_workspace = workspace.clone();
                let delete_workspace = workspace.clone();
                let rename_path = context.path.clone();
                let delete_path = context.path.clone();
                this.child(Self::explorer_menu_item("Rename  F2", move |window, cx| {
                    rename_workspace.update(cx, |this, cx| {
                        window.focus(&this.focus);
                        this.selected_path = Some(rename_path.clone());
                        this.execute_command("project.rename", window, cx);
                    });
                }))
                .child(Self::explorer_menu_item("Delete", move |_, cx| {
                    delete_workspace
                        .update(cx, |this, cx| this.request_delete(delete_path.clone(), cx));
                }))
            })
            .child({
                let workspace = workspace.clone();
                let path = context.path.clone();
                Self::explorer_menu_item("Copy Path", move |_, cx| {
                    workspace.update(cx, |this, cx| this.copy_path(&path, cx));
                })
            })
            .child({
                let workspace = workspace.clone();
                Self::explorer_menu_item("Refresh", move |_, cx| {
                    workspace.update(cx, |this, cx| this.refresh_explorer(cx));
                })
            })
    }

    fn render_new_submenu(
        &self,
        directory: PathBuf,
        opens_left: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspace = cx.entity();
        let t = theme();
        let m = metrics();
        div()
            .absolute()
            .left(if opens_left { px(-296.) } else { px(296.) })
            .top(px(0.))
            .w(px(250.))
            .p_1()
            .flex()
            .flex_col()
            .gap_1()
            .bg(t.menu_background)
            .border_1()
            .border_color(t.border)
            .rounded(m.border_radius_medium)
            .shadow_lg()
            .on_mouse_move(|_, _, _| {})
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("File", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::File, directory.clone(), window, cx)
                    });
                })
            })
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("Directory", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::Directory, directory.clone(), window, cx)
                    });
                })
            })
            .child(div().h(px(1.)).mx_2().bg(t.border_subtle))
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("PHP File", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::PhpFile, directory.clone(), window, cx)
                    });
                })
            })
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("PHP Class", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::PhpClass, directory.clone(), window, cx)
                    });
                })
            })
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("PHP Interface", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(
                            NewItemKind::PhpInterface,
                            directory.clone(),
                            window,
                            cx,
                        )
                    });
                })
            })
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("PHP Trait", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::PhpTrait, directory.clone(), window, cx)
                    });
                })
            })
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("PHP Enum", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::PhpEnum, directory.clone(), window, cx)
                    });
                })
            })
    }

    fn modal_text_field(
        &self,
        workspace: Entity<WorkspaceView>,
        label: &'static str,
        field: ModalField,
        value: String,
        window: &Window,
    ) -> impl IntoElement {
        let t = theme();
        let selected = self.modal_field_selection(field).range.clone();
        let focused = self.explorer_modal_field == field
            && self.modal_field_focus[field as usize].is_focused(window);
        let caret = if self.modal_field_selection(field).reversed {
            selected.start
        } else {
            selected.end
        }
        .min(value.encode_utf16().count());
        div()
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(t.text_secondary)
                    .child(label),
            )
            .child(
                div()
                    .id(if field == ModalField::Name {
                        SharedString::from("explorer-operation-input")
                    } else {
                        SharedString::from(format!("php-input-{field:?}"))
                    })
                    .track_focus(&self.modal_field_focus[field as usize])
                    .debug_selector(move || format!("modal-field-{field:?}"))
                    .h(px(34.))
                    .w_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .bg(t.panel_background)
                    .border_1()
                    .border_color(if focused { t.accent } else { t.border_subtle })
                    .cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, {
                        let workspace = workspace.clone();
                        move |event, window, cx| {
                            cx.stop_propagation();
                            workspace.update(cx, |this, cx| {
                                this.modal_pointer_down(field, event, window, cx);
                            });
                        }
                    })
                    .on_mouse_move({
                        let workspace = workspace.clone();
                        move |event, window, cx| {
                            workspace.update(cx, |this, cx| {
                                this.modal_pointer_move(field, event, window, cx)
                            });
                        }
                    })
                    .overflow_hidden()
                    .child(crate::ui::input_line::render(
                        self.modal_inputs[field as usize].clone(),
                        self.modal_field_focus[field as usize].clone(),
                        value,
                        if focused {
                            utf16_to_byte_offset(self.modal_field_text(field), selected.start)
                                ..utf16_to_byte_offset(self.modal_field_text(field), selected.end)
                        } else {
                            0..0
                        },
                        utf16_to_byte_offset(self.modal_field_text(field), caret),
                        self.modal_caret_for(field, window),
                        self.modal_field_geometry[field as usize].clone(),
                    )),
            )
            .when(focused && !self.modal_type_items.is_empty(), |this| {
                let anchor = self.modal_field_geometry[field as usize]
                    .anchor()
                    .unwrap_or_default();
                let geometry = modal_type_popup_geometry(
                    anchor,
                    window.viewport_size(),
                    self.modal_type_items.len(),
                    Some(px(630.)),
                );
                this.child(
                    gpui::deferred(
                        gpui::anchored()
                            .position_mode(gpui::AnchoredPositionMode::Window)
                            .position(geometry.origin)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.modal_type_popup(workspace.clone(), geometry.size)),
                    )
                    .with_priority(2),
                )
            })
            .when(
                field == ModalField::Name
                    && matches!(
                        self.explorer_operation,
                        Some(ExplorerOperation::NewPhp { .. })
                    )
                    && !valid_php_identifier(self.explorer_input.trim_end_matches(".php")),
                |this| {
                    this.child(
                        div()
                            .text_size(px(10.))
                            .text_color(t.error)
                            .child("Invalid PHP identifier"),
                    )
                },
            )
    }

    fn modal_type_popup(
        &self,
        workspace: Entity<Self>,
        size: gpui::Size<Pixels>,
    ) -> impl IntoElement {
        let t = theme();
        div()
            .id("modal-type-completion")
            .debug_selector(|| "modal-type-completion".into())
            .w(size.width)
            .h(size.height)
            .overflow_y_scroll()
            .track_scroll(&self.modal_type_scroll)
            .bg(t.popup_background)
            .border_1()
            .border_color(t.border)
            .shadow_lg()
            .occlude()
            .children(
                self.modal_type_items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let workspace = workspace.clone();
                        div()
                            .id(("modal-type-candidate", index))
                            .debug_selector(move || format!("modal-type-candidate-{index}"))
                            .h(px(42.))
                            .px_2()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .bg(if index == self.modal_type_selected {
                                t.hover
                            } else {
                                t.popup_background
                            })
                            .hover(move |style| style.bg(t.hover))
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                cx.stop_propagation();
                                workspace.update(cx, |this, cx| {
                                    this.accept_modal_type(index, cx);
                                    window.focus(
                                        &this.modal_field_focus[this.explorer_modal_field as usize],
                                    );
                                });
                            })
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.text_primary)
                                    .overflow_hidden()
                                    .child(item.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(t.text_secondary)
                                    .overflow_hidden()
                                    .child(item.detail.clone().unwrap_or_default()),
                            )
                    }),
            )
    }

    fn render_explorer_operation(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let title = match self.explorer_operation {
            Some(ExplorerOperation::Rename(_)) => "Rename",
            Some(ExplorerOperation::NewDirectory(_)) => "New Directory",
            Some(ExplorerOperation::NewFile(_)) => "File",
            Some(ExplorerOperation::NewPhpFile(_)) => "PHP File",
            Some(ExplorerOperation::NewPhp { keyword, .. }) => match keyword {
                "class" => "Create PHP Class",
                "interface" => "Create PHP Interface",
                "trait" => "Create PHP Trait",
                "enum" => "Create PHP Enum",
                _ => "Create PHP Type",
            },
            None => "",
        };
        let php_valid = self
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| match operation {
                ExplorerOperation::NewPhp { .. } => {
                    valid_php_identifier(self.explorer_input.trim_end_matches(".php"))
                        && valid_php_namespace(&self.explorer_namespace)
                }
                ExplorerOperation::NewPhpFile(_) => !self.explorer_input.trim().is_empty(),
                _ => true,
            });
        div()
            .absolute()
            .top(px(110.))
            .debug_selector(|| "php-type-modal".into())
            .left(px(320.))
            .w(px(520.))
            .max_h(px(680.))
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .bg(t.popup_background)
            .border_1()
            .border_color(t.border)
            .rounded(m.border_radius_medium)
            .cursor(CursorStyle::Arrow)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .shadow_lg()
            .child(
                div()
                    .text_size(px(15.))
                    .text_color(t.text_primary)
                    .child(title),
            )
            .when(
                self.explorer_operation
                    .as_ref()
                    .is_some_and(|operation| matches!(operation, ExplorerOperation::NewPhp { .. })),
                |this| {
                    let directory = self.operation_directory();
                    let root = self.project.as_ref().map(|p| p.root_path());
                    this.child(
                        div()
                            .text_color(t.text_muted)
                            .text_size(px(11.))
                            .child("TYPE"),
                    )
                    .child(
                        div()
                            .id("php-type-segmented")
                            .w_full()
                            .h(px(34.))
                            .flex()
                            .items_center()
                            .rounded(m.border_radius_small)
                            .bg(t.panel_background)
                            .children([("class", "Class"), ("interface", "Interface"), ("trait", "Trait"), ("enum", "Enum")].into_iter().map(|(keyword, label)| {
                                let active = matches!(self.explorer_operation, Some(ExplorerOperation::NewPhp { keyword: current, .. }) if current == keyword);
                                let workspace = workspace.clone();
                                div().id(SharedString::from(format!("php-type-{keyword}"))).flex_1().h_full().flex().items_center().justify_center().px_2().rounded(m.border_radius_small)
                                    .bg(if active { t.inactive_selection } else { t.panel_background })
                                    .border_1().border_color(if active { t.accent } else { t.panel_background })
                                    .text_color(if active { t.text_primary } else { t.text_secondary })
                                    .cursor(CursorStyle::PointingHand).hover(move |style| if active { style } else { style.bg(t.hover) })
                                    .on_click(move |_, window, cx| workspace.update(cx, |this, cx| { this.select_php_type(keyword, cx); window.focus(&this.modal_field_focus[this.explorer_modal_field as usize]); }))
                                    .child(label)
                            }))
                    )
                    .child(
                        div()
                            .text_color(t.text_muted)
                            .text_size(px(11.))
                            .child("DIRECTORY"),
                    )
                    .child(
                        div()
                            .text_color(t.text_secondary)
                            .child(relative_directory_label(root.as_deref(), &directory)),
                    )
                },
            )
            .child(self.modal_text_field(workspace.clone(), "Name", ModalField::Name, self.explorer_input.clone(), _window))            .when(self.explorer_operation.as_ref().is_some_and(|op| matches!(op, ExplorerOperation::NewPhp { .. })), |this| {
                let workspace = workspace.clone();
                this.child(self.modal_text_field(workspace.clone(), "Namespace", ModalField::Namespace, self.explorer_namespace.clone(), _window))
                    .child(self.modal_text_field(workspace.clone(), "File", ModalField::File, self.explorer_file.clone(), _window))
                    .when(self.explorer_operation.as_ref().is_some_and(|op| matches!(op, ExplorerOperation::NewPhp { keyword: "class" | "interface", .. })), |form| {
                        form.child(self.modal_text_field(workspace.clone(), "Extends", ModalField::Extends, self.explorer_extends.clone(), _window))
                    })
                    .when(self.explorer_operation.as_ref().is_some_and(|op| matches!(op, ExplorerOperation::NewPhp { keyword: "class", .. })), |form| {
                        form.child(self.modal_text_field(workspace.clone(), "Implements", ModalField::Implements, self.explorer_implements.clone(), _window))
                    })
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .border_t_1()
                    .border_color(t.border_subtle)
                    .pt_3()
                    .mt_2()
                    .child(
                        div()
                            .id("explorer-operation-cancel")
                            .px_2()
                            .py_1()
                            .cursor(CursorStyle::PointingHand)
                            .on_click({
                                let workspace = workspace.clone();
                                move |_, _, cx| {
                                    workspace
                                        .update(cx, |this, cx| this.cancel_explorer_operation(cx));
                                }
                            })
                            .child("Cancel"),
                    )
                    .child(
                        div()
                            .id("explorer-operation-confirm")
                            .px_2()
                            .py_1()
                            .bg(if php_valid { t.accent } else { t.border_subtle })
                            .cursor(if php_valid {
                                CursorStyle::PointingHand
                            } else {
                                CursorStyle::Arrow
                            })
                            .text_color(if php_valid {
                                t.window_background
                            } else {
                                t.text_muted
                            })
                            .when(php_valid, |this| {
                                this.on_click(move |_, _, cx| {
                                    workspace
                                        .update(cx, |this, cx| this.confirm_explorer_operation(cx));
                                })
                            })
                            .child("Create"),
                    ),
            )
    }

    fn operation_directory(&self) -> PathBuf {
        match self.explorer_operation.as_ref() {
            Some(ExplorerOperation::NewFile(directory))
            | Some(ExplorerOperation::NewPhpFile(directory))
            | Some(ExplorerOperation::NewPhp { directory, .. })
            | Some(ExplorerOperation::NewDirectory(directory)) => directory.clone(),
            Some(ExplorerOperation::Rename(path)) => path.parent().unwrap_or(path).to_path_buf(),
            None => PathBuf::new(),
        }
    }

    fn explorer_menu_item(
        label: &'static str,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        div()
            .id(label)
            .h(m.toolbar_height)
            .px_2()
            .flex()
            .items_center()
            .rounded(m.border_radius_small)
            .hover(move |style| style.bg(t.hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                if debug_input_enabled() {
                    tracing::info!("[SUBMENU ITEM MOUSE DOWN]");
                }
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                if debug_input_enabled() {
                    tracing::info!(item = %label, "[CONTEXT MENU ACTION]");
                    tracing::info!(item = %label, "[SUBMENU ITEM CLICK]");
                }
                handler(window, cx);
                cx.stop_propagation();
            })
            .child(label)
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let project_root = self
            .project
            .as_ref()
            .map(|project| project.root_path().to_path_buf());
        div()
            .h(m.tab_height)
            .flex()
            .bg(t.panel_background)
            .border_b_1()
            .border_color(t.border_subtle)
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                let editor = tab.editor.read(cx);
                let title = editor.title();
                let dirty = editor.is_dirty();
                let icon = file_icon(&tab.path, false, false).glyph();
                let tab_tooltip =
                    tab_display_path(&tab.path, project_root.as_deref(), &self.runtime_stub_path);
                let activate_workspace = workspace.clone();
                let close_workspace = workspace.clone();
                div()
                    .id(("tab", index))
                    .h_full()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(if self.active == Some(index) {
                        t.editor_background
                    } else {
                        t.panel_background
                    })
                    .border_b_2()
                    .border_color(if self.active == Some(index) {
                        t.accent
                    } else {
                        t.panel_background
                    })
                    .text_color(if self.active == Some(index) {
                        t.text_primary
                    } else {
                        t.text_secondary
                    })
                    .hover(move |style| style.bg(t.hover))
                    .tooltip(move |_, cx| tooltip(tab_tooltip.clone(), cx))
                    .on_click(move |_, window, cx| {
                        activate_workspace.update(cx, |this, cx| this.activate(index, window, cx));
                    })
                    .child(
                        div()
                            .text_color(t.accent)
                            .text_size(m.ui_font_size)
                            .child(icon),
                    )
                    .child(title)
                    .when(dirty, |this| {
                        this.child(div().text_color(t.warning).child("●"))
                    })
                    .child(
                        div()
                            .id(("close-tab", index))
                            .px_1()
                            .rounded(m.border_radius_small)
                            .text_color(t.text_muted)
                            .hover(move |style| style.bg(t.pressed).text_color(t.text_primary))
                            .on_click(move |_, _, cx| {
                                close_workspace.update(cx, |this, cx| this.close_tab(index, cx));
                            })
                            .child("×"),
                    )
            }))
    }

    fn action_item(label: &'static str, action: impl Action) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        div()
            .id(label)
            .px_3()
            .h(m.toolbar_height)
            .flex()
            .items_center()
            .rounded(m.border_radius_small)
            .text_color(t.text_primary)
            .hover(move |style| style.bg(t.hover))
            .on_click(move |_, window, cx| {
                if debug_input_enabled() {
                    tracing::info!(target = %label, "[ACTION] menu action dispatch");
                }
                window.dispatch_action(action.boxed_clone(), cx)
            })
            .child(label)
    }

    fn command_item<A: Action + Clone + 'static>(
        &self,
        id: &'static str,
        label: &'static str,
        action: A,
    ) -> impl IntoElement {
        let shortcut = self
            .keymap
            .shortcut(id)
            .map(Self::format_shortcut)
            .unwrap_or_default();
        let t = theme();
        let m = metrics();
        div()
            .id(label)
            .px_3()
            .h(m.toolbar_height)
            .flex()
            .items_center()
            .rounded(m.border_radius_small)
            .text_color(t.text_primary)
            .hover(move |style| style.bg(t.hover))
            .on_click(move |_, window, cx| {
                if debug_input_enabled() {
                    tracing::info!(target = %id, "[ACTION] menu command dispatch");
                }
                window.dispatch_action(action.boxed_clone(), cx)
            })
            .child(label)
            .when(!shortcut.is_empty(), |this| {
                this.child(
                    div()
                        .ml_auto()
                        .text_color(t.text_muted)
                        .child(shortcut.clone()),
                )
            })
    }

    fn command_dispatch_item(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let shortcut = self
            .keymap
            .shortcut(id)
            .map(Self::format_shortcut)
            .unwrap_or_default();
        let workspace = cx.entity();
        let t = theme();
        let m = metrics();
        div()
            .id(SharedString::from(format!("menu-command-{id}")))
            .px_3()
            .h(m.toolbar_height)
            .flex()
            .items_center()
            .rounded(m.border_radius_small)
            .text_color(t.text_primary)
            .hover(move |style| style.bg(t.hover))
            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                if debug_input_enabled() {
                    tracing::info!(label = %label, command = %id, "[MENU ITEM MOUSE DOWN]");
                }
            })
            .on_click(move |_, window, cx| {
                if debug_input_enabled() {
                    tracing::info!(label = %label, command = %id, "[MENU ITEM CLICK]");
                }
                workspace.update(cx, |this, cx| {
                    this.open_menu = None;
                    this.execute_command(id, window, cx);
                    if debug_input_enabled() {
                        tracing::info!(id = %id, executed = true, "[COMMAND RESULT]");
                    }
                });
            })
            .child(label)
            .when(!shortcut.is_empty(), |this| {
                this.child(
                    div()
                        .ml_auto()
                        .text_color(t.text_muted)
                        .child(shortcut.clone()),
                )
            })
    }

    fn format_shortcut(value: &str) -> String {
        value
            .split('-')
            .map(|part| match part {
                "ctrl" => "Ctrl".to_owned(),
                "shift" => "Shift".to_owned(),
                "alt" => "Alt".to_owned(),
                "space" => "Space".to_owned(),
                "`" => "`".to_owned(),
                other => other.to_ascii_uppercase(),
            })
            .collect::<Vec<_>>()
            .join("+")
    }

    fn render_menu_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let menu = self.open_menu;
        let labels = [
            ("File", MenuKind::File),
            ("Edit", MenuKind::Edit),
            ("Code", MenuKind::Code),
            ("View", MenuKind::View),
            ("Navigate", MenuKind::Navigate),
            ("Help", MenuKind::Help),
        ];
        if debug_input_enabled()
            && let Some(active) = menu
        {
            tracing::info!(
                active = ?active,
                dismiss = true,
                dropdown = true,
                anchor_x = ?self.menu_anchor_x,
                z_order = "dropdown-above-dismiss",
                "[MENU RENDER]"
            );
        }
        let dropdown = div()
            .absolute()
            .top(m.menu_height)
            .left(
                self.menu_anchor_x
                    .min((window.viewport_size().width - px(230.)).max(px(0.))),
            )
            .w(px(230.))
            .py_1()
            .flex()
            .flex_col()
            .bg(t.menu_background)
            .border_1()
            .border_color(t.border)
            .rounded_b(m.border_radius_medium)
            .shadow_lg()
            .occlude()
            .when(menu == Some(MenuKind::File), |this| {
                this.child(self.command_dispatch_item("project.open_project", "Open Project…", cx))
                    .child(self.command_dispatch_item("project.open_file", "Open File…", cx))
                    .child(Self::action_item(
                        "Import Runtime Stubs…",
                        ImportRuntimeStubs,
                    ))
                    .child(Self::action_item(
                        "Import Stub Files",
                        ImportRuntimeStubFiles,
                    ))
                    .child(self.command_item("editor.save", "Save", crate::editor_view::Save))
                    .child(Self::action_item("Save All", SaveAll))
                    .child(Self::action_item("Close File", CloseFile))
                    .child(Self::action_item("Close Project", CloseProject))
                    .child(Self::action_item("Settings", Settings))
                    .child(Self::action_item("Exit", Exit))
            })
            .when(menu == Some(MenuKind::Edit), |this| {
                this.child(self.command_item("editor.undo", "Undo", crate::editor_view::Undo))
                    .child(self.command_item("editor.redo", "Redo", crate::editor_view::Redo))
                    .child(self.command_item("editor.cut", "Cut", crate::editor_view::Cut))
                    .child(self.command_item("editor.copy", "Copy", crate::editor_view::Copy))
                    .child(self.command_item("editor.paste", "Paste", crate::editor_view::Paste))
                    .child(self.command_item(
                        "editor.select_all",
                        "Select All",
                        crate::editor_view::SelectAll,
                    ))
                    .child(self.command_item("editor.find", "Find", Find))
            })
            .when(menu == Some(MenuKind::View), |this| {
                this.child(Self::action_item("Project Tool Window", ToggleProject))
                    .child(self.command_item("terminal.toggle", "Terminal", ToggleTerminal))
            })
            .when(menu == Some(MenuKind::Code), |this| {
                this.child(self.command_item(
                    "code.completion",
                    "Completion",
                    crate::editor_view::Complete,
                ))
                .child(self.command_item(
                    "editor.reformat",
                    "Reformat Code",
                    crate::editor_view::Reformat,
                ))
                .child(Self::action_item(
                    "Signature Help",
                    crate::editor_view::SignatureHelp,
                ))
            })
            .when(menu == Some(MenuKind::Navigate), |this| {
                this.child(self.command_dispatch_item("navigate.back", "Back", cx))
                    .child(self.command_dispatch_item("navigate.forward", "Forward", cx))
                    .child(self.command_dispatch_item("navigate.class", "Go to Class", cx))
                    .child(self.command_dispatch_item("navigate.symbol", "Go to Symbol", cx))
                    .child(self.command_dispatch_item(
                        "navigate.definition",
                        "Go to Definition",
                        cx,
                    ))
                    .child(Self::action_item(
                        "Find References",
                        crate::editor_view::References,
                    ))
                    .child(Self::action_item(
                        "Go to Implementation",
                        GoToImplementation,
                    ))
            })
            .when(menu == Some(MenuKind::Help), |this| {
                this.child(self.command_item(
                    "workspace.commands",
                    "Axiom Commands",
                    CommandPalette,
                ))
                .child(self.command_item("help.features", "Axiom Features", ShowFeatures))
                .child(Self::action_item("About Axiom", ShowAbout))
            });
        div()
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .right(px(0.))
            .h(m.menu_height)
            .flex()
            .flex_col()
            .bg(t.window_background)
            .child(
                div()
                    .h(m.menu_height)
                    .flex()
                    .items_center()
                    .children(labels.into_iter().map(|(label, kind)| {
                        let workspace = workspace.clone();
                        let click_workspace = workspace.clone();
                        div()
                            .id(label)
                            .px_3()
                            .h_full()
                            .flex()
                            .items_center()
                            .text_size(m.ui_font_size)
                            .text_color(t.text_secondary)
                            .hover(move |style| style.bg(t.hover).text_color(t.text_primary))
                            .on_mouse_move(move |event, _, cx| {
                                click_workspace.update(cx, |this, cx| {
                                    if this.open_menu.is_some() && this.open_menu != Some(kind) {
                                        let before = this.open_menu;
                                        this.open_menu = Some(kind);
                                        this.menu_anchor_x = event.position.x;
                                        if debug_input_enabled() {
                                            tracing::info!(
                                                menu_before = ?before,
                                                menu_after = ?this.open_menu,
                                                "[MENU HOVER SWITCH]"
                                            );
                                        }
                                        cx.notify();
                                    }
                                });
                            })
                            .on_click(move |event, _, cx| {
                                if debug_input_enabled() {
                                    tracing::info!(target = %label, "[MOUSE] menu click");
                                }
                                workspace.update(cx, |this, cx| {
                                    let before = this.open_menu;
                                    this.menu_anchor_x = event.position().x;
                                    this.open_menu = (this.open_menu != Some(kind)).then_some(kind);
                                    if debug_input_enabled() {
                                        tracing::info!(
                                            menu_before = ?before,
                                            menu_after = ?this.open_menu,
                                            "[MENU] state changed; notify=true"
                                        );
                                        if this.open_menu == Some(MenuKind::File) {
                                            for (index, (label, command)) in [
                                                ("Open Project…", "project.open_project"),
                                                ("Open File…", "project.open_file"),
                                                ("Save", "editor.save"),
                                                ("Save All", "workspace.save_all"),
                                                ("Close File", "workspace.close_file"),
                                                ("Close Project", "workspace.close_project"),
                                                ("Settings", "settings.open"),
                                                ("Exit", "workspace.exit"),
                                            ]
                                            .into_iter()
                                            .enumerate()
                                            {
                                                tracing::info!(
                                                    index,
                                                    label,
                                                    command,
                                                    "[FILE MENU ITEM]"
                                                );
                                            }
                                        }
                                    }
                                    cx.notify();
                                });
                            })
                            .child(label)
                    })),
            )
            .when(menu.is_some(), |this| this.child(dropdown))
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(t.editor_background)
            .text_color(t.text_primary)
            .child(
                div()
                    .w(px(54.))
                    .h(px(54.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(m.border_radius_medium)
                    .bg(t.accent)
                    .text_color(t.window_background)
                    .text_size(px(22.))
                    .child("RS"),
            )
            .child(div().text_size(px(30.)).child("Axiom"))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(t.text_muted)
                    .child("PHP IDE written in Rust"),
            )
            .child(
                div()
                    .id("welcome-open-project")
                    .px_6()
                    .py_3()
                    .rounded(m.border_radius_small)
                    .bg(t.accent)
                    .text_color(t.window_background)
                    .hover(move |style| style.bg(t.accent_hover))
                    .on_click({
                        let workspace = workspace.clone();
                        move |_, window, cx| {
                            workspace.update(cx, |this, cx| {
                                this.execute_command("project.open_project", window, cx);
                            });
                        }
                    })
                    .child("Open Project"),
            )
            .child(
                div()
                    .mt_4()
                    .text_color(t.text_secondary)
                    .child("Recent Projects"),
            )
            .children(self.recent_projects.existing().map(|entry| {
                let path = entry.path.clone();
                let label = path.display().to_string();
                let workspace = workspace.clone();
                div()
                    .id(SharedString::from(format!("recent:{}", path.display())))
                    .px_4()
                    .py_1()
                    .rounded(m.border_radius_small)
                    .text_color(t.text_secondary)
                    .hover(move |style| style.bg(t.hover).text_color(t.text_primary))
                    .on_click(move |_, _, cx| {
                        workspace.update(cx, |this, cx| {
                            this.request_operation(PendingOperation::OpenProject(path.clone()), cx)
                        });
                    })
                    .child(label)
            }))
    }

    fn render_dialogs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        div().when(self.pending_delete.is_some(), |this| {
            this.absolute()
                .top(px(0.))
                .left(px(0.))
                .right(px(0.))
                .bottom(px(0.))
        })
            .when_some(self.pending_delete.clone(), |this, path| {
                let confirm_workspace = workspace.clone();
                let cancel_workspace = workspace.clone();
                let cancel_backdrop = workspace.clone();
                let delete_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("selected entry")
                    .to_owned();
                this.child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .left(px(0.))
                        .right(px(0.))
                        .bottom(px(0.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::rgba(0x00000055))
                        .cursor(CursorStyle::Arrow)
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            cancel_backdrop.update(cx, |this, cx| {
                                this.pending_delete = None;
                                this.pending_delete_is_directory = false;
                                this.delete_focus_pending = false;
                                this.status = "Deletion cancelled".into();
                                if debug_input_enabled() {
                                    tracing::info!(reason = "backdrop", "[DELETE MODAL CLOSE]");
                                }
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .w(px(430.))
                                .p_4()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .bg(t.popup_background)
                                .border_1()
                                .border_color(t.border)
                                .rounded(m.border_radius_medium)
                                .shadow_lg()
                                .text_color(t.text_primary)
                                .cursor(CursorStyle::Arrow)
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(if self.pending_delete_is_directory {
                                    "Delete Directory?"
                                } else {
                                    "Delete File?"
                                })
                                .child(delete_name)
                                .child(if self.pending_delete_is_directory {
                                    "Delete directory and all of its contents? This action cannot be undone."
                                } else {
                                    "This action cannot be undone."
                                })
                                .child(
                                    div()
                                        .flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            div()
                                                .id("cancel-delete")
                                                .px_3()
                                                .py_1()
                                                .rounded(m.border_radius_small)
                                                .cursor(CursorStyle::PointingHand)
                                                .bg(t.pressed)
                                                .on_click(move |_, _, cx| {
                                                    cancel_workspace.update(cx, |this, cx| {
                                                        this.pending_delete = None;
                                                        this.pending_delete_is_directory = false;
                                                        this.delete_focus_pending = false;
                                                        this.status = "Deletion cancelled".into();
                                                        cx.notify();
                                                    });
                                                })
                                                .child("Cancel"),
                                        )
                                        .child(
                                            div()
                                                .id("confirm-delete")
                                                .px_3()
                                                .py_1()
                                                .rounded(m.border_radius_small)
                                                .cursor(CursorStyle::PointingHand)
                                                .bg(t.error)
                                                .text_color(t.window_background)
                                                .on_click(move |_, _, cx| {
                                                    confirm_workspace.update(cx, |this, cx| this.confirm_delete(cx));
                                                })
                                                .child("Delete"),
                                        ),
                                ),
                        ),
                )
            })
            .when(self.pending_operation.is_some(), |this| {
                let save_workspace = workspace.clone();
                let discard_workspace = workspace.clone();
                let cancel_workspace = workspace.clone();
                this.child(
                    div()
                        .p_3()
                        .flex()
                        .items_center()
                        .gap_3()
                        .bg(t.elevated_surface)
                        .border_b_1()
                        .border_color(t.border)
                        .text_color(t.text_primary)
                        .child("You have unsaved changes.")
                        .child(
                            div()
                                .id("save-continue")
                                .px_3()
                                .py_1()
                                .rounded(m.border_radius_small)
                                .bg(t.accent)
                                .on_click(move |_, _, cx| {
                                    save_workspace.update(cx, |this, cx| {
                                        if this.save_all_now(cx)
                                            && let Some(operation) = this.pending_operation.take()
                                        {
                                            this.perform_operation(operation, cx);
                                        }
                                    });
                                })
                                .child("Save All & Continue"),
                        )
                        .child(
                            div()
                                .id("discard-continue")
                                .px_3()
                                .py_1()
                                .rounded(m.border_radius_small)
                                .bg(t.warning)
                                .on_click(move |_, _, cx| {
                                    discard_workspace.update(cx, |this, cx| {
                                        if let Some(operation) = this.pending_operation.take() {
                                            this.perform_operation(operation, cx);
                                        }
                                    });
                                })
                                .child("Discard"),
                        )
                        .child(
                            div()
                                .id("cancel-operation")
                                .px_3()
                                .py_1()
                                .rounded(m.border_radius_small)
                                .bg(t.pressed)
                                .on_click(move |_, _, cx| {
                                    cancel_workspace.update(cx, |this, cx| {
                                        this.pending_operation = None;
                                        this.status = "Operation cancelled".into();
                                        cx.notify();
                                    });
                                })
                                .child("Cancel"),
                        ),
                )
            })
            .when(self.show_about, |this| {
                let workspace = workspace.clone();
                this.child(
                    div()
                        .p_3()
                        .flex()
                        .items_center()
                        .gap_3()
                        .bg(t.elevated_surface)
                        .border_b_1()
                        .border_color(t.border)
                        .text_color(t.text_primary)
                        .child(format!(
                            "Axiom — IDE for PHP written in Rust — Version {}",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .child(
                            div()
                                .id("close-about")
                                .px_3()
                                .py_1()
                                .rounded(m.border_radius_small)
                                .bg(t.pressed)
                                .on_click(move |_, _, cx| {
                                    workspace.update(cx, |this, cx| {
                                        this.show_about = false;
                                        cx.notify();
                                    });
                                })
                                .child("Close"),
                        ),
                )
            })
    }

    fn render_terminal_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let profile = self
            .terminal_session
            .as_ref()
            .map(|session| session.profile_label())
            .unwrap_or("Terminal");
        let status = self
            .terminal_session
            .as_ref()
            .map(|session| {
                if session.is_exited() {
                    "exited"
                } else {
                    "running"
                }
            })
            .unwrap_or("not started");

        div()
            .h(px(220.))
            .min_h(px(120.))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.border)
            .bg(t.editor_background)
            .child(
                div()
                    .h(m.panel_header_height)
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .bg(t.panel_background)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_color(t.text_primary).child("Terminal"))
                            .child(
                                div()
                                    .text_color(t.text_muted)
                                    .child(format!("{profile} - {status}")),
                            ),
                    )
                    .child(
                        div()
                            .id("close-terminal")
                            .px_2()
                            .rounded(m.border_radius_small)
                            .text_color(t.text_muted)
                            .hover(move |style| style.bg(t.hover).text_color(t.text_primary))
                            .on_click(move |_, _, cx| {
                                workspace.update(cx, |this, cx| {
                                    this.terminal_visible = false;
                                    cx.notify();
                                });
                            })
                            .child("x"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .when_some(self.terminal_view.clone(), |this, terminal| {
                        this.child(terminal)
                    }),
            )
    }

    fn render_features(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        div()
            .absolute()
            .top(px(54.))
            .left(px(180.))
            .w(px(680.))
            .max_h(px(620.))
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .bg(t.window_background)
            .border_1()
            .border_color(t.border)
            .rounded(m.border_radius_medium)
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child("Axiom Features")
                    .child(div().id("close-features").px_2().child("×").on_click(
                        move |_, _, cx| {
                            workspace.update(cx, |this, cx| {
                                this.features_visible = false;
                                cx.notify();
                            });
                        },
                    )),
            )
            .child(
                div()
                    .text_color(t.text_muted)
                    .child("Implemented and available in this build"),
            )
            .children(
                [
                    "Editor",
                    "Navigation",
                    "Code",
                    "Project",
                    "Tool Windows",
                    "Help",
                ]
                .into_iter()
                .map(|category| {
                    let commands = self
                        .keymap
                        .commands()
                        .iter()
                        .filter(|command| command.category == category);
                    div()
                        .mt_2()
                        .child(div().text_color(t.accent).child(category))
                        .children(commands.map(|command| {
                            let shortcut = self
                                .keymap
                                .shortcut(&command.id)
                                .map(Self::format_shortcut)
                                .unwrap_or_else(|| "None".into());
                            div()
                                .flex()
                                .gap_2()
                                .child(command.title.clone())
                                .child(div().text_color(t.text_muted).child(shortcut))
                                .child(
                                    div()
                                        .text_color(t.text_secondary)
                                        .child(format!(" — {}", command.description)),
                                )
                        }))
                }),
            )
    }
}

impl Drop for WorkspaceView {
    fn drop(&mut self) {
        self.runtime_watch_stop.store(true, Ordering::Relaxed);
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.find_usages_focus_pending {
            self.find_usages_focus_pending = false;
            window.focus(&self.find_usages_focus);
        }
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        if let Some(path) = self.startup_file.take() {
            self.open_file(path, window, cx);
        }
        let active_editor = self
            .active
            .and_then(|index| self.tabs.get(index))
            .map(|tab| tab.editor.clone());
        if self.focus_active_editor {
            if let Some(editor) = active_editor.as_ref() {
                let handle = editor.read(cx).focus_handle(cx);
                let focus_before = handle.is_focused(window);
                window.focus(&handle);
                if debug_input_enabled() {
                    tracing::info!(
                        path = %editor.read(cx).document_path().unwrap_or_else(|| Path::new("<untitled>")).display(),
                        focus_before,
                        focus_after = handle.is_focused(window),
                        ready_for_input = handle.is_focused(window),
                        "[EDITOR ACTIVATE]"
                    );
                }
            }
            self.focus_active_editor = false;
        }
        if self.explorer_operation.is_some() && self.modal_focus_pending {
            self.reset_modal_caret();
            window.focus(&self.modal_input_focus);
            self.modal_focus_pending = false;
            if debug_input_enabled() {
                tracing::info!(
                    active = self.modal_input_focus.is_focused(window),
                    "[MODAL INPUT HANDLER]"
                );
            }
            if debug_input_enabled() {
                tracing::info!(target = "name_input", "[MODAL FOCUS REQUEST]");
                tracing::info!(focused = true, "[MODAL FOCUS]");
            }
        }
        if self.pending_delete.is_some() && self.delete_focus_pending {
            window.focus(&self.modal_input_focus);
            self.delete_focus_pending = false;
            if debug_input_enabled() {
                tracing::info!(
                    active = self.modal_input_focus.is_focused(window),
                    "[DELETE MODAL FOCUS]"
                );
            }
        }
        let title = self.project.as_ref().map_or_else(
            || "Axiom".to_owned(),
            |project| {
                self.active
                    .and_then(|index| self.tabs.get(index))
                    .map_or_else(
                        || format!("{} — Axiom", project.name()),
                        |tab| {
                            format!(
                                "{} — {} — Axiom",
                                tab.editor.read(cx).title(),
                                project.name()
                            )
                        },
                    )
            },
        );
        window.set_window_title(&title);
        let lsp_status = match self.lsp.as_ref().map(|lsp| lsp.status()) {
            Some(ServerStatus::Starting) => "Starting",
            Some(ServerStatus::Ready) => "Ready",
            Some(ServerStatus::Stopped) => "Stopped",
            Some(ServerStatus::NotFound) | None => "Not Found",
        };
        let runtime_stub_status = self.runtime_stubs.label();
        let definition_loading_message = format!(
            "Loading definition{}",
            ".".repeat(usize::from(self.definition_loading_tick))
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, |event, _, _| {
                if debug_input_enabled() {
                    tracing::debug!(x = ?event.position.x, y = ?event.position.y, "[MOUSE RAW]");
                }
            })
            .on_key_down(cx.listener(Self::handle_workspace_keydown))
            .on_action(cx.listener(Self::open_project))
            .on_action(cx.listener(Self::open_file_dialog))
            .on_action(cx.listener(Self::save_all))
            .on_action(cx.listener(Self::close_active_file))
            .on_action(cx.listener(Self::close_project_action))
            .on_action(cx.listener(Self::exit))
            .on_action(cx.listener(Self::show_about))
            .on_action(cx.listener(Self::show_features))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::toggle_project))
            .on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(Self::import_runtime_stubs_action))
            .on_action(cx.listener(Self::import_runtime_stub_files_action))
            .on_action(cx.listener(Self::navigate_back))
            .on_action(cx.listener(Self::navigate_forward))
            .on_action(cx.listener(Self::go_to_class))
            .on_action(cx.listener(Self::go_to_symbol))
            .on_action(cx.listener(Self::native_definition_action))
            .on_action(cx.listener(Self::find_usages_action))
            .on_action(cx.listener(Self::go_to_implementation))
            .on_action(cx.listener(Self::command_palette))
            .on_action(cx.listener(Self::settings))
            .on_action(cx.listener(Self::debug_input))
            .on_action(cx.listener(Self::palette_up))
            .on_action(cx.listener(Self::palette_down))
            .on_action(cx.listener(Self::palette_confirm))
            .on_action(cx.listener(Self::palette_escape))
            .bg(t.window_background)
            .text_size(m.ui_font_size)
            .text_color(t.text_primary)
            .child(div().h(m.menu_height))
            .when(self.project.is_none(), |this| this.child(self.render_welcome(cx)))
            .when(self.project.is_some(), |this| this.child(
                div()
                    .id("terminal-tool-window")
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .bg(t.panel_background)
                    .border_t_1()
                    .border_color(t.border_subtle)
                    .text_color(t.text_primary)
                    .hover(|style| style.bg(t.hover))
                    .on_click({
                        let workspace = workspace.clone();
                        move |_, window, cx| {
                            workspace.update(cx, |this, cx| this.toggle_terminal(&ToggleTerminal, window, cx));
                        }
                    })
                    .child("▣  Terminal"),
            ))
            .when(self.project.is_some(), |this| this.child(
                div()
                    .id("workspace-project-area")
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .on_mouse_move(cx.listener(Self::project_panel_resize_move))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(Self::project_panel_resize_end),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(Self::project_panel_resize_end),
                    )
                    .on_hover({
                        let workspace = workspace.clone();
                        move |hovered, _, cx| {
                            workspace.update(cx, |this, cx| {
                                this.explorer_scroll_hovered = *hovered;
                                cx.notify();
                            });
                        }
                    })
                    .child(self.render_activity_bar(cx))
                    .when(self.project_panel_visible, |this| {
                        this.child(self.render_explorer(cx))
                    })
                    .child(
                    div()
                        .flex_1()
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(self.render_tabs(cx))
                        .child(
                            div()
                                .flex_1()
                                .when_some(active_editor, |this, editor| this.child(editor))
                                .when(self.active.is_none(), |this| {
                                    this.flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(t.editor_background)
                                        .text_color(t.text_muted)
                                        .child("Selecione um arquivo no Project Explorer")
                                }),
                        )
                        .when(self.terminal_visible, |this| {
                            this.child(self.render_terminal_panel(cx))
                        }),
                ),
            ))
            .when(self.project.is_some(), |this| this.child(
                div()
                    .h(m.status_bar_height)
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(t.border_subtle)
                    .bg(t.panel_background)
                    .text_size(m.ui_font_size)
                    .text_color(t.text_secondary)
                    .child(self.status.clone())
                    .child(format!(
                        "PHP  ·  Intelephense: {lsp_status}  ·  Runtime Stubs: {runtime_stub_status}  ·  UTF-8"
                    )),
            ))
            .when(self.open_menu.is_some(), |this| {
                this.child(
                    div()
                        .absolute()
                        .top(m.menu_height)
                        .left(px(0.))
                        .right(px(0.))
                        .bottom(px(0.))
                        .id("menu-dismiss-layer")
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                            if let Some(menu) = this.open_menu.take() {
                                if debug_input_enabled() {
                                    tracing::info!(reason = "outside_click", menu = ?menu, "[MENU DISMISS]");
                                }
                                cx.notify();
                            }
                        })),
                )
            })
            .when(self.find_usages_visible, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .left(px(0.))
                        .right(px(0.))
                        .bottom(px(0.))
                        .id("find-usages-dismiss-layer")
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                            this.close_find_usages(&CloseFindUsages, window, cx);
                        })),
                )
                .child(self.render_find_usages(window, cx))
            })
            .when(self.explorer_context.is_some(), |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .left(px(0.))
                        .right(px(0.))
                        .bottom(px(0.))
                        .id("context-menu-dismiss-layer")
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                            if debug_input_enabled() {
                                tracing::info!(
                                    selected_path = ?this.selected_path,
                                    "[CONTEXT MENU OUTSIDE CLICK]"
                                );
                            }
                            window.focus(&this.focus);
                            this.close_context_menu("outside_click", cx);
                        })),
                )
            })
            .when(self.explorer_context.is_some(), |this| {
                this.child(self.render_explorer_context(window, cx))
            })
            .when(self.explorer_operation.is_some(), |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .left(px(0.))
                        .right(px(0.))
                        .bottom(px(0.))
                        .id("modal-backdrop")
                        .bg(gpui::rgba(0x00000055))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(self.render_explorer_operation(window, cx)),
                )
            })
            .child(self.render_menu_bar(window, cx))
            .child(self.render_dialogs(cx))
            .when(self.command_palette_visible, |this| {
                this.child(self.render_command_palette(cx))
            })
            .when(self.settings_visible, |this| this.child(self.render_settings(cx)))
            .when(self.features_visible, |this| this.child(self.render_features(cx)))
            .when(self.definition_loading, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(54.))
                        .right(px(24.))
                        .w(px(260.))
                        .p_3()
                        .flex()
                        .items_center()
                        .rounded(m.border_radius_medium)
                        .bg(t.popup_background)
                        .border_1()
                        .border_color(t.accent)
                        .shadow_lg()
                        .text_color(t.text_primary)
                        .child(definition_loading_message),
                )
            })
            .when(self.debug_overlay_visible, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(260.))
                        .left(px(420.))
                        .right(px(420.))
                        .h(px(100.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(t.error)
                        .text_color(t.window_background)
                        .text_size(px(24.))
                        .child("INPUT TEST ACTIVE"),
                )
            })
            .when(self.index_results.is_some(), |this| {
                this.child(
                    div()
                        .id("project-indexing-overlay")
                        .absolute()
                        .top(px(0.))
                        .left(px(0.))
                        .right(px(0.))
                        .bottom(px(0.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::rgba(0x00000066))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .w(px(360.))
                                .p_5()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .rounded(m.border_radius_medium)
                                .bg(t.popup_background)
                                .border_1()
                                .border_color(t.accent)
                                .shadow_lg()
                                .text_color(t.text_primary)
                                .child(div().text_size(px(16.)).child("Indexing project"))
                                .child(
                                    div()
                                        .relative()
                                        .w(px(300.))
                                        .h(px(5.))
                                        .rounded(px(3.))
                                        .bg(t.border_subtle)
                                        .child(
                                            div()
                                                .absolute()
                                                .left(px(self.indexing_phase as f32 * 2.2))
                                                .top(px(0.))
                                                .w(px(80.))
                                                .h(px(5.))
                                                .rounded(px(3.))
                                                .bg(t.accent),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(t.text_muted)
                                        .child("Preparing completion and navigation…"),
                                ),
                        ),
                )
            })
    }
}

/// Each mounted input has an immutable field identity. Text and selection live
/// only in that field's persistent workspace slots; focus never copies them.
struct ModalInput {
    owner: gpui::WeakEntity<WorkspaceView>,
    field: ModalField,
}

impl EntityInputHandler for ModalInput {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        actual: &mut Option<std::ops::Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        actual.replace(range.clone());
        self.owner
            .update(cx, |owner, _| {
                let text = owner.modal_field_text(self.field);
                text[utf16_to_byte_offset(text, range.start)..utf16_to_byte_offset(text, range.end)]
                    .to_owned()
            })
            .ok()
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        self.owner
            .update(cx, |owner, _| {
                let selection = owner.modal_field_selection(self.field);
                UTF16Selection {
                    range: selection.range.clone(),
                    reversed: selection.reversed,
                }
            })
            .ok()
    }
    fn marked_text_range(
        &self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        None
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}
    fn replace_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.owner.update(cx, |owner, cx| {
            if owner.explorer_operation.is_none() {
                return;
            }
            let range =
                range.unwrap_or_else(|| owner.modal_field_selection(self.field).range.clone());
            owner.modal_replace_field_range(self.field, range, text, cx);
        });
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        _: Option<std::ops::Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range, text, window, cx);
    }
    fn bounds_for_range(
        &mut self,
        _: std::ops::Range<usize>,
        bounds: gpui::Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::Bounds<Pixels>> {
        Some(bounds)
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        self.owner
            .update(cx, |owner, _| {
                let byte = owner.modal_field_geometry[self.field as usize].hit_test(point.x);
                byte_to_utf16_offset(owner.modal_field_text(self.field), byte)
            })
            .ok()
    }
}

impl EntityInputHandler for WorkspaceView {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        actual: &mut Option<std::ops::Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        actual.replace(range.clone());
        let query = if self.explorer_operation.is_some() {
            self.modal_field_text(self.explorer_modal_field)
        } else if self.settings_visible && !self.command_palette_visible {
            &self.settings_query
        } else {
            &self.command_palette_query
        };
        let start = utf16_to_byte_offset(query, range.start);
        let end = utf16_to_byte_offset(query, range.end);
        Some(query[start..end].to_owned())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.explorer_operation.is_some() {
            let selection = self.modal_field_selection(self.explorer_modal_field);
            return Some(UTF16Selection {
                range: selection.range.clone(),
                reversed: selection.reversed,
            });
        }
        Some(UTF16Selection {
            range: {
                let length = if self.explorer_operation.is_some() {
                    self.explorer_input.encode_utf16().count()
                } else if self.settings_visible && !self.command_palette_visible {
                    self.settings_query.encode_utf16().count()
                } else {
                    self.command_palette_query.encode_utf16().count()
                };
                length..length
            },
            reversed: false,
        })
    }
    fn marked_text_range(
        &self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        None
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}
    fn replace_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.explorer_operation.is_some() {
            let range = range.unwrap_or_else(|| {
                self.modal_field_selection(self.explorer_modal_field)
                    .range
                    .clone()
            });
            self.modal_replace_range(range, text, cx);
            return;
        }
        let query = if self.settings_visible && !self.command_palette_visible {
            &mut self.settings_query
        } else {
            &mut self.command_palette_query
        };
        let range = range.unwrap_or_else(|| {
            let end = query.encode_utf16().count();
            end..end
        });
        *query = replace_utf16_range(query, range, text).0;
        self.command_palette_selected = 0;
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        _: Option<std::ops::Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range, text, window, cx);
    }
    fn bounds_for_range(
        &mut self,
        _: std::ops::Range<usize>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        Some(bounds)
    }
    fn character_index_for_point(
        &mut self,
        point: gpui::Point<gpui::Pixels>,
        _window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(if self.explorer_operation.is_some() {
            let byte =
                self.modal_field_geometry[self.explorer_modal_field as usize].hit_test(point.x);
            byte_to_utf16_offset(self.modal_field_text(self.explorer_modal_field), byte)
        } else if self.settings_visible && !self.command_palette_visible {
            self.settings_query.encode_utf16().count()
        } else {
            self.command_palette_query.encode_utf16().count()
        })
    }
}

struct WorkspaceInputElement {
    workspace: Entity<WorkspaceView>,
    focus: FocusHandle,
}
impl IntoElement for WorkspaceInputElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for WorkspaceInputElement {
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
        _: gpui::Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.workspace.clone()),
            cx,
        );
    }
}

impl Focusable for WorkspaceView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.active
            .and_then(|index| self.tabs.get(index))
            .map(|tab| tab.editor.read(cx).focus_handle(cx))
            .unwrap_or_else(|| self.focus.clone())
    }
}

fn utf16_to_byte_offset(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= offset {
            return byte;
        }
        units += ch.len_utf16();
        if units >= offset {
            return byte + ch.len_utf8();
        }
    }
    text.len()
}

fn byte_to_utf16_offset(text: &str, offset: usize) -> usize {
    let byte = offset.min(text.len());
    text.get(..byte).unwrap_or(text).encode_utf16().count()
}

fn replace_utf16_range(
    text: &str,
    range: std::ops::Range<usize>,
    replacement: &str,
) -> (String, usize) {
    let start = utf16_to_byte_offset(text, range.start);
    let end = utf16_to_byte_offset(text, range.end);
    let mut result = text.to_owned();
    result.replace_range(start..end, replacement);
    let caret = range.start + replacement.encode_utf16().count();
    (result, caret)
}

#[cfg(test)]
fn utf16_slice(text: &str, start: usize, end: usize) -> String {
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let start_byte = utf16_to_byte_offset(text, start);
    let end_byte = utf16_to_byte_offset(text, end);
    text[start_byte..end_byte].to_owned()
}

#[cfg(debug_assertions)]
fn debug_keys_enabled() -> bool {
    std::env::var_os("AXIOM_DEBUG_KEYS").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    })
}

#[cfg(not(debug_assertions))]
fn debug_keys_enabled() -> bool {
    false
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

struct UiKeyProbe {
    key: String,
    started: Instant,
}

impl UiKeyProbe {
    fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            started: Instant::now(),
        }
    }
}

impl Drop for UiKeyProbe {
    fn drop(&mut self) {
        if !debug_ui_stall_enabled() {
            return;
        }
        let total_us = self.started.elapsed().as_micros();
        if total_us >= 10_000 {
            tracing::info!(target: "axiom.ui_stall",
                key = %self.key,
                total_us,
                dispatch_us = 0_u128,
                other_us = total_us,
                "[UI KEY CALLBACK]"
            );
        }
    }
}

#[cfg(not(debug_assertions))]
fn debug_input_enabled() -> bool {
    false
}

#[derive(Default)]
struct StubImportReport {
    copied: usize,
    conflicts: usize,
}

fn stub_snapshot(root: &Path) -> std::io::Result<HashMap<PathBuf, (u64, u128)>> {
    fn visit(
        root: &Path,
        current: &Path,
        out: &mut HashMap<PathBuf, (u64, u128)>,
    ) -> std::io::Result<()> {
        if !current.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, out)?;
            } else if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("php"))
            {
                let metadata = fs::metadata(&path)?;
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |duration| duration.as_nanos());
                if let Ok(relative) = path.strip_prefix(root) {
                    out.insert(relative.to_path_buf(), (metadata.len(), modified));
                }
            }
        }
        Ok(())
    }
    let mut snapshot = HashMap::new();
    visit(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn copy_stub_files(files: &[PathBuf], target: &Path) -> std::io::Result<StubImportReport> {
    let mut report = StubImportReport::default();
    fs::create_dir_all(target)?;
    for file in files {
        if file.extension().and_then(|value| value.to_str()) != Some("php") {
            continue;
        }
        let destination = target.join(file.file_name().unwrap_or_default());
        if destination.exists() {
            report.conflicts += 1;
            continue;
        }
        fs::copy(file, destination)?;
        report.copied += 1;
    }
    Ok(report)
}

fn copy_stub_tree(source: &Path, target: &Path) -> std::io::Result<StubImportReport> {
    fn copy_directory(
        source: &Path,
        target: &Path,
        report: &mut StubImportReport,
    ) -> std::io::Result<()> {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let path = entry.path();
            let destination = target.join(entry.file_name());
            if path.is_dir() {
                copy_directory(&path, &destination, report)?;
            } else if path.is_file()
                && path.extension().and_then(|value| value.to_str()) == Some("php")
            {
                if destination.exists() {
                    report.conflicts += 1;
                    continue;
                }
                fs::copy(&path, &destination)?;
                report.copied += 1;
            }
        }
        Ok(())
    }

    let mut report = StubImportReport::default();
    copy_directory(source, target, &mut report)?;
    Ok(report)
}

#[allow(dead_code)]
fn debug_stubs_enabled() -> bool {
    std::env::var_os("AXIOM_DEBUG_INPUT").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    }) || std::env::var_os("AXIOM_DEBUG_STUBS").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    })
}

fn normalize_modifiers(modifiers: Modifiers) -> (bool, bool, bool) {
    (modifiers.control, modifiers.shift, modifiers.alt)
}

fn tab_display_path(path: &Path, project_root: Option<&Path>, runtime_root: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(runtime_root) {
        return format!("Runtime Stub\n{}", relative.display());
    }
    if let Some(root) = project_root
        && let Ok(relative) = path.strip_prefix(root)
    {
        return relative.display().to_string();
    }
    path.display().to_string()
}

#[cfg(test)]
mod modifier_tests {
    #[gpui::test]
    fn references_action_and_enter_use_indexed_unsaved_buffer(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unsaved.php");
        let text = "<?php function run() {} run(); run();";
        let mut builder = axiom_index::SnapshotBuilder::empty(axiom_index::SemanticRevision(1));
        builder.replace_workspace_file(&path, text);
        let engine =
            std::sync::Arc::new(axiom_index::SemanticEngine::from_snapshot(builder.finish()));
        let (workspace, cx) =
            cx.add_window_view(|_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        let editor = cx.new(|cx| {
            let mut editor = EditorView::from_document(
                path.clone(),
                axiom_editor::Document::from_content(text),
                None,
                cx,
            );
            editor.reveal_lsp_position(
                axiom_lsp::PositionCodec::offset_to_position(
                    text,
                    text.find("run").unwrap(),
                    Default::default(),
                ),
                cx,
            );
            editor
        });
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.semantic_engine = Some(engine);
                workspace.tabs.push(OpenTab {
                    path: path.clone(),
                    editor: editor.clone(),
                });
                workspace.active = Some(0);
                workspace.find_usages_action(&crate::editor_view::References, window, cx);
            })
        });
        cx.run_until_parked();
        workspace.update(cx, |workspace, _| {
            assert!(workspace.find_usages_visible, "{}", workspace.status);
            assert_eq!(workspace.find_usages.len(), 2);
        });
        cx.simulate_keystrokes("down enter");
        workspace.update(cx, |workspace, cx| {
            assert!(!workspace.find_usages_visible);
            assert_eq!(workspace.tabs.len(), 1);
            assert_eq!(
                editor.read(cx).current_cursor_offset(),
                text.rfind("run").unwrap()
            );
            assert_eq!(editor.read(cx).document_content(), text);
            assert!(!path.exists());
        });
        // The edit happens before the UI can apply the asynchronous query result.
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.find_usages_action(&crate::editor_view::References, window, cx);
                editor.update(cx, |editor, cx| {
                    editor.apply_formatting(
                        &[lsp_types::TextEdit {
                            range: lsp_types::Range::default(),
                            new_text: " ".into(),
                        }],
                        editor.document_session(),
                        0,
                        cx,
                    )
                });
            })
        });
        cx.run_until_parked();
        workspace.update(cx, |workspace, _| {
            assert!(
                !workspace.find_usages_visible,
                "stale response must not reopen the popup"
            );
            assert!(workspace.find_usages.is_empty());
        });
    }

    #[test]
    fn implementation_preparation_uses_interface_identity_and_deterministic_order() {
        let dir = tempfile::tempdir().unwrap();
        let interface = dir.path().join("Cache.php");
        let redis = dir.path().join("Redis.php");
        let file = dir.path().join("main.php");
        let interface_text = "<?php interface Cache { public function get(): void; }";
        let redis_text =
            "<?php class RedisCache implements \\Cache { public function get(): void {} }";
        let main_text = "<?php class Unrelated {}\n";
        std::fs::write(&interface, interface_text).unwrap();
        std::fs::write(&redis, redis_text).unwrap();
        std::fs::write(&file, main_text).unwrap();
        let mut index = axiom_index::ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = axiom_index::SemanticSnapshot::from_project_index(
            &index,
            axiom_index::SemanticRevision(1),
        );
        let interface_id = snapshot.symbols_for_fqn("Cache")[0];
        let interface_offset = snapshot.symbol(interface_id).unwrap().range.start + 1;
        let targets =
            super::prepare_implementations(&snapshot, &interface, interface_offset).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].file.file_name(), redis.file_name());
        assert!(
            super::prepare_implementations(
                &snapshot,
                &file,
                main_text.find("Unrelated").unwrap() + 1
            )
            .is_err()
        );
    }
    #[gpui::test]
    fn references_popup_keyboard_and_empty_results(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) =
            cx.add_window_view(|_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.find_usages_visible = true;
            workspace.find_usages_focus_pending = true;
            for index in 0..1000 {
                workspace.find_usages.push(super::FindUsageTarget {
                    file: "example.php".into(),
                    span: index..index + 1,
                    role: Some(axiom_index::ReferenceRole::FunctionCall),
                    position: lsp_types::Position::new(index as u32, 0),
                    label: format!("example.php:{index}"),
                    snippet: "$value".into(),
                });
            }
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("down down up");
        workspace.update(cx, |workspace, cx| {
            assert_eq!(workspace.find_usages_selected, 1);
            cx.notify();
        });
        cx.simulate_keystrokes("home end");
        workspace.update(cx, |workspace, cx| {
            assert_eq!(workspace.find_usages_selected, 999);
            workspace.find_usages.clear();
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter escape");
        workspace.update(cx, |workspace, _| assert!(!workspace.find_usages_visible));
    }

    #[test]
    fn references_preparation_prefers_dirty_text_and_rejects_stale_snapshot() {
        use axiom_index::{PersistentFileKey, SemanticRevision, SnapshotBuilder};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dirty.php");
        std::fs::write(&path, "<?php function old() {}").unwrap();
        let dirty = "<?php function run() {} run(); run();";
        let mut builder = SnapshotBuilder::empty(SemanticRevision(1));
        builder.replace_workspace_file(&path, dirty);
        let snapshot = builder.finish();
        let key = PersistentFileKey::workspace(&path);
        let buffers = std::collections::HashMap::from([(key.clone(), dirty.to_owned())]);
        let (targets, _) =
            super::prepare_find_usages(&snapshot, &path, dirty.find("run").unwrap(), buffers)
                .unwrap();
        assert_eq!(targets.len(), 2);
        assert!(targets[0].span.start < targets[1].span.start);
        assert!(
            targets
                .iter()
                .all(|target| &dirty[target.span.clone()] == "run")
        );
        let buffers = std::collections::HashMap::from([(key, format!("{dirty} run();"))]);
        assert!(super::prepare_find_usages(&snapshot, &path, 15, buffers).is_err());
    }

    #[gpui::test]
    fn references_context_rejects_edit_session_project_and_replaced_request(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx) =
            cx.add_window_view(|_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        let path = std::path::PathBuf::from("references.php");
        let editor = cx.new(|cx| {
            EditorView::from_document(
                path.clone(),
                axiom_editor::Document::from_content("<?php function run() {} run();"),
                None,
                cx,
            )
        });
        workspace.update(cx, |workspace, cx| {
            let snapshot = std::sync::Arc::new(axiom_index::SemanticSnapshot::default());
            workspace.semantic_engine = Some(std::sync::Arc::new(
                axiom_index::SemanticEngine::from_snapshot((*snapshot).clone()),
            ));
            let snapshot = workspace.semantic_engine.as_ref().unwrap().snapshot();
            workspace.tabs.push(OpenTab {
                path: path.clone(),
                editor: editor.clone(),
            });
            workspace.active = Some(0);
            let context = std::sync::Arc::new(super::FindUsagesContext {
                kind: super::NavigationQueryKind::References,
                project_generation: workspace.project_semantic_generation,
                snapshot,
                documents: workspace.references_document_stamps(cx),
                source_session: editor.read(cx).document_session(),
            });
            workspace.find_usages_context = Some(context.clone());
            assert!(workspace.references_context_current(&context, cx));
            workspace.project_semantic_generation += 1;
            assert!(!workspace.references_context_current(&context, cx));
            workspace.project_semantic_generation -= 1;
            workspace.tabs[0].path = "renamed.php".into();
            assert!(!workspace.references_context_current(&context, cx));
            workspace.tabs[0].path = path;
            editor.update(cx, |editor, cx| {
                editor.apply_formatting(
                    &[lsp_types::TextEdit {
                        range: lsp_types::Range::default(),
                        new_text: " ".into(),
                    }],
                    editor.document_session(),
                    0,
                    cx,
                );
            });
            assert!(!workspace.references_context_current(&context, cx));
            workspace.find_usages_context = None;
            assert!(!workspace.references_context_current(&context, cx));
        });
    }
    use super::{
        EditorView, EntryKind, ExplorerContext, OpenTab, SemanticDefinitionRoute, StartupTarget,
        WorkspaceView, byte_to_utf16_offset, normalize_modifiers, replace_utf16_range,
        semantic_update_matches, utf16_slice, utf16_to_byte_offset, vendor_allowed_for_route,
    };
    use gpui::{AppContext, Modifiers};
    use std::collections::HashMap;
    use std::collections::HashSet;

    #[gpui::test]
    fn clear_project_discards_explorer_transient_state(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) =
            cx.add_window_view(move |_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.explorer_context = Some(ExplorerContext {
                path: std::path::PathBuf::from("old.php"),
                kind: EntryKind::File,
            });
            workspace.explorer_new_menu_open = true;
            workspace.explorer_operation = Some(super::ExplorerOperation::Rename(
                std::path::PathBuf::from("old.php"),
            ));
            workspace.clear_project(cx);
            assert!(workspace.project.is_none());
            assert!(workspace.explorer_context.is_none());
            assert!(!workspace.explorer_new_menu_open);
            assert!(workspace.explorer_operation.is_none());
        });
    }

    #[gpui::test]
    fn modal_types_keyboard_mouse_and_field_isolation(cx: &mut gpui::TestAppContext) {
        use super::ModalField::*;
        let (view, cx) =
            cx.add_window_view(move |_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        cx.update(|window, cx| view.update(cx, |w, cx| {
            let mut project = axiom_index::ProjectSymbolIndex::new();
            let root = tempfile::tempdir().unwrap();
            project.index_project(root.path()).unwrap();
            project.index_file_text("modal.php", "<?php namespace Psr\\Log; interface LoggerInterface {} interface LoggerOther {} class LoggerBase {}").unwrap();
            w.project_index = Some(std::sync::Arc::new(std::sync::RwLock::new(project)));
            w.explorer_operation = Some(super::ExplorerOperation::NewPhp { directory: "App".into(), keyword: "class" });
            w.explorer_input = "FileStone".into();
            w.explorer_namespace = "App".into();
            w.explorer_file = "Custom.php".into();
            w.explorer_file_auto = false;
            w.set_modal_field(Implements);
            window.focus(&w.modal_field_focus[Implements as usize]);
            w.modal_replace_field_range(Implements, 0..0, "CacheInterface, Log", cx);
            assert_eq!(w.modal_type_items.len(), 2);
            assert!(w.modal_completion_key("down", cx));
            assert_eq!(w.modal_type_selected, 1);
            assert!(w.modal_completion_key("up", cx));
            assert_eq!(w.modal_type_selected, 0);
            assert!(w.modal_completion_key("enter", cx));
            assert_eq!(w.explorer_implements, "CacheInterface, Psr\\Log\\LoggerInterface");
            assert!(w.modal_type_items.is_empty());
            w.modal_replace_field_range(Implements, 0..200, "Log", cx);
            assert!(w.modal_completion_key("escape", cx));
            assert!(w.explorer_operation.is_some());
            assert_eq!(w.explorer_implements, "Log");
            w.refresh_modal_types();
            assert!(!w.modal_completion_key("tab", cx));
            w.cycle_modal_field(false);
            assert_eq!(w.explorer_modal_field, Name);
            assert!(w.modal_type_items.is_empty());
            w.set_modal_field(Extends);
            w.modal_replace_field_range(Extends, 0..200, "Log", cx);
            assert_eq!(w.modal_type_items[0].label, "LoggerBase");
            w.select_php_type("interface", cx);
            assert!(w.modal_type_items.is_empty());
            w.refresh_modal_types();
            assert_eq!(w.modal_type_items.len(), 2);
            w.select_php_type("trait", cx);
            assert!(w.modal_type_context().is_none());
            assert!(w.modal_type_items.is_empty());
            w.select_php_type("enum", cx);
            assert!(w.modal_type_context().is_none());
            w.select_php_type("class", cx);
            w.set_modal_field(Implements);
            window.focus(&w.modal_field_focus[Implements as usize]);
            w.modal_replace_field_range(Implements, 0..200, "Log", cx);
            cx.notify();
        }));
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("modal-type-candidate-0")
            .expect("popup row is rendered");
        let open_bounds = cx.debug_bounds("php-type-modal").unwrap();
        let input_bounds = cx.debug_bounds("modal-field-Implements").unwrap();
        let popup_bounds = cx.debug_bounds("modal-type-completion").unwrap();
        assert_eq!(popup_bounds.size.width, input_bounds.size.width);
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("php-type-modal").unwrap(), open_bounds);
        cx.update(|window, cx| {
            view.update(cx, |w, _| {
                assert_eq!(w.explorer_implements, "Psr\\Log\\LoggerInterface");
                assert!(w.modal_type_items.is_empty());
                assert!(w.modal_field_focus[Implements as usize].is_focused(window));
                assert_eq!(w.explorer_input, "FileStone");
                assert_eq!(w.explorer_namespace, "App");
                assert_eq!(w.explorer_file, "Custom.php");
                let caret = w.explorer_implements.encode_utf16().count();
                assert_eq!(w.modal_field_selection(Implements).range, caret..caret);
            })
        });
    }

    #[gpui::test]
    fn php_inputs_independent_handlers_mouse_tab_and_auto_link(cx: &mut gpui::TestAppContext) {
        use super::ModalField::*;
        use gpui::EntityInputHandler;
        let (view, cx) =
            cx.add_window_view(move |_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        let inputs = view.update(cx, |w, _| {
            w.explorer_operation = Some(super::ExplorerOperation::NewPhp {
                directory: "App".into(),
                keyword: "class",
            });
            w.explorer_namespace = "App".into();
            w.modal_inputs.clone()
        });
        let ids: std::collections::HashSet<_> =
            inputs.iter().map(|input| input.entity_id()).collect();
        assert_eq!(ids.len(), 5);
        cx.update(|window, cx| {
            inputs[0].update(cx, |input, cx| {
                input.replace_text_in_range(None, "FileStone", window, cx)
            });
            view.update(cx, |w, _| {
                assert_eq!(w.explorer_input, "FileStone");
                assert_eq!(w.explorer_namespace, "App");
                assert_eq!(w.explorer_file, "FileStone.php");
            });
            for (index, text) in [
                (1, "App\\Service"),
                (3, "BaseService"),
                (4, "CacheInterface"),
            ] {
                inputs[index].update(cx, |input, cx| {
                    input.replace_text_in_range(Some(0..100), text, window, cx)
                });
            }
            let expected = [
                "FileStone",
                "App\\Service",
                "FileStone.php",
                "BaseService",
                "CacheInterface",
            ];
            view.update(cx, |w, cx| {
                for field in [Name, Namespace, File, Extends, Implements] {
                    let event = gpui::MouseDownEvent {
                        button: gpui::MouseButton::Left,
                        position: gpui::point(gpui::px(0.), gpui::px(0.)),
                        modifiers: Default::default(),
                        click_count: 1,
                        first_mouse: false,
                    };
                    w.modal_pointer_down(field, &event, window, cx);
                    for (i, other) in [Name, Namespace, File, Extends, Implements]
                        .into_iter()
                        .enumerate()
                    {
                        assert_eq!(w.modal_field_text(other), expected[i]);
                        assert_eq!(w.modal_field_focus[i].is_focused(window), other == field);
                        for blink in [false, true] {
                            w.modal_caret_visible = blink;
                            assert_eq!(w.modal_caret_for(other, window), blink && other == field);
                        }
                    }
                }
                for backwards in [false, true] {
                    for _ in 0..5 {
                        let event = gpui::KeyDownEvent {
                            keystroke: gpui::Keystroke {
                                key: "tab".into(),
                                key_char: None,
                                modifiers: gpui::Modifiers {
                                    shift: backwards,
                                    ..Default::default()
                                },
                            },
                            is_held: false,
                        };
                        w.handle_workspace_keydown(&event, window, cx);
                        for (i, field) in [Name, Namespace, File, Extends, Implements]
                            .into_iter()
                            .enumerate()
                        {
                            assert_eq!(w.modal_field_text(field), expected[i]);
                        }
                    }
                }
                window.focus(&w.focus);
                for field in [Name, Namespace, File, Extends, Implements] {
                    assert!(!w.modal_caret_for(field, window));
                }
            });
            inputs[2].update(cx, |input, cx| {
                input.replace_text_in_range(Some(0..100), "Custom.php", window, cx)
            });
            inputs[0].update(cx, |input, cx| {
                input.replace_text_in_range(Some(0..100), "Other", window, cx)
            });
            view.update(cx, |w, cx| {
                assert!(!w.explorer_file_auto);
                assert_eq!(w.explorer_file, "Custom.php");
                for keyword in ["interface", "trait", "enum"] {
                    w.select_php_type(keyword, cx);
                    assert_eq!(w.explorer_input, "Other");
                    assert_eq!(w.explorer_namespace, "App\\Service");
                    assert_eq!(w.explorer_file, "Custom.php");
                    if keyword == "interface" {
                        assert_eq!(w.explorer_extends, "BaseService");
                    }
                }
            });
        });
    }

    #[gpui::test]
    fn php_inputs_psr4_initializes_only_namespace(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("App")).unwrap();
        std::fs::write(
            dir.path().join("composer.json"),
            r#"{"autoload":{"psr-4":{"App\\":"App/"}}}"#,
        )
        .unwrap();
        let project = axiom_project::Project::open(dir.path()).unwrap();
        let selected_directory = project.root_path().join("App");
        let (view, cx) =
            cx.add_window_view(move |_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        view.update(cx, |w, cx| {
            w.project = Some(project);
            w.new_php_item(selected_directory, "class", cx);
            assert_eq!(w.explorer_namespace, "App");
            assert_eq!(w.explorer_input, "NewItem");
            assert_eq!(w.explorer_file, "NewItem.php");
            assert!(w.explorer_extends.is_empty());
            assert!(w.explorer_implements.is_empty());
        });
    }

    #[gpui::test]
    fn php_inputs_shortcuts_and_selection_are_field_local(cx: &mut gpui::TestAppContext) {
        use super::ModalField::*;
        let (view, cx) =
            cx.add_window_view(move |_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        view.update(cx, |w, cx| {
            w.explorer_operation = Some(super::ExplorerOperation::NewPhp {
                directory: "App".into(),
                keyword: "class",
            });
            w.explorer_input = "FileStone".into();
            w.explorer_file = "FileStone.php".into();
            let control = gpui::Modifiers {
                control: true,
                ..Default::default()
            };
            for field in [Namespace, File, Extends, Implements] {
                w.set_modal_field(field);
                w.modal_key_edit("a", control, cx);
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("a🙂b".into()));
                w.modal_key_edit("v", control, cx);
                w.modal_key_edit("left", Default::default(), cx);
                assert_eq!(w.modal_field_selection(field).range, 3..3);
                w.modal_key_edit("backspace", Default::default(), cx);
                assert_eq!(w.modal_field_text(field), "ab");
                w.modal_key_edit("delete", Default::default(), cx);
                assert_eq!(w.modal_field_text(field), "a");
                w.modal_key_edit("a", control, cx);
                w.modal_key_edit("c", control, cx);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "a");
                w.modal_key_edit("x", control, cx);
                assert_eq!(w.modal_field_text(field), "");
                w.modal_key_edit("v", control, cx);
                w.set_modal_field(Name);
                assert_eq!(w.modal_field_text(field), "a");
                assert_eq!(w.modal_field_selection(field).range, 1..1);
                assert_eq!(w.explorer_input, "FileStone");
            }
        });
    }

    #[gpui::test]
    fn php_inputs_focus_preserves_filestone_and_namespace(cx: &mut gpui::TestAppContext) {
        let (view, cx) =
            cx.add_window_view(move |_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        view.update(cx, |workspace, cx| {
            workspace.explorer_operation = Some(super::ExplorerOperation::NewPhp {
                directory: "App".into(),
                keyword: "class",
            });
            workspace.explorer_namespace = "App".into();
            workspace.modal_replace_range(0..0, "FileStone", cx);
            workspace.cycle_modal_field(false);
            assert_eq!(workspace.explorer_input, "FileStone");
            assert_eq!(workspace.explorer_namespace, "App");
            assert_eq!(workspace.explorer_file, "FileStone.php");
            workspace.set_modal_field_at(super::ModalField::File, 0);
            assert_eq!(workspace.explorer_input, "FileStone");
        });
    }

    #[gpui::test]
    fn new_file_input_clipboard_selection_and_unicode_editing(cx: &mut gpui::TestAppContext) {
        let (view, cx) =
            cx.add_window_view(move |_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        view.update(cx, |workspace, cx| {
            workspace.explorer_operation = Some(super::ExplorerOperation::NewFile(
                std::path::PathBuf::from("E:/dev"),
            ));
            workspace.explorer_input = "old.txt".into();
            let control = gpui::Modifiers {
                control: true,
                ..Default::default()
            };
            assert!(workspace.modal_key_edit("a", control, cx));
            assert_eq!(workspace.explorer_selection.range, 0..7);
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("a🙂b.txt".into()));
            assert!(workspace.modal_key_edit("v", control, cx));
            assert_eq!(workspace.explorer_input, "a🙂b.txt");
            workspace.explorer_selection.range = 3..3;
            workspace.modal_key_edit("left", Default::default(), cx);
            assert_eq!(workspace.explorer_selection.range, 1..1);
            workspace.modal_key_edit("right", Default::default(), cx);
            assert_eq!(workspace.explorer_selection.range, 3..3);
            workspace.modal_key_edit("backspace", Default::default(), cx);
            assert_eq!(workspace.explorer_input, "ab.txt");
            workspace.modal_key_edit("delete", Default::default(), cx);
            assert_eq!(workspace.explorer_input, "a.txt");
            assert!(matches!(
                workspace.explorer_operation,
                Some(super::ExplorerOperation::NewFile(_))
            ));
        });
    }

    #[gpui::test]
    fn new_file_cut_consumes_shortcut_and_collapses_unicode_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let handle = cx.add_window(|_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        handle
            .update(cx, |workspace, window, cx| {
                workspace.explorer_operation =
                    Some(super::ExplorerOperation::NewFile("E:/dev".into()));
                workspace.explorer_input = "a🙂b.txt".into();
                workspace.explorer_selection.range = 1..3;
                window.focus(&workspace.modal_input_focus);
                let event = gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke {
                        key: "x".into(),
                        key_char: None,
                        modifiers: gpui::Modifiers {
                            control: true,
                            ..Default::default()
                        },
                    },
                    is_held: false,
                };
                workspace.handle_workspace_keydown(&event, window, cx);
                assert_eq!(workspace.explorer_input, "ab.txt");
                assert_eq!(workspace.explorer_selection.range, 1..1);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "🙂");
                workspace.handle_workspace_keydown(&event, window, cx);
                assert_eq!(workspace.explorer_input, "ab.txt");
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "🙂");
                assert!(workspace.modal_caret_visible);
            })
            .unwrap();
    }

    #[gpui::test]
    fn navigation_text_prefers_dirty_open_buffer_and_preserves_utf8_byte_spans(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Service.php");
        std::fs::write(
            &path,
            "<?php\nclass Service { public function old(): void {} }\n",
        )
        .unwrap();
        let dirty =
            "<?php\n$label = \"ação ç ã 😀\";\nclass Service { public function run(): void {} }\n";
        let path_for_editor = path.clone();
        let dirty_for_editor = dirty.to_owned();
        let (workspace, cx) =
            cx.add_window_view(move |_, cx| WorkspaceView::new(StartupTarget::Welcome, cx));
        let editor = cx.new(|cx| {
            EditorView::from_document(
                path_for_editor.clone(),
                axiom_editor::Document::from_content(&dirty_for_editor),
                None,
                cx,
            )
        });
        workspace.update(cx, |workspace, _| {
            workspace.tabs.push(OpenTab {
                path: path_for_editor,
                editor,
            });
        });
        let (text, source) = cx.read(|app| {
            let workspace = workspace.read(app);
            workspace.current_text_for_path(&path, app).unwrap()
        });
        assert_eq!(source, super::TargetTextSource::Memory);
        let start = text.rfind("run").unwrap();
        assert_eq!(&text[start..start + 3], "run");
        let closed_path = dir.path().join("Closed.php");
        std::fs::write(&closed_path, "<?php class Closed {}\n").unwrap();
        let (closed_text, closed_source) = cx.read(|app| {
            let workspace = workspace.read(app);
            workspace.current_text_for_path(&closed_path, app).unwrap()
        });
        assert_eq!(closed_source, super::TargetTextSource::Disk);
        assert!(closed_text.contains("class Closed"));
    }

    #[test]
    fn stale_semantic_project_generations_are_rejected() {
        assert!(!semantic_update_matches(3, 1));
        assert!(!semantic_update_matches(3, 2));
        assert!(semantic_update_matches(3, 3));
        assert!(!semantic_update_matches(3, 4));
    }

    #[test]
    fn definition_routing_never_sends_stale_or_ambiguous_results_to_vendor() {
        use axiom_index::SemanticDefinitionOutcome;

        assert!(!vendor_allowed_for_route(SemanticDefinitionRoute::Outcome(
            SemanticDefinitionOutcome::StaleSnapshot,
        )));
        assert!(!vendor_allowed_for_route(SemanticDefinitionRoute::Outcome(
            SemanticDefinitionOutcome::Ambiguous,
        )));
        assert!(!vendor_allowed_for_route(SemanticDefinitionRoute::Resolved));
        assert!(vendor_allowed_for_route(SemanticDefinitionRoute::Outcome(
            SemanticDefinitionOutcome::DeferredVendor,
        )));
        assert!(vendor_allowed_for_route(
            SemanticDefinitionRoute::Unavailable
        ));
    }

    #[test]
    fn preserves_plain_control_shift_and_alt() {
        assert_eq!(
            normalize_modifiers(Modifiers {
                control: true,
                shift: true,
                alt: false,
                ..Default::default()
            }),
            (true, true, false)
        );
        assert_eq!(
            normalize_modifiers(Modifiers {
                control: true,
                shift: false,
                alt: true,
                ..Default::default()
            }),
            (true, false, true)
        );
    }

    #[test]
    fn modal_text_ranges_use_utf16_offsets() {
        let text = "A😀B";
        assert_eq!(&text[..utf16_to_byte_offset(text, 1)], "A");
        assert_eq!(&text[..utf16_to_byte_offset(text, 3)], "A😀");
        assert_eq!(utf16_to_byte_offset(text, 4), text.len());
    }

    #[test]
    fn modal_selection_replacement_preserves_extension_boundary() {
        let value = "test.php";
        let end = value
            .rsplit_once('.')
            .map(|(basename, _)| basename.encode_utf16().count())
            .unwrap();
        let start_byte = utf16_to_byte_offset(value, 0);
        let end_byte = utf16_to_byte_offset(value, end);
        let mut replaced = value.to_owned();
        replaced.replace_range(start_byte..end_byte, "Example");
        assert_eq!(replaced, "Example.php");
    }

    #[test]
    fn rename_insertions_preserve_the_logical_caret() {
        assert_eq!(
            replace_utf16_range("test.php", 0..0, "X"),
            ("Xtest.php".into(), 1)
        );
        assert_eq!(
            replace_utf16_range("test.php", 2..2, "X"),
            ("teXst.php".into(), 3)
        );
        assert_eq!(
            replace_utf16_range("test.php", 4..4, "X"),
            ("testX.php".into(), 5)
        );
        assert_eq!(
            replace_utf16_range("test.php", 8..8, "X"),
            ("test.phpX".into(), 9)
        );
    }

    #[test]
    fn rename_initial_selection_excludes_extension() {
        let value = "test.php";
        let basename_len = value
            .rsplit_once('.')
            .map(|(basename, _)| basename.encode_utf16().count())
            .unwrap();
        assert_eq!(basename_len, 4);
    }

    #[test]
    fn vendor_definition_requests_deduplicate_loading_and_allow_ready_retry() {
        let mut inflight = HashSet::new();
        let fqn = "Omegaalfa\\FiberEventLoop\\FiberEventLoop".to_owned();
        assert!(inflight.insert(fqn.clone()));
        assert!(!inflight.insert(fqn.clone()));
        inflight.remove(&fqn);
        assert!(inflight.insert(fqn));
    }

    #[test]
    fn definition_cache_reuses_existing_target_after_tab_close() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AsyncHttpClient.php");
        std::fs::write(&path, "<?php class AsyncHttpClient {}").unwrap();
        let mut cache = HashMap::new();
        cache.insert(
            "project::AsyncHttpClient".to_owned(),
            super::DefinitionTarget {
                path: path.clone(),
                position: lsp_types::Position::new(0, 7),
            },
        );
        let target = super::definition_cache_lookup(&cache, "project::AsyncHttpClient");
        assert_eq!(target.map(|target| target.path), Some(path));
    }

    #[test]
    fn rename_unicode_insert_uses_utf16_selection() {
        let (result, caret) = replace_utf16_range("João.php", 2..2, "X");
        assert_eq!(result, "JoXão.php");
        assert_eq!(caret, 3);
        assert_eq!(byte_to_utf16_offset(&result, 3), 3);
    }

    #[test]
    fn modal_selection_slice_handles_unicode_ranges() {
        assert_eq!(utf16_slice("A😀B.php", 0, 3), "A😀");
        assert_eq!(utf16_slice("A😀B.php", 3, 4), "B");
    }
}
