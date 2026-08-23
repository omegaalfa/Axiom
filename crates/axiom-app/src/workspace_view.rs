use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Instant,
};

use axiom_app::commands::Keymap;
use axiom_app::shell_state::{
    RecentProjects, StartupTarget, recent_projects_path, unix_timestamp_now,
};
use axiom_editor::Document;
use axiom_index::ProjectSymbolIndex;
use axiom_lsp::{ServerStatus, uri_to_path};
use axiom_php::{RuntimeSymbolIndex, StubProvider};
use axiom_project::{EntryKind, FileContent, Project, ProjectEntry, read_file_content};
use axiom_terminal::{TerminalLink, TerminalLinkKind, TerminalProfile, TerminalSession};
use gpui::{
    Action, App, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, KeyDownEvent,
    LayoutId, Modifiers, MouseButton, Pixels, Point, SharedString, Style, TextRun, Timer,
    UTF16Selection, Window, actions, div, font, prelude::*, px, relative,
};

use crate::{
    editor_view::EditorView,
    lsp_bridge::{IdeLspEvent, LspBridge},
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

impl RuntimeStubStatus {
    fn label(&self) -> String {
        match self {
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
    lsp: Option<std::sync::Arc<LspBridge>>,
    runtime_stubs: RuntimeStubStatus,
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
    explorer_namespace: String,
    explorer_extends: String,
    explorer_implements: String,
    modal_input_focus: FocusHandle,
    modal_focus_pending: bool,
    delete_focus_pending: bool,
    explorer_selection: UTF16Selection,
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
    project_index: Option<Arc<RwLock<ProjectSymbolIndex>>>,
    index_generation: u64,
    index_results: Option<Receiver<(u64, Result<ProjectSymbolIndex, String>)>>,
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
}

impl WorkspaceView {
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
                    .flex()
                    .child(
                        div()
                            .w(px(180.))
                            .p_3()
                            .bg(t.panel_background)
                            .child("Keymap")
                            .child(div().mt_2().text_color(t.text_muted).child("PHP"))
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
        let (runtime_stubs, runtime_symbols) = Self::load_runtime_stubs();
        let recent_path = recent_projects_path();
        let recent_projects = recent_path
            .as_deref()
            .map(RecentProjects::load)
            .unwrap_or_default();
        let keymap = Keymap::load_user();
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
            lsp: None,
            runtime_stubs,
            _runtime_symbols: runtime_symbols,
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
            explorer_namespace: String::new(),
            explorer_extends: String::new(),
            explorer_implements: String::new(),
            modal_input_focus: cx.focus_handle(),
            modal_focus_pending: false,
            delete_focus_pending: false,
            explorer_selection: UTF16Selection {
                range: 0..0,
                reversed: false,
            },
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
            project_index: None,
            index_generation: 0,
            index_results: None,
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
        };
        if let StartupTarget::Project { root, initial_file } = startup {
            workspace.begin_open_project(root, cx);
            workspace.startup_file = initial_file;
        }
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(std::time::Duration::from_millis(100)).await;
                if this
                    .update(cx, |this, cx| {
                        this.poll_lsp(cx);
                        this.poll_index(cx);
                        this.poll_project_load(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        workspace
    }

    fn load_runtime_stubs() -> (
        RuntimeStubStatus,
        Option<std::sync::Arc<RuntimeSymbolIndex>>,
    ) {
        let Some(provider) = StubProvider::from_env() else {
            return (RuntimeStubStatus::NotFound, None);
        };
        match provider.load() {
            Ok((index, report)) => (
                RuntimeStubStatus::Loaded {
                    files: report.files_parsed,
                    symbols: report.symbols_indexed,
                },
                Some(std::sync::Arc::new(index)),
            ),
            Err(error) => {
                tracing::warn!(%error, "PHP runtime stubs unavailable");
                (RuntimeStubStatus::NotFound, None)
            }
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
        let root = project.root_path().to_path_buf();
        self.index_generation = self.index_generation.wrapping_add(1);
        let generation = self.index_generation;
        let (sender, receiver) = mpsc::channel();
        self.index_results = Some(receiver);
        self.project_index = None;
        self.status = "Project opened — indexing...".into();
        let index_root = root.clone();
        thread::spawn(move || {
            let mut index = ProjectSymbolIndex::new();
            let result = index
                .index_project(&index_root)
                .map(|_| index)
                .map_err(|error| error.to_string());
            let _ = sender.send((generation, result));
        });
        match Ok::<_, axiom_project::ProjectError>(entries) {
            Ok(entries) => {
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
            Err(error) => self.status = format!("Falha ao ler projeto: {error}").into(),
        }
    }

    fn poll_index(&mut self, cx: &mut Context<Self>) {
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
                let report = index.report();
                let shared = Arc::new(RwLock::new(index));
                self.project_index = Some(shared.clone());
                for tab in &self.tabs {
                    tab.editor
                        .update(cx, |editor, _| editor.set_project_symbols(shared.clone()));
                }
                self.status = format!("PHP • {} symbols", report.symbols).into();
            }
            Err(error) => self.status = format!("PHP Index Failed: {error}").into(),
        }
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
        self.navigation_back.clear();
        self.navigation_forward.clear();
        for tab in &self.tabs {
            tab.editor.read(cx).close_lsp_document();
        }
        self.tabs.clear();
        self.active = None;
        self.explorer.clear();
        self.expanded.clear();
        self.project = None;
        self.project_index = None;
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
            if let Some(index) = &self.project_index {
                let editor = tab.editor.read(cx);
                if let Some(path) = editor.document_path() {
                    if let Ok(mut index) = index.write() {
                        let _ = index.index_file_text(path, editor.document_text());
                    }
                }
            }
        }
        self.status = if errors.is_empty() {
            "All files saved".into()
        } else {
            format!("Save All failed: {}", errors.join("; ")).into()
        };
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

    fn find(&mut self, _: &Find, _: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.status = "Find UI is deferred; editor text search is not implemented yet".into();
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
                && let Ok(index) = index.read()
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

    fn navigate_native_definition(&mut self, cx: &mut Context<Self>) {
        let native = self
            .active
            .and_then(|index| self.tabs.get(index))
            .and_then(|tab| tab.editor.read(cx).native_definition_location());
        if let Some((path, position)) = native {
            if debug_input_enabled() {
                tracing::info!(path = %path.display(), line = position.line, character = position.character, "[NAVIGATION TARGET]");
            }
            self.navigate_to_definition(DefinitionTarget { path, position }, cx);
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
        self.navigate_native_definition(cx);
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
        if self.explorer_operation.is_some() {
            if debug_input_enabled() && !matches!(key.as_str(), "escape" | "enter") {
                tracing::info!(key = %key, "[MODAL INPUT]");
            }
            match key.as_str() {
                "escape" => self.cancel_explorer_operation(cx),
                "enter" => self.confirm_explorer_operation(cx),
                _ if self.modal_key_edit(&key, event.keystroke.modifiers, cx) => {}
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
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::NewPhpFile(directory));
        cx.notify();
    }

    fn new_directory(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
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
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::NewDirectory(directory));
        cx.notify();
    }

    fn new_php_item(&mut self, directory: PathBuf, keyword: &'static str, cx: &mut Context<Self>) {
        if self.explorer_context.is_some() && debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, reason = "action", "[CONTEXT MENU CLOSE]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_undo.clear();
        self.explorer_input = "NewItem".into();
        self.explorer_selection = UTF16Selection {
            range: 0..self.explorer_input.encode_utf16().count(),
            reversed: false,
        };
        self.modal_focus_pending = true;
        self.explorer_namespace = self
            .project
            .as_ref()
            .and_then(|project| project.path_to_namespace(directory.join("NewItem.php")))
            .and_then(|value| value.rsplit_once('\\').map(|(prefix, _)| prefix.to_owned()))
            .unwrap_or_default();
        self.explorer_extends.clear();
        self.explorer_implements.clear();
        self.explorer_operation = Some(ExplorerOperation::NewPhp { directory, keyword });
        cx.notify();
    }

    fn rename_entry(&mut self, path: PathBuf, cx: &mut Context<Self>) {
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
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::Rename(path));
        cx.notify();
    }

    fn cancel_explorer_operation(&mut self, cx: &mut Context<Self>) {
        self.explorer_operation = None;
        self.explorer_new_menu_open = false;
        self.modal_focus_pending = false;
        self.explorer_input.clear();
        self.explorer_undo.clear();
        self.explorer_namespace.clear();
        self.explorer_extends.clear();
        self.explorer_implements.clear();
        cx.notify();
    }

    fn modal_replace_range(
        &mut self,
        range: std::ops::Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
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

    fn modal_key_edit(&mut self, key: &str, modifiers: Modifiers, cx: &mut Context<Self>) -> bool {
        let length = self.explorer_input.encode_utf16().count();
        let start = self
            .explorer_selection
            .range
            .start
            .min(self.explorer_selection.range.end);
        let end = self
            .explorer_selection
            .range
            .start
            .max(self.explorer_selection.range.end);
        if modifiers.control
            && key == "z"
            && self
                .explorer_operation
                .as_ref()
                .is_some_and(|operation| matches!(operation, ExplorerOperation::Rename(_)))
        {
            if let Some((value, selection)) = self.explorer_undo.pop() {
                self.explorer_input = value;
                self.explorer_selection = selection;
                if debug_input_enabled() {
                    tracing::info!(
                        selection = ?self.explorer_selection.range,
                        "[RENAME UNDO]"
                    );
                }
                cx.notify();
            }
            return true;
        }
        if modifiers.control && key == "a" {
            self.explorer_selection = UTF16Selection {
                range: 0..length,
                reversed: false,
            };
            cx.notify();
            return true;
        }
        if key == "backspace" {
            if start != end {
                self.modal_replace_range(start..end, "", cx);
            } else if start > 0 {
                self.modal_replace_range(start - 1..start, "", cx);
            }
            return true;
        }
        if key == "delete" {
            if start != end {
                self.modal_replace_range(start..end, "", cx);
            } else if end < length {
                self.modal_replace_range(end..end + 1, "", cx);
            }
            return true;
        }
        if matches!(key, "left" | "right") {
            let next = if key == "left" {
                start.saturating_sub(1)
            } else {
                end.min(length).saturating_add(1).min(length)
            };
            self.explorer_selection = UTF16Selection {
                range: next..next,
                reversed: false,
            };
            cx.notify();
            return true;
        }
        false
    }

    fn confirm_explorer_operation(&mut self, cx: &mut Context<Self>) {
        let Some(operation) = self.explorer_operation.take() else {
            return;
        };
        self.modal_focus_pending = false;
        let name = self.explorer_input.trim().to_owned();
        self.explorer_input.clear();
        self.explorer_undo.clear();
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
            self.status = "Invalid name".into();
            cx.notify();
            return;
        }
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let result = match operation {
            ExplorerOperation::NewFile(directory) => {
                project.create_file(&directory, &name).map(Some)
            }
            ExplorerOperation::NewPhpFile(directory) => {
                let name = if name.ends_with(".php") {
                    name
                } else {
                    format!("{name}.php")
                };
                project.create_file(&directory, &name).map(Some)
            }
            ExplorerOperation::NewPhp { directory, keyword } => {
                let name = if name.ends_with(".php") {
                    name
                } else {
                    format!("{name}.php")
                };
                let symbol = name.trim_end_matches(".php");
                let namespace_line = if self.explorer_namespace.trim().is_empty() {
                    String::new()
                } else {
                    format!("\nnamespace {};\n", self.explorer_namespace.trim())
                };
                let extends = if self.explorer_extends.trim().is_empty() {
                    String::new()
                } else {
                    format!(" extends {}", self.explorer_extends.trim())
                };
                let implements = if self.explorer_implements.trim().is_empty() {
                    String::new()
                } else {
                    format!(" implements {}", self.explorer_implements.trim())
                };
                let inheritance = if keyword == "class" {
                    format!("{extends}{implements}")
                } else {
                    String::new()
                };
                let declaration = format!("{keyword} {symbol}{inheritance}");
                let body = format!("<?php\n{namespace_line}\n{declaration}\n{{\n}}\n");
                project.create_file(&directory, &name).map(|path| {
                    let _ = fs::write(&path, body);
                    Some(path)
                })
            }
            ExplorerOperation::NewDirectory(directory) => {
                project.create_directory(&directory, &name).map(|_| None)
            }
            ExplorerOperation::Rename(path) => project.rename(&path, &name).map(|destination| {
                for tab in &mut self.tabs {
                    if let Ok(relative) = tab.path.strip_prefix(&path) {
                        tab.path = destination.join(relative);
                        tab.editor
                            .update(cx, |editor, _| editor.relocate_path(&path, &destination));
                    }
                }
                None
            }),
        };
        match result {
            Ok(Some(path)) => {
                self.refresh_explorer(cx);
                self.open_file_background(path, cx);
            }
            Ok(None) => self.refresh_explorer(cx),
            Err(error) => self.status = format!("Operation failed: {error}").into(),
        }
        cx.notify();
    }

    fn request_delete(&mut self, path: PathBuf, cx: &mut Context<Self>) {
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
        let Some(path) = self.pending_delete.take() else {
            return;
        };
        self.delete_focus_pending = false;
        self.pending_delete_is_directory = false;
        match self
            .project
            .as_ref()
            .expect("project is open")
            .delete(&path)
        {
            Ok(()) => {
                if debug_input_enabled() {
                    tracing::info!(success = true, "[DELETE RESULT]");
                }
                for tab in self.tabs.iter().filter(|tab| tab.path.starts_with(&path)) {
                    tab.editor.read(cx).close_lsp_document();
                }
                self.tabs.retain(|tab| !tab.path.starts_with(&path));
                self.active = if self.tabs.is_empty() {
                    None
                } else {
                    Some(self.active.unwrap_or(0).min(self.tabs.len() - 1))
                };
                self.refresh_explorer(cx);
                self.status = "Entry deleted".into();
            }
            Err(error) => {
                if debug_input_enabled() {
                    tracing::info!(success = false, error = %error, "[DELETE RESULT]");
                }
                self.status = format!("Falha ao excluir: {error}").into();
                cx.notify();
            }
        }
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
            cx.notify();
            return;
        }
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
            editor.update(cx, |editor, _| editor.set_project_symbols(index.clone()));
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
            editor.update(cx, |editor, _| editor.set_project_symbols(index.clone()));
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
            window.focus(&tab.editor.read(cx).focus_handle(cx));
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
        let Some(lsp) = self.lsp.clone() else { return };
        let events = lsp.drain_events();
        if events.is_empty() {
            return;
        }
        for event in events {
            match event {
                IdeLspEvent::Diagnostics(params) => {
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&params.uri))
                    {
                        tab.editor.update(cx, |editor, cx| {
                            editor.set_diagnostics(params.version, params.diagnostics, cx)
                        });
                    }
                }
                IdeLspEvent::Completion { uri, items } => {
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&uri))
                    {
                        tab.editor
                            .update(cx, |editor, cx| editor.set_completions(items, cx));
                    }
                }
                IdeLspEvent::Formatting { uri, edits } => {
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&uri))
                    {
                        tab.editor
                            .update(cx, |editor, cx| editor.apply_formatting(&edits, cx));
                    }
                }
                IdeLspEvent::SignatureHelp { uri, text } => {
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&uri))
                    {
                        tab.editor
                            .update(cx, |editor, cx| editor.set_signature_help(text, cx));
                    }
                }
                IdeLspEvent::Hover { uri, text } => {
                    if let Some(tab) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.editor.read(cx).lsp_uri() == Some(&uri))
                    {
                        tab.editor
                            .update(cx, |editor, cx| editor.set_hover(text, cx));
                    }
                }
                IdeLspEvent::Definition { locations } => {
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
                IdeLspEvent::References { count } => {
                    self.status = format!("{count} referência(s) encontrada(s)").into();
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
        let path = match fs::canonicalize(&target.path) {
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
            let document = match Document::from_file(&path) {
                Ok(document) => document,
                Err(error) => {
                    self.status = format!("Falha ao abrir definition: {error}").into();
                    return;
                }
            };
            let lsp = self.lsp.clone();
            let editor = cx.new(|cx| EditorView::from_document(path.clone(), document, lsp, cx));
            if let Some(symbols) = &self._runtime_symbols {
                editor.update(cx, |editor, _| editor.set_runtime_symbols(symbols.clone()));
            }
            if let Some(index) = &self.project_index {
                editor.update(cx, |editor, _| editor.set_project_symbols(index.clone()));
            }
            cx.observe(&editor, |_, _, cx| cx.notify()).detach();
            self.tabs.push(OpenTab { path, editor });
            self.tabs.len() - 1
        };
        self.active = Some(index);
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
                editor.update(cx, |editor, _| editor.set_project_symbols(index.clone()));
            }
            cx.observe(&editor, |_, _, cx| cx.notify()).detach();
            self.tabs.push(OpenTab { path, editor });
            self.tabs.len() - 1
        };
        self.active = Some(index);
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
        div()
            .w(m.sidebar_default_width)
            .min_w(px(180.))
            .h_full()
            .flex()
            .flex_col()
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
                    .overflow_y_scroll()
                    .children(self.explorer.iter().enumerate().map(|(index, item)| {
                        let item = item.clone();
                        let workspace = workspace.clone();
                        let context_workspace = workspace.clone();
                        let context_item = item.clone();
                        let is_expanded = self.expanded.contains(&item.path);
                        let is_active = active_path.as_ref() == Some(&item.path);
                        let icon =
                            file_icon(&item.path, item.kind == EntryKind::Directory, is_expanded)
                                .glyph();
                        div()
                            .id(("explorer", index))
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
                    })),
            )
    }

    fn render_explorer_context(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let context = self
            .explorer_context
            .clone()
            .expect("context menu is visible");
        let root = self
            .project
            .as_ref()
            .expect("project is open")
            .root_path()
            .to_path_buf();
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

    fn render_explorer_operation(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let selection_start = self
            .explorer_selection
            .range
            .start
            .min(self.explorer_selection.range.end);
        let selection_end = self
            .explorer_selection
            .range
            .start
            .max(self.explorer_selection.range.end);
        let caret = self
            .explorer_selection
            .range
            .end
            .min(self.explorer_input.encode_utf16().count());
        let caret_x: f32 = modal_text_line(window, &self.explorer_input)
            .x_for_index(utf16_to_byte_offset(&self.explorer_input, caret))
            .into();
        let input_before = utf16_slice(&self.explorer_input, 0, selection_start);
        let input_selected = utf16_slice(&self.explorer_input, selection_start, selection_end);
        let input_after = utf16_slice(
            &self.explorer_input,
            selection_end,
            self.explorer_input.encode_utf16().count(),
        );
        let input_width =
            ((self.explorer_input.encode_utf16().count() as f32) * 8.0 + 24.0).clamp(140.0, 300.0);
        let title = match self.explorer_operation {
            Some(ExplorerOperation::Rename(_)) => "Rename",
            Some(ExplorerOperation::NewDirectory(_)) => "New Directory",
            Some(ExplorerOperation::NewFile(_)) => "File",
            Some(ExplorerOperation::NewPhpFile(_)) => "PHP File",
            Some(ExplorerOperation::NewPhp { keyword, .. }) => match keyword {
                "class" => "Create New PHP Class",
                "interface" => "Create New PHP Interface",
                "trait" => "Create New PHP Trait",
                "enum" => "Create New PHP Enum",
                _ => "Create New PHP Item",
            },
            None => "",
        };
        div()
            .absolute()
            .top(px(110.))
            .left(px(320.))
            .w(px(320.))
            .p_3()
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
            .child(title)
            .when(self.explorer_operation.as_ref().is_some_and(|operation| matches!(operation, ExplorerOperation::NewPhp { .. })), |this| {
                this.child(format!("Directory: {}", self.operation_directory().display()))
                    .child(format!("Namespace: {}", if self.explorer_namespace.is_empty() { "(none)" } else { &self.explorer_namespace }))
                    .child("Extends / Implements: optional text fields supported by the generated template")
            })
            .child(
                div()
                    .h(px(34.))
                    .w(px(input_width))
                    .px_2()
                    .flex()
                    .items_center()
                    .bg(t.panel_background)
                    .cursor(CursorStyle::IBeam)
                    .track_focus(&self.modal_input_focus)
                    .id("explorer-operation-input")
                    .on_mouse_down(MouseButton::Left, {
                        let workspace = workspace.clone();
                        move |event, window, cx| {
                            cx.stop_propagation();
                            workspace.update(cx, |this, cx| {
                                let before = this.explorer_selection.range.clone();
                                window.focus(&this.modal_input_focus);
                                let absolute_x: f32 = event.position.x.into();
                                // The input is inside the fixed modal at
                                // left=320, border=1, padding=12+8. Keep this
                                // origin in one place for mouse and caret
                                // diagnostics; the text hit-test itself uses
                                // GPUI's shaped line metrics.
                                let text_origin_x = 341.0;
                                let local_text_x = absolute_x - text_origin_x;
                                let (byte_index, utf16_index) =
                                    modal_hit_test(window, &this.explorer_input, local_text_x);
                                this.explorer_selection = UTF16Selection {
                                    range: utf16_index..utf16_index,
                                    reversed: false,
                                };
                                if debug_input_enabled() {
                                    let text_width: f32 = modal_text_width(window, &this.explorer_input).into();
                                    let input_selection = this
                                        .selected_text_range(false, window, cx)
                                        .map(|selection| selection.range);
                                    tracing::info!(
                                        local_x = local_text_x,
                                        absolute_x,
                                        text_origin_x,
                                        text_width,
                                        "[RENAME INPUT CLICK]"
                                    );
                                    tracing::info!(byte_index, utf16_index, "[RENAME HIT TEST]");
                                    tracing::info!(before = ?before, after = ?this.explorer_selection.range, "[RENAME SELECTION]");
                                    tracing::info!(
                                        selected_range = ?input_selection,
                                        "[INPUT HANDLER SELECTION]"
                                    );
                                }
                                cx.notify();
                            });
                        }
                    })
                    .on_click({
                        let workspace = workspace.clone();
                        move |_, window, cx| {
                            workspace.update(cx, |this, cx| {
                                window.focus(&this.modal_input_focus);
                                if debug_input_enabled() {
                                    tracing::info!("[MODAL INPUT MOUSE DOWN]");
                                }
                                cx.notify();
                            });
                        }
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .h_full()
                            .font_family("Cascadia Mono")
                            .child(
                                div()
                                    .flex()
                                    .h_full()
                                    .items_center()
                                    .child(input_before)
                                    .child(
                                        div()
                                            .when(!input_selected.is_empty(), |this| this.bg(t.selection))
                                            .child(input_selected),
                                    )
                                    .child(input_after),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(px(caret_x))
                                    .top(px(4.))
                                    .w(px(1.))
                                    .h(px(25.))
                                    .bg(t.text_primary),
                            ),
                    )
                    .child(WorkspaceInputElement {
                        workspace: workspace.clone(),
                        focus: self.modal_input_focus.clone(),
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
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
                            .bg(t.accent)
                            .cursor(CursorStyle::PointingHand)
                            .text_color(t.window_background)
                            .on_click(move |_, _, cx| {
                                workspace
                                    .update(cx, |this, cx| this.confirm_explorer_operation(cx));
                            })
                            .child("Confirm"),
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

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                window.focus(&editor.read(cx).focus_handle(cx));
            }
            self.focus_active_editor = false;
        }
        if self.explorer_operation.is_some() && self.modal_focus_pending {
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
            .on_action(cx.listener(Self::navigate_back))
            .on_action(cx.listener(Self::navigate_forward))
            .on_action(cx.listener(Self::go_to_class))
            .on_action(cx.listener(Self::go_to_symbol))
            .on_action(cx.listener(Self::native_definition_action))
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
                    .flex_1()
                    .flex()
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
                        "PHP  ·  Intelephense: {lsp_status}  ·  Runtime: {runtime_stub_status}  ·  UTF-8"
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
            &self.explorer_input
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
            return Some(UTF16Selection {
                range: self.explorer_selection.range.clone(),
                reversed: self.explorer_selection.reversed,
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editing_explorer = self.explorer_operation.is_some();
        let editing_rename = self
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| matches!(operation, ExplorerOperation::Rename(_)));
        let editing_settings =
            self.settings_visible && !self.command_palette_visible && !editing_explorer;
        let query = if editing_explorer {
            self.explorer_input.clone()
        } else if editing_settings {
            self.settings_query.clone()
        } else {
            self.command_palette_query.clone()
        };
        let range = range.unwrap_or_else(|| {
            if editing_explorer {
                let start = self
                    .explorer_selection
                    .range
                    .start
                    .min(self.explorer_selection.range.end);
                let end = self
                    .explorer_selection
                    .range
                    .start
                    .max(self.explorer_selection.range.end);
                start..end
            } else {
                let end = query.encode_utf16().count();
                end..end
            }
        });
        let before_len = query.encode_utf16().count();
        if editing_rename {
            self.explorer_undo.push((
                self.explorer_input.clone(),
                UTF16Selection {
                    range: self.explorer_selection.range.clone(),
                    reversed: self.explorer_selection.reversed,
                },
            ));
        }
        let (query, caret) = replace_utf16_range(&query, range.clone(), text);
        if editing_explorer {
            self.explorer_input = query;
            let length = self.explorer_input.encode_utf16().count();
            self.explorer_selection = UTF16Selection {
                range: caret..caret,
                reversed: false,
            };
            if debug_input_enabled() {
                tracing::info!(
                    active = self.modal_input_focus.is_focused(window),
                    "[MODAL INPUT HANDLER]"
                );
                tracing::info!(
                    kind = if editing_rename { "rename" } else { "explorer" },
                    range_start = range.start,
                    range_end = range.end,
                    inserted_len = text.encode_utf16().count(),
                    "[MODAL REPLACE TEXT]"
                );
                tracing::info!(
                    value_len_before = before_len,
                    value_len_after = length,
                    changed = true,
                    "[MODAL STATE]"
                );
                tracing::info!(
                    selection_after = ?self.explorer_selection.range,
                    "[RENAME SELECTION AFTER EDIT]"
                );
                if editing_rename {
                    tracing::info!(old_len = before_len, new_len = length, "[RENAME STATE]");
                }
                tracing::info!(notify = true, "[MODAL NOTIFY]");
                if text.is_empty() {
                    tracing::info!(
                        range_start = range.start,
                        range_end = range.end,
                        "[MODAL DELETE]"
                    );
                }
            }
        } else if editing_settings {
            self.settings_query = query;
        } else {
            self.command_palette_query = query;
        }
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
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(if self.explorer_operation.is_some() {
            let x: f32 = point.x.into();
            let (_, utf16_index) = modal_hit_test(window, &self.explorer_input, x);
            utf16_index
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

fn modal_text_line(window: &mut Window, text: &str) -> gpui::ShapedLine {
    let text: SharedString = text.to_owned().into();
    let run = TextRun {
        len: text.len(),
        font: font("Cascadia Mono"),
        color: window.text_style().color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(text, px(14.), &[run], None)
}

fn modal_text_width(window: &mut Window, text: &str) -> Pixels {
    modal_text_line(window, text).width
}

fn modal_hit_test(window: &mut Window, text: &str, local_text_x: f32) -> (usize, usize) {
    let shaped = modal_text_line(window, text);
    let byte_index = shaped
        .closest_index_for_x(px(local_text_x.max(0.0)))
        .min(text.len());
    (byte_index, byte_to_utf16_offset(text, byte_index))
}

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

fn debug_keys_enabled() -> bool {
    std::env::var_os("AXIOM_DEBUG_KEYS").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    })
}

fn debug_input_enabled() -> bool {
    std::env::var_os("AXIOM_DEBUG_INPUT").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    })
}

fn normalize_modifiers(modifiers: Modifiers) -> (bool, bool, bool) {
    (modifiers.control, modifiers.shift, modifiers.alt)
}

#[cfg(test)]
mod modifier_tests {
    use super::{
        byte_to_utf16_offset, normalize_modifiers, replace_utf16_range, utf16_slice,
        utf16_to_byte_offset,
    };
    use gpui::Modifiers;

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
