use std::sync::{Arc, Mutex};

use axiom_terminal::{CellFlags, CursorShape, NamedColor, Rgb, TerminalCell, TerminalColor as Color, TerminalScrollState, TerminalSession, TerminalSize, encode_key};
use gpui::{AsyncApp, ClipboardItem, Context, FocusHandle, Focusable, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Point, Pixels, Render, ScrollDelta, ScrollWheelEvent, Timer, Window, background_executor, div, prelude::*, px, rgb, FontWeight};
use std::time::Duration;
use crate::ui::metrics::{CODE_FONT_FAMILY, code_font, metrics};

const DEFAULT_FOREGROUND: u32 = 0xd4d4d4;
const DEFAULT_BACKGROUND: u32 = 0x1e1e1e;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CellStyle { foreground: u32, background: u32, bold: bool, italic: bool, underline: bool }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct TerminalPoint { row: usize, column: usize }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectionRange { start: TerminalPoint, end: TerminalPoint }

#[derive(Clone, Copy, Debug, PartialEq)]
struct CursorRect { x: Pixels, y: Pixels, width: Pixels, height: Pixels }

fn scrollbar_thumb(track_height: Pixels, state: TerminalScrollState) -> Option<(Pixels, Pixels)> {
    if state.max_offset == 0 { return None; }
    let minimum = px(20.).min(track_height);
    let height = (track_height * state.visible_rows as f32 / (state.visible_rows + state.max_offset) as f32).max(minimum).min(track_height);
    let travel = track_height - height;
    let progress = 1. - (state.display_offset as f32 / state.max_offset as f32).clamp(0., 1.);
    Some((travel * progress, height))
}

fn scrollbar_offset_from_top(top: Pixels, track_height: Pixels, thumb_height: Pixels, max_offset: usize) -> usize {
    let travel = (track_height - thumb_height).max(px(0.));
    if travel <= px(0.) { return 0; }
    ((1. - ((top / travel).clamp(0., 1.))) * max_offset as f32).round() as usize
}

fn first_visible_row(state: TerminalScrollState) -> usize { state.max_offset.saturating_sub(state.display_offset) }
fn logical_row(state: TerminalScrollState, viewport_row: usize) -> usize { first_visible_row(state).saturating_add(viewport_row) }
fn selection_viewport_row(raw_row: i32, visible_rows: usize) -> (usize, i8) {
    if visible_rows == 0 { return (0, 0); }
    if raw_row < 0 { (0, -1) }
    else if raw_row as usize >= visible_rows { (visible_rows - 1, 1) }
    else { (raw_row as usize, 0) }
}

fn cursor_rect(row: usize, column: usize, shape: CursorShape, cell_width: Pixels, cell_height: Pixels, content_origin: Point<Pixels>, glyph_height: Pixels) -> Option<CursorRect> {
    let x = content_origin.x + cell_width * column as f32;
    let cell_y = content_origin.y + cell_height * row as f32;
    match shape {
        CursorShape::Hidden => None,
        CursorShape::Beam => {
            let height = glyph_height.min(cell_height);
            Some(CursorRect { x, y: cell_y + (cell_height - height) / 2., width: px(1.).min(cell_width), height })
        }
        CursorShape::Underline => {
            let height = px(2.).min(cell_height);
            Some(CursorRect { x, y: cell_y + cell_height - height, width: cell_width, height })
        }
        _ => Some(CursorRect { x, y: cell_y, width: cell_width, height: cell_height }),
    }
}

fn normalize_selection(anchor: TerminalPoint, head: TerminalPoint) -> SelectionRange {
    if anchor <= head { SelectionRange { start: anchor, end: head } } else { SelectionRange { start: head, end: anchor } }
}

fn selected(range: SelectionRange, point: TerminalPoint) -> bool {
    if point.row < range.start.row || point.row > range.end.row { return false; }
    if range.start.row == range.end.row { return point.column >= range.start.column && point.column <= range.end.column; }
    if point.row == range.start.row { return point.column >= range.start.column; }
    if point.row == range.end.row { return point.column <= range.end.column; }
    true
}

