//! Geometry shared by the Find and file-name inputs, not the document editor.
use gpui::{
    Bounds, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Pixels, Point,
    ShapedLine, SharedString, TextRun, Window, canvas, fill, point, prelude::*, px, size,
};
use std::{cell::RefCell, ops::Range, rc::Rc};

pub fn blink_due(
    now: std::time::Instant,
    activity: std::time::Instant,
    toggle: std::time::Instant,
) -> bool {
    let interval = std::time::Duration::from_millis(500);
    now.duration_since(activity) >= interval && now.duration_since(toggle) >= interval
}

/// Byte ranges at the input boundary; empty selection must leave the clipboard alone.
pub fn cut_selection(text: &mut String, range: Range<usize>) -> Option<(String, usize)> {
    if range.is_empty() {
        return None;
    }
    let selected = text.get(range.clone())?.to_owned();
    text.replace_range(range.clone(), "");
    Some((selected, range.start))
}

pub fn replace_all_text(text: &str, ranges: &[Range<usize>], replacement: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for range in ranges {
        output.push_str(&text[cursor..range.start]);
        output.push_str(replacement);
        cursor = range.end;
    }
    output.push_str(&text[cursor..]);
    output
}

#[derive(Clone, Default)]
pub struct InputGeometry(
    Rc<RefCell<Option<(ShapedLine, Point<Pixels>)>>>,
    Rc<RefCell<Option<Vec<usize>>>>,
);

/// Domain-neutral state for one independent single-line input.
/// Owners may attach their own FocusHandle; text and selection never share
/// storage with another instance.
#[derive(Clone)]
#[allow(dead_code)]
pub struct SingleLineInputState {
    pub text: String,
    pub selection_anchor: usize,
    pub selection_active: usize,
    pub geometry: InputGeometry,
    pub focus: Option<FocusHandle>,
    pub active: bool,
    pub dragging: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub enum InputVisualMode {
    #[default]
    Plain,
    Masked,
}

/// Builds a display-only representation while preserving UTF-8 byte boundaries.
/// Each scalar is replaced by the same number of one-byte mask glyphs, so the
/// renderer's byte offsets remain valid for the real value.
pub fn visual_text(text: &str, mode: InputVisualMode) -> String {
    match mode {
        InputVisualMode::Plain => text.to_owned(),
        InputVisualMode::Masked => text
            .chars()
            .flat_map(|ch| std::iter::repeat_n('*', ch.len_utf8()))
            .collect(),
    }
}

impl Default for SingleLineInputState {
    fn default() -> Self {
        Self {
            text: String::new(),
            selection_anchor: 0,
            selection_active: 0,
            geometry: InputGeometry::default(),
            focus: None,
            active: false,
            dragging: false,
        }
    }
}

#[allow(dead_code)]
impl SingleLineInputState {
    pub fn sanitize_single_line(text: &str) -> String {
        text.replace(&format!("{}{}", char::from(13), char::from(10)), " ")
            .replace(char::from(10), " ")
            .replace(char::from(13), " ")
    }
    pub fn selection(&self) -> Range<usize> {
        self.selection_anchor.min(self.selection_active)
            ..self.selection_anchor.max(self.selection_active)
    }
    pub fn replace_selection(&mut self, text: &str) {
        let text = Self::sanitize_single_line(text);
        let range = self.selection();
        self.text.replace_range(range.clone(), &text);
        let end = range.start + text.len();
        self.selection_anchor = end;
        self.selection_active = end;
    }
    pub fn move_left(&mut self) {
        let pos = self.selection();
        self.selection_active = if !pos.is_empty() {
            pos.start
        } else {
            self.text[..self.selection_active]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i)
        };
        self.selection_anchor = self.selection_active;
    }
    pub fn move_right(&mut self) {
        let pos = self.selection();
        self.selection_active = if !pos.is_empty() {
            pos.end
        } else {
            self.text[self.selection_active..]
                .chars()
                .next()
                .map_or(self.text.len(), |c| self.selection_active + c.len_utf8())
        };
        self.selection_anchor = self.selection_active;
    }
    pub fn backspace(&mut self) {
        let pos = self.selection();
        if !pos.is_empty() {
            self.replace_selection("");
        } else if self.selection_active > 0 {
            let end = self.selection_active;
            self.selection_active = self.text[..end]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
            self.replace_selection("");
        }
    }
    pub fn delete(&mut self) {
        let pos = self.selection();
        if !pos.is_empty() {
            self.replace_selection("");
        } else if self.selection_active < self.text.len() {
            let end = self.selection_active
                + self.text[self.selection_active..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8);
            self.text.replace_range(self.selection_active..end, "");
        }
    }
    pub fn select_all(&mut self) {
        self.selection_anchor = 0;
        self.selection_active = self.text.len();
    }
    pub fn move_home(&mut self, extend: bool) {
        if !extend {
            self.selection_anchor = 0;
        }
        self.selection_active = 0;
    }
    pub fn move_end(&mut self, extend: bool) {
        if !extend {
            self.selection_anchor = self.text.len();
        }
        self.selection_active = self.text.len();
    }
    pub fn extend_left(&mut self) {
        self.selection_active = self.text[..self.selection_active]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i);
    }
    pub fn extend_right(&mut self) {
        if let Some(ch) = self.text[self.selection_active..].chars().next() {
            self.selection_active += ch.len_utf8();
        }
    }
    pub fn begin_drag(&mut self, offset: usize) {
        self.selection_anchor = offset;
        self.selection_active = offset;
        self.dragging = true;
    }
    pub fn mouse_down(&mut self, offset: usize, click_count: usize) {
        if click_count >= 2 {
            self.select_all();
            self.dragging = false;
        } else {
            self.begin_drag(offset);
        }
    }
    pub fn drag_to(&mut self, offset: usize) {
        if self.dragging {
            self.selection_active = offset.min(self.text.len());
        }
    }
    pub fn end_drag(&mut self) {
        self.dragging = false;
    }
}

