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

#[derive(Clone, Default)]
pub struct InputGeometry(Rc<RefCell<Option<(ShapedLine, Point<Pixels>)>>>);

impl InputGeometry {
    pub fn hit_test(&self, x: Pixels) -> usize {
        self.0.borrow().as_ref().map_or(0, |(line, origin)| {
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
        })
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

#[cfg(test)]
mod tests {
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
    fn input_cut_unicode_and_empty_selection() {
        let mut text = "a🙂ação.txt".to_owned();
        let (clipboard, caret) = cut_selection(&mut text, 1..11).unwrap();
        assert_eq!(clipboard, "🙂ação");
        assert_eq!(text, "a.txt");
        assert_eq!(caret, 1);
        assert_eq!(cut_selection(&mut text, 1..1), None);
        assert_eq!(text, "a.txt");
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