fn selected_text(cells: &[Vec<TerminalCell>], range: SelectionRange, first_row: usize) -> String {
    let mut output = String::new();
    let selected_rows = cells.iter().enumerate().filter_map(|(row, cells)| {
        let absolute_row = first_row + row;
        if absolute_row < range.start.row || absolute_row > range.end.row { return None; }
        let start = if absolute_row == range.start.row { range.start.column } else { 0 };
        let end = if absolute_row == range.end.row { range.end.column } else { cells.len().saturating_sub(1) };
        let line: String = cells.get(start..=end).unwrap_or(&[]).iter().map(|cell| cell.character).collect();
        Some(line.trim_end_matches(' ').to_owned())
    }).collect::<Vec<_>>();
    for (index, line) in selected_rows.iter().enumerate() {
        output.push_str(line.trim_end_matches(' '));
        if index + 1 < selected_rows.len() { output.push_str("\r\n"); }
    }
    output
}

fn ansi_color(index: u8) -> u32 {
    const COLORS: [u32; 16] = [0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666, 0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xe5e5e5];
    match index {
        0..=15 => COLORS[index as usize],
        16..=231 => {
            let value = index - 16;
            let channel = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            (channel(value / 36) as u32) << 16 | (channel((value / 6) % 6) as u32) << 8 | channel(value % 6) as u32
        }
        232..=255 => { let value = 8 + (index - 232) * 10; u32::from(value) << 16 | u32::from(value) << 8 | u32::from(value) }
    }
}

fn color_to_rgb(color: Color) -> u32 {
    match color {
        Color::Spec(Rgb { r, g, b }) => u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
        Color::Indexed(index) => ansi_color(index),
        Color::Named(name) => match name {
            NamedColor::Foreground | NamedColor::BrightForeground => DEFAULT_FOREGROUND,
            NamedColor::Background => DEFAULT_BACKGROUND,
            NamedColor::Cursor => 0xffffff,
            NamedColor::DimForeground => 0x888888,
            NamedColor::DimBlack => 0x000000,
            NamedColor::DimRed => 0x892020,
            NamedColor::DimGreen => 0x08784f,
            NamedColor::DimYellow => 0x909008,
            NamedColor::DimBlue => 0x174a82,
            NamedColor::DimMagenta => 0x782878,
            NamedColor::DimCyan => 0x0b6c80,
            NamedColor::DimWhite => 0x909090,
            _ => ansi_color(name as u8),
        },
    }
}

fn cell_style(cell: &TerminalCell) -> CellStyle {
    let inverse = cell.flags.contains(CellFlags::INVERSE);
    let (mut foreground, mut background) = (color_to_rgb(cell.foreground), color_to_rgb(cell.background));
    if inverse { std::mem::swap(&mut foreground, &mut background); }
    if cell.flags.contains(CellFlags::DIM) { foreground = dim_color(foreground); }
    CellStyle { foreground, background, bold: cell.flags.contains(CellFlags::BOLD), italic: cell.flags.contains(CellFlags::ITALIC), underline: cell.flags.contains(CellFlags::UNDERLINE) }
}

fn dim_color(color: u32) -> u32 {
    let scale = |value: u32| value * 2 / 3;
    scale(color >> 16) << 16 | scale((color >> 8) & 0xff) << 8 | scale(color & 0xff)
}

fn row_cells(cells: &[TerminalCell], absolute_row: usize, selection: Option<SelectionRange>) -> Vec<(char, CellStyle)> {
    cells.iter().enumerate().map(|(column, cell)| {
        let mut style = cell_style(cell);
        if selection.is_some_and(|range| selected(range, TerminalPoint { row: absolute_row, column })) { style.background = 0x264f78; }
        (cell.character, style)
    }).collect()
}