impl InputGeometry {
    pub fn anchor(&self) -> Option<Point<Pixels>> {
        self.0.borrow().as_ref().map(|(_, origin)| *origin)
    }

    pub fn hit_test(&self, x: Pixels) -> usize {
        let raw = self.0.borrow().as_ref().map_or(0, |(line, origin)| {
            let local_x = (x - origin.x).max(px(0.));
            // Include the trailing edge: GPUI's closest_index_for_x jumps
            // straight to len after the final glyph origin, even before its midpoint.
            let mut nearest = (0, local_x);
            for (byte, glyph_x) in line
                .runs
                .iter()
                .flat_map(|run| {
                    run.glyphs
                        .iter()
                        .map(|glyph| (glyph.index, glyph.position.x))
                })
                .chain([(line.len(), line.width)])
            {
                let distance = (glyph_x - local_x).abs();
                if distance < nearest.1 {
                    nearest = (byte, distance);
                }
            }
            nearest.0
        });
        if let Some(boundaries) = self.1.borrow().as_ref() {
            boundaries
                .iter()
                .copied()
                .min_by_key(|b| b.abs_diff(raw))
                .unwrap_or(0)
        } else {
            raw
        }
    }

    pub fn set_valid_boundaries(&self, boundaries: Option<Vec<usize>>) {
        *self.1.borrow_mut() = boundaries;
    }
}

fn shape(text: String, window: &mut Window) -> ShapedLine {
    let text: SharedString = text.into();
    let run = TextRun {
        len: text.len(),
        font: super::metrics::code_font(),
        color: super::theme().text_primary.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(text, px(14.), &[run], None)
}

pub fn render<T: EntityInputHandler>(
    entity: Entity<T>,
    focus: FocusHandle,
    text: String,
    selection: Range<usize>,
    active: usize,
    caret_visible: bool,
    geometry: InputGeometry,
) -> impl IntoElement {
    let theme = super::theme();
    canvas(
        move |bounds, window, _| {
            let line = shape(text, window);
            let caret = line.x_for_index(active);
            let shift = (caret - bounds.size.width + px(2.)).max(px(0.));
            let origin = point(
                bounds.left() - shift,
                bounds.top() + (bounds.size.height - px(22.)) / 2.,
            );
            geometry.0.replace(Some((line.clone(), origin)));
            (line, origin)
        },
        move |bounds, (line, origin), window, cx| {
            window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
            if !selection.is_empty() {
                let start = line.x_for_index(selection.start);
                let end = line.x_for_index(selection.end);
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + start, origin.y),
                        size(end - start, px(22.)),
                    ),
                    theme.selection,
                ));
            }
            let _ = line.paint(origin, px(22.), window, cx);
            if caret_visible && focus.is_focused(window) {
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + line.x_for_index(active), origin.y),
                        size(px(1.), px(22.)),
                    ),
                    theme.text_primary,
                ));
            }
        },
    )
    .size_full()
}

