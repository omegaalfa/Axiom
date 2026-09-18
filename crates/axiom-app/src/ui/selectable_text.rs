use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{
    Bounds, Pixels, Point, SharedString, TextRun, WrappedLine, canvas, div, fill, point,
    prelude::*, px, size,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatTextSelection {
    pub message_id: u64,
    pub anchor: usize,
    pub head: usize,
}

impl ChatTextSelection {
    pub fn range(&self, text_len: usize) -> Range<usize> {
        self.anchor.min(self.head).min(text_len)..self.anchor.max(self.head).min(text_len)
    }
}

pub fn selected_text(text: &str, selection: &ChatTextSelection) -> Option<String> {
    let range = selection.range(text.len());
    if range.is_empty() || !text.is_char_boundary(range.start) || !text.is_char_boundary(range.end)
    {
        return None;
    }
    Some(text[range].to_owned())
}

#[derive(Clone, Default)]
pub struct SelectableTextGeometry(Rc<RefCell<Option<SelectableTextLayout>>>);

#[derive(Clone)]
struct SelectableTextLayout {
    bounds: Bounds<Pixels>,
    lines: Vec<WrappedLine>,
    line_height: Pixels,
}

impl SelectableTextGeometry {
    pub fn hit_test(&self, position: Point<Pixels>) -> Option<usize> {
        let state = self.0.borrow();
        let layout = state.as_ref()?;
        let mut local = position - layout.bounds.origin;
        let mut start = 0;
        for line in &layout.lines {
            let height = line.size(layout.line_height).height;
            if local.y <= height {
                let index = line
                    .closest_index_for_position(gpui::point(local.x, local.y), layout.line_height)
                    .unwrap_or(line.len());
                return Some(start + index);
            }
            local.y -= height;
            start += line.text.len() + 1;
        }
        Some(start.saturating_sub(1))
    }
}

pub fn render(
    text: String,
    selection: Option<Range<usize>>,
    geometry: SelectableTextGeometry,
) -> impl IntoElement {
    let selection = selection.unwrap_or(0..0);
    let layout_text = text.clone();
    let highlight = canvas(
        move |bounds, window, _| {
            let run = TextRun {
                len: layout_text.len(),
                font: super::metrics::code_font(),
                color: super::theme().text_secondary.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let lines = window
                .text_system()
                .shape_text(
                    SharedString::from(layout_text.clone()),
                    px(14.),
                    &[run],
                    Some(bounds.size.width),
                    None,
                )
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            let line_height = px(22.);
            geometry.0.replace(Some(SelectableTextLayout {
                bounds,
                lines: lines.clone(),
                line_height,
            }));
            (lines, line_height)
        },
        move |bounds, (lines, line_height), window, _cx| {
            let mut start = 0;
            let mut line_y = bounds.top() + px(2.);
            let color = super::theme().selection;
            for line in &lines {
                let origin = point(bounds.left(), line_y);
                if !selection.is_empty() {
                    let mut segments = vec![0];
                    segments.extend(line.wrap_boundaries().iter().filter_map(|boundary| {
                        let index = line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index;
                        line.position_for_index(index, line_height).map(|_| index)
                    }));
                    segments.push(line.len());
                    for pair in segments.windows(2) {
                        let a = start + pair[0];
                        let b = start + pair[1];
                        let left = a.max(selection.start);
                        let right = b.min(selection.end);
                        if left < right {
                            if let (Some(lp), Some(rp)) = (
                                line.position_for_index(left - start, line_height),
                                line.position_for_index(right - start, line_height),
                            ) {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(origin.x + lp.x, origin.y + lp.y),
                                        size(rp.x - lp.x, line_height),
                                    ),
                                    color,
                                ));
                            }
                        }
                    }
                }
                line_y += line.size(line_height).height;
                start += line.text.len() + 1;
            }
        },
    )
    .absolute()
    .inset_0();
    div()
        .relative()
        .w_full()
        .min_w_0()
        .child(highlight)
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_forward_backward_unicode_ranges() {
        let text = "João 🚀 versão";
        let selection = ChatTextSelection {
            message_id: 7,
            anchor: text.find("🚀").unwrap(),
            head: text.len(),
        };
        assert_eq!(
            selected_text(text, &selection).as_deref(),
            Some("🚀 versão")
        );
        let reverse = ChatTextSelection {
            message_id: 7,
            anchor: text.len(),
            head: 0,
        };
        assert_eq!(selected_text(text, &reverse).as_deref(), Some(text));
    }
}
