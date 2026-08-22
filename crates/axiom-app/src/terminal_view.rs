use std::{ops::Range, sync::Arc, time::Duration};

use axiom_terminal::{TerminalLink, TerminalSession, detect_links};
use gpui::{
    App, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Focusable, GlobalElementId, IntoElement, KeyBinding, LayoutId, Pixels, Point,
    Render, SharedString, Style, UTF16Selection, WeakEntity, Window, actions, div, prelude::*,
    relative,
};

use crate::ui::{metrics, theme};
use crate::workspace_view::WorkspaceView;

actions!(
    terminal,
    [
        Enter, Backspace, Tab, Up, Down, Left, Right, Interrupt, Eof, Paste, Escape,
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", Enter, Some("Terminal")),
        KeyBinding::new("backspace", Backspace, Some("Terminal")),
        KeyBinding::new("tab", Tab, Some("Terminal")),
        KeyBinding::new("up", Up, Some("Terminal")),
        KeyBinding::new("down", Down, Some("Terminal")),
        KeyBinding::new("left", Left, Some("Terminal")),
        KeyBinding::new("right", Right, Some("Terminal")),
        KeyBinding::new("ctrl-c", Interrupt, Some("Terminal")),
        KeyBinding::new("ctrl-d", Eof, Some("Terminal")),
        KeyBinding::new("ctrl-shift-v", Paste, Some("Terminal")),
        KeyBinding::new("escape", Escape, Some("Terminal")),
    ]
}

pub struct TerminalView {
    session: Arc<TerminalSession>,
    focus: FocusHandle,
    contents: SharedString,
    revision: u64,
    marked_text: Option<String>,
    line_links: Vec<Vec<TerminalLink>>,
    hovered_link: Option<TerminalLink>,
    workspace: WeakEntity<WorkspaceView>,
    context_menu: Option<TerminalLink>,
    context_menu_open: bool,
    context_position: Point<Pixels>,
    select_all: bool,
}

impl TerminalView {
    pub fn new(
        session: Arc<TerminalSession>,
        workspace: WeakEntity<WorkspaceView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            session,
            focus: cx.focus_handle(),
            contents: "Starting terminal…".into(),
            revision: 0,
            marked_text: None,
            line_links: Vec::new(),
            hovered_link: None,
            workspace,
            context_menu: None,
            context_menu_open: false,
            context_position: Point::default(),
            select_all: false,
        };
        view.refresh();
        cx.spawn(async move |this, cx| {
            loop {
                gpui::Timer::after(Duration::from_millis(33)).await;
                if this
                    .update(cx, |this, cx| {
                        if this.refresh() {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        view
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn refresh(&mut self) -> bool {
        let revision = self.session.revision();
        if revision == self.revision {
            return false;
        }
        self.revision = revision;
        self.contents = self.session.contents().into();
        let cwd = self.session.cwd().to_path_buf();
        self.line_links = self
            .contents
            .lines()
            .map(|line| detect_links(line, &cwd))
            .collect();
        true
    }

    fn link_hover(
        &mut self,
        link: TerminalLink,
        event: &gpui::MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.control {
            if self.hovered_link.as_ref() != Some(&link) {
                self.hovered_link = Some(link);
                cx.notify();
            }
        } else if self.hovered_link.take().is_some() {
            cx.notify();
        }
    }

    fn link_click(
        &mut self,
        link: TerminalLink,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.control {
            let workspace = self.workspace.clone();
            cx.defer_in(window, move |_, window, cx| {
                if let Some(workspace) = workspace.upgrade() {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_terminal_link(link, window, cx)
                    });
                }
            });
        }
    }

    fn show_context_menu(
        &mut self,
        link: Option<TerminalLink>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = link;
        self.context_menu_open = true;
        self.context_position = position;
        cx.notify();
    }

    fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        if self.context_menu_open {
            self.context_menu = None;
            self.context_menu_open = false;
            cx.notify();
        }
    }

    fn select_all_output(&mut self, cx: &mut Context<Self>) {
        self.select_all = true;
        self.context_menu = None;
        self.context_menu_open = false;
        cx.notify();
    }

    fn copy_output(&mut self, cx: &mut Context<Self>) {
        if self.select_all {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.contents.to_string()));
        }
        self.context_menu = None;
        self.context_menu_open = false;
        cx.notify();
    }

    fn clear_output(&mut self, cx: &mut Context<Self>) {
        self.session.clear_screen();
        self.context_menu = None;
        self.context_menu_open = false;
        self.select_all = false;
        self.refresh();
        cx.notify();
    }

    fn paste_output(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.write(text.as_bytes());
        }
        self.context_menu = None;
        self.context_menu_open = false;
        cx.notify();
    }

