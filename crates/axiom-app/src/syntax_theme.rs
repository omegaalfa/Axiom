use std::ops::Range;

use axiom_syntax::{HighlightKind, HighlightSpan};
use gpui::{HighlightStyle, StyledText};

use crate::ui::theme;

pub fn styled_segment(text: String, absolute_start: usize, spans: &[HighlightSpan]) -> StyledText {
    let absolute_end = absolute_start + text.len();
    let mut cursor = absolute_start;
    let highlights = spans.iter().filter_map(|span| {
        let start = span.start_byte.max(absolute_start).max(cursor);
        let end = span.end_byte.min(absolute_end);
        if start >= end {
            return None;
        }
        cursor = end;
        Some((
            Range {
                start: start - absolute_start,
                end: end - absolute_start,
            },
            style(span.kind),
        ))
    });
    StyledText::new(text).with_highlights(highlights)
}

fn style(kind: HighlightKind) -> HighlightStyle {
    let theme = theme();
    let color = match kind {
        HighlightKind::Keyword => theme.syntax_keyword,
        HighlightKind::String => theme.syntax_string,
        HighlightKind::Comment => theme.syntax_comment,
        HighlightKind::Number => theme.syntax_number,
        HighlightKind::Type => theme.syntax_type,
        HighlightKind::Function | HighlightKind::Method => theme.syntax_function,
        HighlightKind::Variable | HighlightKind::Property => theme.syntax_variable,
        HighlightKind::Constant => theme.syntax_constant,
        HighlightKind::Namespace => theme.syntax_namespace,
        HighlightKind::Operator | HighlightKind::Punctuation => theme.syntax_operator,
        HighlightKind::Attribute => theme.syntax_attribute,
    };
    HighlightStyle {
        color: Some(color.into()),
        ..Default::default()
    }
}