/// Adapter for independent input state; delegates to the canonical renderer.
#[allow(dead_code)]
pub fn render_state<T: EntityInputHandler>(
    entity: Entity<T>,
    state: &SingleLineInputState,
) -> impl IntoElement {
    let focus = state
        .focus
        .clone()
        .expect("input state focus must be attached before rendering");
    render(
        entity,
        focus,
        state.text.clone(),
        state.selection_anchor.min(state.selection_active)
            ..state.selection_anchor.max(state.selection_active),
        state.selection_active,
        state.active,
        state.geometry.clone(),
    )
}

/// Renders an input using a display-only visual mode. State, selection,
/// clipboard and hit-test offsets continue to refer to the real text.
#[allow(dead_code)]
pub fn render_state_with_mode<T: EntityInputHandler>(
    entity: Entity<T>,
    state: &SingleLineInputState,
    mode: InputVisualMode,
) -> impl IntoElement {
    let boundaries = if mode == InputVisualMode::Masked {
        Some(
            state
                .text
                .char_indices()
                .map(|(i, _)| i)
                .chain([state.text.len()])
                .collect(),
        )
    } else {
        None
    };
    state.geometry.set_valid_boundaries(boundaries);
    let focus = state
        .focus
        .clone()
        .expect("input state focus must be attached before rendering");
    render(
        entity,
        focus,
        visual_text(&state.text, mode),
        state.selection_anchor.min(state.selection_active)
            ..state.selection_anchor.max(state.selection_active),
        state.selection_active,
        state.active,
        state.geometry.clone(),
    )
}

#[cfg(test)]
mod state_isolation_tests {
    use super::*;

    #[test]
    fn input_blink_uses_existing_half_second_rule() {
        let start = std::time::Instant::now();
        let ms = std::time::Duration::from_millis;
        assert!(!blink_due(start + ms(499), start, start));
        assert!(blink_due(start + ms(500), start, start));
        assert!(!blink_due(start + ms(600), start, start + ms(500)));
        assert!(!blink_due(start + ms(600), start + ms(400), start));
    }

    #[test]
    fn masked_visual_preserves_real_text_and_byte_offsets() {
        let text = "sk-😀é";
        let masked = visual_text(text, InputVisualMode::Masked);
        assert_eq!(text, "sk-😀é");
        assert_eq!(masked.len(), text.len());
        assert!(masked.chars().all(|c| c == '*'));
        assert_eq!(
            text.char_indices().map(|(i, _)| i).collect::<Vec<_>>(),
            [0, 1, 2, 3, 7]
        );
    }

    #[test]
    fn double_click_selects_all_without_invalidating_empty_input() {
        let mut input = SingleLineInputState {
            text: "ab🔑ç".into(),
            ..Default::default()
        };
        input.mouse_down(3, 2);
        assert_eq!(input.selection(), 0..input.text.len());
        let mut empty = SingleLineInputState::default();
        empty.mouse_down(0, 2);
        assert!(empty.selection().is_empty());
    }

    #[test]
    fn drag_preserves_anchor_when_direction_reverses() {
        let mut input = SingleLineInputState {
            text: "0123456789abcdefghij".into(),
            ..Default::default()
        };
        input.begin_drag(8);
        input.drag_to(3);
        assert_eq!(input.selection_anchor, 8);
        assert_eq!(input.selection_active, 3);
        input.drag_to(10);
        assert_eq!(input.selection_anchor, 8);
        assert_eq!(input.selection_active, 10);
    }

    #[test]
    fn input_cut_unicode_and_empty_selection() {
        let mut text = "a🙂ação.txt".to_owned();
        let (clipboard, caret) = cut_selection(&mut text, 1..11).unwrap();
        assert_eq!(clipboard, "🙂ação");
        assert_eq!(text, "a.txt");
        assert_eq!(caret, 1);
        assert_eq!(cut_selection(&mut text, 1..1), None);
        assert_eq!(text, "a.txt");
    }