    fn open_context_link(
        &mut self,
        link: TerminalLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = None;
        self.context_menu_open = false;
        let workspace = self.workspace.clone();
        cx.defer_in(window, move |_, window, cx| {
            if let Some(workspace) = workspace.upgrade() {
                workspace.update(cx, |workspace, cx| {
                    workspace.open_terminal_link(link, window, cx)
                });
            }
        });
    }

    fn render_line(
        &self,
        line: &str,
        links: &[TerminalLink],
        row: usize,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut children: Vec<gpui::AnyElement> = Vec::new();
        let mut cursor = 0usize;
        for link in links {
            if link.range.start > cursor {
                children.push(
                    div()
                        .child(line[cursor..link.range.start].to_owned())
                        .into_any_element(),
                );
            }
            let active = self.hovered_link.as_ref() == Some(link);
            let terminal = cx.entity();
            let terminal_click = terminal.clone();
            let terminal_context = terminal.clone();
            children.push(
                div()
                    .id(ElementId::Name(
                        format!("terminal-link-{row}-{}", link.range.start).into(),
                    ))
                    .text_color(if active {
                        theme().accent_hover
                    } else {
                        theme().text_primary
                    })
                    .when(active, |this| this.underline())
                    .when(active, |this| this.cursor(CursorStyle::PointingHand))
                    .on_mouse_move({
                        let link = link.clone();
                        move |event, _, cx| {
                            terminal.update(cx, |this, cx| this.link_hover(link.clone(), event, cx))
                        }
                    })
                    .on_mouse_down(gpui::MouseButton::Left, {
                        let link = link.clone();
                        move |event, window, cx| {
                            terminal_click.update(cx, |this, cx| {
                                this.link_click(link.clone(), event, window, cx)
                            })
                        }
                    })
                    .on_mouse_down(gpui::MouseButton::Right, {
                        let link = link.clone();
                        move |event, _, cx| {
                            terminal_context.update(cx, |this, cx| {
                                this.show_context_menu(Some(link.clone()), event.position, cx)
                            })
                        }
                    })
                    .child(line[link.range.clone()].to_owned())
                    .into_any_element(),
            );
            cursor = link.range.end;
        }
        if cursor < line.len() {
            children.push(div().child(line[cursor..].to_owned()).into_any_element());
        }
        div().flex().children(children)
    }