pub struct TerminalView { session: Arc<Mutex<TerminalSession>>, focus: FocusHandle, viewport_height: Pixels, ready_callback: Option<Arc<dyn Fn(&mut AsyncApp) + Send + Sync>>, first_activity_reported: bool, last_terminal_size: Option<(usize, usize)>, selection_anchor: Option<TerminalPoint>, selection_head: Option<TerminalPoint>, selection_pointer: Option<TerminalPoint>, selection_pointer_y: Option<Pixels>, selection_edge: i8, is_selecting: bool, selection_dragged: bool, context_menu: Option<Point<Pixels>>, scrollbar_dragging: bool, scrollbar_grab_offset_y: Pixels, scrollbar_drag_thumb_y: Option<Pixels>, scrollbar_last_target: Option<usize>, scroll_remainder: f32, cursor_blink_visible: bool, terminal_focused: bool }
impl TerminalView {
    pub fn new(session: Arc<Mutex<TerminalSession>>, viewport_height: Pixels, ready_callback: Option<Arc<dyn Fn(&mut AsyncApp) + Send + Sync>>, cx: &mut Context<Self>) -> Self {
        let view = Self { session: session.clone(), focus: cx.focus_handle(), viewport_height, ready_callback, first_activity_reported: false, last_terminal_size: None, selection_anchor: None, selection_head: None, selection_pointer: None, selection_pointer_y: None, selection_edge: 0, is_selecting: false, selection_dragged: false, context_menu: None, scrollbar_dragging: false, scrollbar_grab_offset_y: px(0.), scrollbar_drag_thumb_y: None, scrollbar_last_target: None, scroll_remainder: 0., cursor_blink_visible: true, terminal_focused: false };
        let events = session.lock().ok().map(|session| session.event_receiver());
        cx.spawn(async move |this, cx| {
            let Some(events) = events else { return; };
            loop {
                let receiver = events.clone();
                let changed = background_executor().spawn(async move { receiver.lock().ok().is_some_and(|events| events.recv().is_ok()) }).await;
                if !changed { break; }
                let callback = this.update(cx, |view, _| {
                    let active = view.session.lock().ok().map(|session| {
                        let snapshot = session.snapshot();
                        snapshot.cells.iter().flatten().any(|cell| !cell.character.is_whitespace()) || !matches!(snapshot.state, axiom_terminal::TerminalState::Running)
                    }).unwrap_or(false);
                    if active && !view.first_activity_reported {
                        view.first_activity_reported = true;
                        view.ready_callback.clone()
                    } else { None }
                });
                if let Ok(Some(callback)) = callback { callback(cx); }
                if this.update(cx, |_, cx| cx.notify()).is_err() { break; }
            }
        }).detach();
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(500)).await;
                if this.update(cx, |view, cx| { if view.terminal_focused { view.cursor_blink_visible = !view.cursor_blink_visible; cx.notify(); } }).is_err() { break; }
            }
        }).detach();
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(120)).await;
                if this.update(cx, |view, cx| view.auto_scroll_selection_edge(cx)).is_err() { break; }
            }
        }).detach();
        view
    }
    pub fn set_viewport_height(&mut self, viewport_height: Pixels) { self.viewport_height = viewport_height; }
    fn prepare_for_terminal_input(&mut self, cx: &mut Context<Self>) {
        self.cursor_blink_visible = true;
        if self.session.lock().ok().is_some_and(|session| session.display_offset() > 0) {
            if let Ok(session) = self.session.lock() { session.scroll_to_bottom(); }
            cx.notify();
        }
    }
    fn selection_range(&self) -> Option<SelectionRange> { self.selection_anchor.zip(self.selection_head).map(|(anchor, head)| normalize_selection(anchor, head)) }
    fn logical_row(&self, viewport_row: usize) -> usize {
        self.session.lock().ok().map(|session| logical_row(session.scroll_state(), viewport_row)).unwrap_or(viewport_row)
    }
    fn update_selection_head_from_pointer(&mut self, cx: &mut Context<Self>) {
        if let Some(pointer) = self.selection_pointer {
            let visible_rows = self.session.lock().ok().map(|session| session.scroll_state().visible_rows).unwrap_or(0);
            let viewport_row = if self.selection_edge < 0 { 0 } else if self.selection_edge > 0 { visible_rows.saturating_sub(1) } else { pointer.row };
            let point = TerminalPoint { row: self.logical_row(viewport_row), column: pointer.column };
            self.selection_dragged |= self.selection_anchor.is_some_and(|anchor| point != anchor);
            self.selection_head = Some(point);
            cx.notify();
        }
    }
    fn scroll_selection(&mut self, lines: i32, cx: &mut Context<Self>) {
        if let Ok(session) = self.session.lock() { session.scroll_lines(lines); }
        self.update_selection_head_from_pointer(cx);
    }
    fn auto_scroll_selection_edge(&mut self, cx: &mut Context<Self>) {
        if !self.is_selecting { return; }
        if self.selection_edge < 0 {
            self.scroll_selection(1, cx);
        } else if self.selection_edge > 0 {
            self.scroll_selection(-1, cx);
        }
    }
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, point: TerminalPoint, cx: &mut Context<Self>) {
        if event.button == MouseButton::Right {
            self.context_menu = Some(event.position);
            window.focus(&self.focus);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if event.button != MouseButton::Left { return; }
        let viewport_point = point;
        let point = TerminalPoint { row: self.logical_row(viewport_point.row), column: viewport_point.column };
        self.selection_anchor = Some(point); self.selection_head = Some(point); self.selection_pointer = Some(viewport_point); self.selection_pointer_y = Some(event.position.y); self.selection_edge = 0; self.is_selecting = true; self.selection_dragged = false;
        window.focus(&self.focus); cx.notify();
    }
    fn mouse_move_cell(&mut self, event: &MouseMoveEvent, point: TerminalPoint, cx: &mut Context<Self>) {
        if !self.is_selecting || !event.dragging() { return; }
        self.selection_pointer = Some(point);
        self.selection_pointer_y = Some(event.position.y);
        self.selection_edge = 0;
        self.update_selection_head_from_pointer(cx);
    }
    fn scrollbar_mouse_down(&mut self, event: &MouseDownEvent, thumb_top: Pixels, cx: &mut Context<Self>) { if event.button == MouseButton::Left { self.scrollbar_dragging = true; self.scrollbar_grab_offset_y = event.position.y - thumb_top; self.scrollbar_drag_thumb_y = Some(thumb_top); self.scrollbar_last_target = None; self.is_selecting = false; cx.stop_propagation(); } }
    fn mouse_move(&mut self, event: &MouseMoveEvent, track_height: Pixels, thumb_height: Pixels, cell_height: Pixels, visible_rows: usize, cx: &mut Context<Self>) {
        if self.is_selecting && event.dragging() && cell_height > px(0.) {
            if let (Some(pointer), Some(pointer_y)) = (self.selection_pointer, self.selection_pointer_y) {
                let row_delta = ((event.position.y - pointer_y) / cell_height).floor() as i32;
                let (row, edge) = selection_viewport_row(pointer.row as i32 + row_delta, visible_rows);
                self.selection_pointer = Some(TerminalPoint { row, column: pointer.column });
                self.selection_pointer_y = Some(event.position.y);
                self.selection_edge = edge;
                self.update_selection_head_from_pointer(cx);
            }
        }
        if self.scrollbar_dragging && event.dragging() {
            let top = (event.position.y - self.scrollbar_grab_offset_y).clamp(px(0.), (track_height - thumb_height).max(px(0.)));
            let thumb_moved = self.scrollbar_drag_thumb_y != Some(top);
            self.scrollbar_drag_thumb_y = Some(top);
            let target = self.session.lock().ok().map(|session| { let state = session.scroll_state(); (state.display_offset, scrollbar_offset_from_top(top, track_height, thumb_height, state.max_offset)) });
            if let Some((current, target)) = target && self.scrollbar_last_target != Some(target) { self.scrollbar_last_target = Some(target); if current != target { if let Ok(session) = self.session.lock() { session.scroll_to_offset(target); } } }
            if thumb_moved { cx.notify(); }
            cx.stop_propagation();
        }
    }
    fn mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) { if event.button == MouseButton::Left { self.scrollbar_dragging = false; self.scrollbar_drag_thumb_y = None; self.scrollbar_last_target = None; self.is_selecting = false; self.selection_edge = 0; self.selection_pointer_y = None; if !self.selection_dragged { self.selection_anchor = None; self.selection_head = None; } self.selection_dragged = false; cx.notify(); } }
    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cell_height: Pixels, cx: &mut Context<Self>) {
        let delta = match event.delta { ScrollDelta::Lines(delta) => delta.y, ScrollDelta::Pixels(delta) => delta.y / cell_height };
        self.scroll_remainder += delta;
        let lines = if self.scroll_remainder >= 0. { self.scroll_remainder.floor() as i32 } else { self.scroll_remainder.ceil() as i32 };
        if lines == 0 { return; }
        self.scroll_remainder -= lines as f32;
        if self.is_selecting { self.scroll_selection(lines, cx); }
        else if let Ok(session) = self.session.lock() { session.scroll_lines(lines); }
        cx.notify();
        cx.stop_propagation();
    }
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke.key;
        let ctrl = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        let selection = self.selection_range();
        let has_selection = selection.is_some_and(|range| range.start != range.end);
        if ctrl && key.eq_ignore_ascii_case("c") && (shift || has_selection) {
            if let (Some(range), Ok(session)) = (selection, self.session.lock()) {
                let snapshot = session.snapshot();
                let scroll = session.scroll_state();
                let first_row = scroll.max_offset.saturating_sub(snapshot.display_offset);
                let text = selected_text(&snapshot.cells, range, first_row);
                if !text.is_empty() { cx.write_to_clipboard(ClipboardItem::new_string(text)); }
            }
            cx.stop_propagation();
            return;
        }
        if ctrl && key.eq_ignore_ascii_case("v") {
            self.prepare_for_terminal_input(cx);
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                if !text.is_empty() { if let Ok(session) = self.session.lock() { let _ = session.paste(&text); } }
            }
            cx.stop_propagation();
            return;
        }
        let bytes = event.keystroke.key_char.as_deref().filter(|_| !ctrl).map(str::as_bytes).map(|b| b.to_vec());
        if let Some(bytes) = bytes {
            self.prepare_for_terminal_input(cx);
            if let Ok(session) = self.session.lock() { let _ = session.write(&bytes); }
            window.focus(&self.focus); cx.stop_propagation(); return;
        }
        let application_cursor = self.session.lock().ok().map(|session| session.application_cursor_enabled());
        if let Some(Some(bytes)) = application_cursor.map(|application_cursor| encode_key(key, ctrl, application_cursor)) {
            self.prepare_for_terminal_input(cx);
            if let Ok(session) = self.session.lock() { let _ = session.write(&bytes); }
            window.focus(&self.focus); cx.stop_propagation();
        }
    }
    fn copy_selection(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(range) = self.selection_range() && let Ok(session) = self.session.lock() {
            let snapshot = session.snapshot();
            let scroll = session.scroll_state();
            let first_row = scroll.max_offset.saturating_sub(snapshot.display_offset);
            let text = selected_text(&snapshot.cells, range, first_row);
            if !text.is_empty() { cx.write_to_clipboard(ClipboardItem::new_string(text)); }
        }
        self.context_menu = None;
        cx.stop_propagation();
        cx.notify();
    }
}
impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut viewport = window.viewport_size();
        viewport.height = self.viewport_height;
        let font_size = metrics().editor_font_size;
        let font_id = window.text_system().resolve_font(&code_font());
        let (cell_width, cell_height) = window.text_system().advance(font_id, font_size, 'M')
            .ok()
            .zip(Some(window.text_system().bounding_box(font_id, font_size).size.height))
            .filter(|(width, height)| width.width > px(0.) && *height > px(0.))
            .map(|(width, height)| (width.width, height))
            .unwrap_or((px(1.), font_size));
        let glyph_height = window.text_system().bounding_box(font_id, font_size).size.height.min(cell_height);
        let cols = (viewport.width / cell_width).floor().max(1.) as usize;
        let rows = (viewport.height / cell_height).floor().max(1.) as usize;
        if self.last_terminal_size != Some((cols, rows)) {
            if let Ok(mut session) = self.session.lock() { let _ = session.resize(TerminalSize::new(cols as u16, rows as u16)); }
            self.last_terminal_size = Some((cols, rows));
        }
        let snapshot = self.session.lock().ok().map(|session| session.snapshot());
        let scroll_state = self.session.lock().ok().map(|session| session.scroll_state()).unwrap_or(TerminalScrollState { display_offset: 0, max_offset: 0, visible_rows: rows });
        let first_row = first_visible_row(scroll_state);
        let cursor = snapshot.as_ref().map(|snapshot| (snapshot.cursor, snapshot.cursor_visible, snapshot.cursor_shape));
        let selection = self.selection_range();
        let body = snapshot.as_ref().map(|snapshot| div().flex().flex_col().children(snapshot.cells.iter().enumerate().map(|(row_index, row)| div().h(cell_height).flex().children(row_cells(row, first_row + row_index, selection).into_iter().enumerate().map(|(column, (character, style))| {
            let point = TerminalPoint { row: row_index, column };
            div().w(cell_width).h(cell_height).bg(rgb(style.background)).child(character.to_string()).text_color(rgb(style.foreground)).when(style.bold, |this| this.font_weight(FontWeight::BOLD)).when(style.italic, |this| this.italic()).when(style.underline, |this| this.underline()).on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, window, cx| this.mouse_down(event, window, point, cx))).on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| this.mouse_move_cell(event, point, cx)))
        }))))) .unwrap_or_else(|| div().child("Terminal unavailable"));
        self.terminal_focused = self.focus.is_focused(window);
        let cursor = cursor.and_then(|((column, row), visible, shape)| {
            (self.terminal_focused && self.cursor_blink_visible && visible && snapshot.as_ref().is_some_and(|snapshot| snapshot.display_offset == 0)).then_some((column, row, if matches!(shape, CursorShape::Block) { CursorShape::Beam } else { shape }))
        });
        let scrollbar_geometry = scrollbar_thumb(viewport.height, scroll_state);
        let thumb_height = scrollbar_geometry.map(|(_, height)| height).unwrap_or(px(0.));
        let thumb_top = if self.scrollbar_dragging { self.scrollbar_drag_thumb_y.or_else(|| scrollbar_geometry.map(|(top, _)| top)).unwrap_or(px(0.)) } else { scrollbar_geometry.map(|(top, _)| top).unwrap_or(px(0.)) };
        let scrollbar = scrollbar_geometry.map(|(_, height)| div().absolute().right_0().top(thumb_top).w(px(8.)).h(height).bg(rgb(0x5a6575)).opacity(0.8).on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| this.scrollbar_mouse_down(event, thumb_top, cx))));
        div().id("axiom-terminal-view").relative().flex_1().p_2().font_family(CODE_FONT_FAMILY).text_size(font_size).track_focus(&self.focus).on_key_down(cx.listener(Self::key_down)).on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, window, cx| this.mouse_down(event, window, TerminalPoint { row: 0, column: 0 }, cx))).on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| this.mouse_move(event, viewport.height, thumb_height, cell_height, rows, cx))).on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up)).on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up)).on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| this.scroll_wheel(event, cell_height, cx))).child(body).when_some(cursor.and_then(|(column, row, shape)| cursor_rect(row, column, shape, cell_width, cell_height, Point { x: px(8.), y: px(8.) }, glyph_height)), |this, rect| {
            this.child(div().absolute().left(rect.x).top(rect.y).w(rect.width).h(rect.height).bg(rgb(DEFAULT_FOREGROUND)).opacity(0.8))
        }).when_some(scrollbar, |this, scrollbar| this.child(scrollbar)).when_some(self.context_menu, |this, position| {
            this.child(div().absolute().left(position.x).top(position.y).px_2().py_1().bg(rgb(0x252526)).border_1().border_color(rgb(0x454545)).child(div().child("Copy").on_mouse_up(MouseButton::Left, cx.listener(Self::copy_selection))))
        })
    }
}
impl Focusable for TerminalView { fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle { self.focus.clone() } }

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(character: char) -> TerminalCell { TerminalCell { character, foreground: Color::Named(NamedColor::Foreground), background: Color::Named(NamedColor::Background), flags: CellFlags::empty() } }

    #[test]
    fn selections_normalize_in_row_major_order() {
        assert_eq!(normalize_selection(TerminalPoint { row: 5, column: 20 }, TerminalPoint { row: 3, column: 4 }), SelectionRange { start: TerminalPoint { row: 3, column: 4 }, end: TerminalPoint { row: 5, column: 20 } });
        assert!(selected(SelectionRange { start: TerminalPoint { row: 1, column: 2 }, end: TerminalPoint { row: 2, column: 1 } }, TerminalPoint { row: 1, column: 3 }));
    }

    #[test]
    fn multiline_selection_includes_partial_and_intermediate_rows() {
        let range = normalize_selection(TerminalPoint { row: 3, column: 2 }, TerminalPoint { row: 1, column: 4 });
        assert!(!selected(range, TerminalPoint { row: 0, column: 0 }));
        assert!(!selected(range, TerminalPoint { row: 1, column: 3 }));
        assert!(selected(range, TerminalPoint { row: 1, column: 4 }));
        assert!(selected(range, TerminalPoint { row: 2, column: 0 }));
        assert!(selected(range, TerminalPoint { row: 2, column: 99 }));
        assert!(selected(range, TerminalPoint { row: 3, column: 0 }));
        assert!(selected(range, TerminalPoint { row: 3, column: 2 }));
        assert!(!selected(range, TerminalPoint { row: 3, column: 3 }));
    }

    #[test]
    fn scrolling_reanchors_pointer_to_new_visible_terminal_rows() {
        let before = TerminalScrollState { display_offset: 0, max_offset: 100, visible_rows: 24 };
        let after_scrolling_up = TerminalScrollState { display_offset: 4, max_offset: 100, visible_rows: 24 };
        let anchor = TerminalPoint { row: logical_row(before, 5), column: 3 };
        let head = TerminalPoint { row: logical_row(after_scrolling_up, 5), column: 3 };
        assert_eq!(anchor.row, 105);
        assert_eq!(head.row, 101);
        let range = normalize_selection(anchor, head);
        assert!(selected(range, TerminalPoint { row: 103, column: 0 }));
    }

    #[test]
    fn drag_rows_outside_viewport_activate_edge_scroll_only_beyond_bounds() {
        assert_eq!(selection_viewport_row(0, 24), (0, 0));
        assert_eq!(selection_viewport_row(23, 24), (23, 0));
        assert_eq!(selection_viewport_row(-1, 24), (0, -1));
        assert_eq!(selection_viewport_row(24, 24), (23, 1));
        assert_eq!(selection_viewport_row(24, 0), (0, 0));
    }

    #[test]
    fn edge_scroll_keeps_selection_endpoint_on_newly_exposed_edge_row() {
        let scrolled_up = TerminalScrollState { display_offset: 1, max_offset: 100, visible_rows: 24 };
        let scrolled_down = TerminalScrollState { display_offset: 0, max_offset: 100, visible_rows: 24 };
        assert_eq!(logical_row(scrolled_up, 0), 99);
        assert_eq!(logical_row(scrolled_down, 23), 123);
        assert_eq!(selection_viewport_row(-3, 24).0, 0);
        assert_eq!(selection_viewport_row(28, 24).0, 23);
    }

    #[test]
    fn rendered_cell_styles_highlight_every_row_of_multiline_selection() {
        let cells = vec![vec![cell('A'), cell('B'), cell('C')]; 3];
        let range = normalize_selection(TerminalPoint { row: 2, column: 1 }, TerminalPoint { row: 0, column: 1 });
        let first = row_cells(&cells[0], 0, Some(range));
        let middle = row_cells(&cells[1], 1, Some(range));
        let last = row_cells(&cells[2], 2, Some(range));
        assert_eq!(first.iter().map(|(_, style)| style.background).collect::<Vec<_>>(), vec![DEFAULT_BACKGROUND, 0x264f78, 0x264f78]);
        assert!(middle.iter().all(|(_, style)| style.background == 0x264f78));
        assert_eq!(last.iter().map(|(_, style)| style.background).collect::<Vec<_>>(), vec![0x264f78, 0x264f78, DEFAULT_BACKGROUND]);
    }

    #[test]
    fn selected_text_preserves_lines_and_trims_padding() {
        let cells = vec![vec![cell('A'), cell(' '), cell(' ')], vec![cell('B'), cell(' '), cell('C')]];
        assert_eq!(selected_text(&cells, SelectionRange { start: TerminalPoint { row: 0, column: 0 }, end: TerminalPoint { row: 1, column: 2 } }, 0), "A\r\nB C");
    }

    #[test]
    fn cursor_rects_stay_inside_the_target_cell() {
        let origin = Point { x: px(8.), y: px(8.) };
        let block = cursor_rect(2, 3, CursorShape::Block, px(10.), px(20.), origin, px(16.)).unwrap();
        assert_eq!(block, CursorRect { x: px(38.), y: px(48.), width: px(10.), height: px(20.) });
        let beam = cursor_rect(2, 3, CursorShape::Beam, px(10.), px(20.), origin, px(16.)).unwrap();
        assert!(beam.width <= px(10.) && beam.height <= px(20.) && beam.y >= px(48.));
        let underline = cursor_rect(2, 3, CursorShape::Underline, px(10.), px(20.), origin, px(16.)).unwrap();
        assert_eq!(underline.y + underline.height, px(68.));
        assert!(cursor_rect(2, 3, CursorShape::Hidden, px(10.), px(20.), origin, px(16.)).is_none());
    }
}