    #[test]
    fn replace_all_text_handles_sizes_empty_and_adjacent_ranges() {
        let ranges = vec![0..1, 1..2, 3..4];
        assert_eq!(replace_all_text("abcd", &ranges, "🙂"), "🙂🙂c🙂");
        assert_eq!(replace_all_text("abc", &[0..1, 1..2, 2..3], ""), "");
        assert_eq!(replace_all_text("abc", &[], "x"), "abc");
        assert_eq!(replace_all_text("aba", &[0..1, 2..3], "aba"), "abababa");
    }

    #[gpui::test]
    fn ui_input_click_and_caret_share_glyphs_and_actual_origin(cx: &mut gpui::TestAppContext) {
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            for text in ["document", "ação🙂.txt"] {
                let line = shape(text.to_owned(), window);
                for origin_x in [px(17.), px(341.), px(700.)] {
                    let geometry = InputGeometry::default();
                    geometry
                        .0
                        .replace(Some((line.clone(), point(origin_x, px(0.)))));
                    assert_eq!(geometry.hit_test(origin_x - px(10.)), 0);
                    assert_eq!(
                        geometry.hit_test(origin_x + line.width + px(10.)),
                        text.len()
                    );
                    for byte in text
                        .char_indices()
                        .map(|(byte, _)| byte)
                        .chain([text.len()])
                    {
                        let clicked = geometry.hit_test(origin_x + line.x_for_index(byte));
                        assert_eq!(clicked, byte, "{text} at {byte}");
                        let mut edited = text.to_owned();
                        edited.insert_str(clicked, "X");
                        assert_eq!(&edited[..byte], &text[..byte]);
                        assert_eq!(&edited[byte..byte + 1], "X");
                    }
                }
            }
        });
    }

    #[test]
    fn ui_input_navigation_and_deletion_never_split_surrogate_pairs() {
        let text = "a🙂b";
        assert_eq!(adjacent_utf16(text, 1, true), 3);
        assert_eq!(adjacent_utf16(text, 3, false), 1);
        assert_eq!(adjacent_utf16(text, 0, false), 0);
        assert_eq!(adjacent_utf16(text, 4, true), 4);
    }
}

pub fn adjacent_utf16(text: &str, offset: usize, forward: bool) -> usize {
    let mut units = 0;
    let mut previous = 0;
    for ch in text.chars() {
        if forward && units > offset {
            return units;
        }
        if !forward && units >= offset {
            return previous;
        }
        previous = units;
        units += ch.len_utf16();
    }
    if forward { units } else { previous }
}

#[derive(Debug, Default)]
pub struct TextInput {
    pub query: String,
    pub selection_anchor: usize,
    pub selection_active: usize,
}
impl TextInput {
    pub fn selection(&self) -> Range<usize> {
        self.selection_anchor.min(self.selection_active)
            ..self.selection_anchor.max(self.selection_active)
    }

    pub fn set_caret(&mut self, offset: usize) {
        let offset = {
            let mut offset = offset.min(self.query.len());
            while !self.query.is_char_boundary(offset) {
                offset -= 1;
            }
            offset
        };
        self.selection_anchor = offset;
        self.selection_active = offset;
    }

    pub fn select_all(&mut self) {
        self.selection_anchor = 0;
        self.selection_active = self.query.len();
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        let selection = self.selection();
        if selection.is_empty() && text.is_empty() {
            return false;
        }
        self.query.replace_range(selection.clone(), text);
        self.set_caret(selection.start + text.len());
        true
    }

    pub fn move_left(&mut self) {
        let selection = self.selection();
        let offset = if !selection.is_empty() {
            selection.start
        } else {
            self.query[..self.selection_active]
                .char_indices()
                .next_back()
                .map_or(0, |(offset, _)| offset)
        };
        self.set_caret(offset);
    }

    pub fn move_right(&mut self) {
        let selection = self.selection();
        let offset = if !selection.is_empty() {
            selection.end
        } else {
            self.query[self.selection_active..]
                .chars()
                .next()
                .map_or(self.query.len(), |character| {
                    self.selection_active + character.len_utf8()
                })
        };
        self.set_caret(offset);
    }

