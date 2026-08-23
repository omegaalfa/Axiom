use gpui::{AnyView, App, Context, IntoElement, Render, SharedString, Window, div, prelude::*};

use super::{metrics, theme};

struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        div()
            .px_2()
            .py_1()
            .rounded(m.border_radius_small)
            .border_1()
            .border_color(t.border)
            .bg(t.elevated_surface)
            .text_color(t.text_primary)
            .text_size(m.ui_font_size)
            .shadow_md()
            .child(self.text.clone())
    }
}

pub fn separator() -> impl IntoElement {
    div().h_px().mx_1().bg(theme().border_subtle)
}

pub fn tooltip(text: impl Into<SharedString>, cx: &mut App) -> AnyView {
    cx.new(|_| Tooltip { text: text.into() }).into()
}