    fn write(&self, bytes: &[u8]) {
        if let Err(error) = self.session.write(bytes) {
            tracing::warn!("terminal input failed: {error}");
        }
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, _: &mut Context<Self>) {
        self.write(b"\r");
    }
    fn backspace(&mut self, _: &Backspace, _: &mut Window, _: &mut Context<Self>) {
        self.write(b"\x7f");
    }
    fn tab(&mut self, _: &Tab, _: &mut Window, _: &mut Context<Self>) {
        self.write(b"\t");
    }
    fn up(&mut self, _: &Up, _: &mut Window, _: &mut Context<Self>) {
        self.write(b"\x1b[A");
    }
    fn down(&mut self, _: &Down, _: &mut Window, _: &mut Context<Self>) {
        self.write(b"\x1b[B");
    }
    fn left(&mut self, _: &Left, _: &mut Window, _: &mut Context<Self>) {
        self.write(b"\x1b[D");
    }
    fn right(&mut self, _: &Right, _: &mut Window, _: &mut Context<Self>) {
        self.write(b"\x1b[C");
    }
    fn interrupt(&mut self, _: &Interrupt, _: &mut Window, _: &mut Context<Self>) {
        self.write(&[0x03]);
    }
    fn eof(&mut self, _: &Eof, _: &mut Window, _: &mut Context<Self>) {
        self.write(&[0x04]);
    }
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.write(text.as_bytes());
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        div()
            .relative()
            .size_full()
            .key_context("Terminal")
            .track_focus(&self.focus)
            .bg(t.editor_background)
            .text_color(t.text_primary)
            .font_family("Cascadia Mono")
            .text_size(m.editor_font_size)
            .line_height(m.editor_line_height)
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::interrupt))
            .on_action(cx.listener(Self::eof))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::escape))
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    this.show_context_menu(None, event.position, cx)
                }),
            )
            .child(div().absolute().size_full().child(TerminalInputElement {
                terminal: cx.entity(),
            }))
            .child(
                div()
                    .id("terminal-output")
                    .absolute()
                    .size_full()
                    .p_2()
                    .overflow_y_scroll()
                    .children(self.contents.lines().enumerate().map(|(row, line)| {
                        self.render_line(
                            line,
                            self.line_links.get(row).map(Vec::as_slice).unwrap_or(&[]),
                            row,
                            cx,
                        )
                    })),
            )
            .child(
                div()
                    .absolute()
                    .left(self.context_position.x)
                    .top(self.context_position.y)
                    .when_some(self.context_menu.clone(), |menu, link| {
                        menu.child(
                            div()
                                .id("terminal-open-link")
                                .px_2()
                                .py_1()
                                .bg(t.popup_background)
                                .text_color(t.text_primary)
                                .child(
                                    if matches!(link.kind, axiom_terminal::TerminalLinkKind::Url) {
                                        "Open Link"
                                    } else {
                                        "Open File"
                                    },
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(link) = this.context_menu.clone() {
                                        this.open_context_link(link, window, cx);
                                    }
                                })),
                        )
                    })
                    .when(self.context_menu_open, |menu| {
                        menu.child(
                            div()
                                .id("terminal-copy")
                                .px_2()
                                .py_1()
                                .text_color(if self.select_all {
                                    t.text_primary
                                } else {
                                    t.text_muted
                                })
                                .child("Copy")
                                .on_click(cx.listener(|this, _, _, cx| this.copy_output(cx))),
                        )
                        .child(
                            div()
                                .id("terminal-paste")
                                .px_2()
                                .py_1()
                                .child("Paste")
                                .on_click(cx.listener(|this, _, _, cx| this.paste_output(cx))),
                        )
                        .child(
                            div()
                                .id("terminal-select-all")
                                .px_2()
                                .py_1()
                                .child("Select All")
                                .on_click(cx.listener(|this, _, _, cx| this.select_all_output(cx))),
                        )
                        .child(
                            div()
                                .id("terminal-clear")
                                .px_2()
                                .py_1()
                                .child("Clear Terminal")
                                .on_click(cx.listener(|this, _, _, cx| this.clear_output(cx))),
                        )
                    }),
            )
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        actual.replace(0..0);
        Some(String::new())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text.as_ref().map(|text| 0..text.len())
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_text = None;
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.marked_text = None;
        self.write(text.as_bytes());
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.marked_text = Some(text.to_owned());
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
        Some(0)
    }
}

struct TerminalInputElement {
    terminal: Entity<TerminalView>,
}

impl IntoElement for TerminalInputElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalInputElement {
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
        let focus = {
            let terminal = self.terminal.read(cx);
            let rows =
                (f32::from(bounds.size.height) / f32::from(metrics().editor_line_height)) as u16;
            let cols = (f32::from(bounds.size.width) / 8.4) as u16;
            if let Err(error) = terminal.session.resize(rows, cols) {
                tracing::warn!("terminal resize failed: {error}");
            }
            terminal.focus.clone()
        };
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.terminal.clone()),
            cx,
        );
    }
}