    pub fn backspace(&mut self) -> bool {
        let selection = self.selection();
        if !selection.is_empty() {
            return self.replace_selection("");
        }
        if self.selection_active == 0 {
            return false;
        }
        let previous = self.query[..self.selection_active]
            .char_indices()
            .next_back()
            .map_or(0, |(offset, _)| offset);
        self.selection_anchor = previous;
        self.replace_selection("")
    }

    pub fn delete(&mut self) -> bool {
        let selection = self.selection();
        if !selection.is_empty() {
            return self.replace_selection("");
        }
        if self.selection_active == self.query.len() {
            return false;
        }
        let next = self.selection_active
            + self.query[self.selection_active..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        self.selection_active = next;
        self.replace_selection("")
    }

    pub fn select_word_at(&mut self, offset: usize) {
        let range = word_range_at(&self.query, offset);
        self.selection_anchor = range.start;
        self.selection_active = range.end;
    }
}

fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let mut start = offset;
    while start > 0 {
        let previous = text[..start].char_indices().next_back();
        if previous.is_some_and(|(_, ch)| is_word(ch)) {
            start = previous.unwrap().0;
        } else {
            break;
        }
    }
    let end = text[offset..]
        .char_indices()
        .find(|(_, ch)| !is_word(*ch))
        .map_or(text.len(), |(byte, _)| offset + byte);
    start..end
}

#[cfg(test)]
mod tests {
    use super::SingleLineInputState;

    #[test]
    fn single_line_input_states_are_independent() {
        let mut a = SingleLineInputState::default();
        let b = SingleLineInputState::default();
        a.text = "alpha".into();
        a.selection_anchor = 1;
        a.selection_active = 4;
        a.active = true;
        a.select_all();
        a.replace_selection("beta");
        a.move_left();
        a.backspace();
        assert!(b.text.is_empty());
        assert_eq!((b.selection_anchor, b.selection_active), (0, 0));
        assert!(!b.active);
        assert!(!std::ptr::eq(&a.geometry, &b.geometry));
    }

    #[test]
    fn single_line_replacement_normalizes_newlines() {
        let mut input = SingleLineInputState::default();
        input.text = "prefix suffix".into();
        input.selection_anchor = 7;
        input.selection_active = 13;
        let multiline = format!("a{}{}b{}c", char::from(13), char::from(10), char::from(10));
        input.replace_selection(&multiline);
        assert_eq!(input.text, "prefix a b c");
        assert!(!input.text.contains(char::from(10)));
        assert!(!input.text.contains(char::from(13)));
    }

    #[test]
    fn single_line_handles_unicode_and_selection_bounds() {
        let mut input = SingleLineInputState::default();
        input.text = "😀 café".into();
        input.select_all();
        input.replace_selection("x	😀");
        assert_eq!(input.text, "x	😀");
        assert_eq!(input.selection_anchor, input.text.len());
        assert_eq!(input.selection_active, input.text.len());
        assert!(input.selection().end <= input.text.len());
    }

    #[test]
    fn single_line_multiline_paste_in_middle_preserves_spaces() {
        let mut input = SingleLineInputState::default();
        input.text = "abCD".into();
        input.selection_anchor = 2;
        input.selection_active = 2;
        let paste = format!(
            "one{}two{}{}three{}four",
            char::from(10),
            char::from(13),
            char::from(10),
            char::from(13)
        );
        input.replace_selection(&paste);
        assert_eq!(input.text, "abone two three fourCD");
        assert!(!input.text.contains(char::from(10)));
        assert!(!input.text.contains(char::from(13)));
        assert!(input.selection_anchor <= input.text.len());
    }

    #[test]
    fn single_line_empty_paste_and_large_text_remain_valid() {
        let mut input = SingleLineInputState::default();
        input.text = "keep".into();
        input.selection_anchor = 2;
        input.selection_active = 2;
        input.replace_selection("");
        assert_eq!(input.text, "keep");
        let large = (0..256)
            .map(|i| format!("line{i}{}", char::from(10)))
            .collect::<String>();
        input.replace_selection(&large);
        assert!(!input.text.contains(char::from(10)));
        assert!(!input.text.contains(char::from(13)));
        assert!(input.selection_active <= input.text.len());
    }
}
