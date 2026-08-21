use std::sync::OnceLock;

use gpui::{Rgba, rgb};

/// Semantic colors for the Axiom dark desktop theme.
#[allow(dead_code)] // Complete token contract; some tokens await GPUI scrollbar/focus hooks.
pub struct AxiomTheme {
    pub window_background: Rgba,
    pub editor_background: Rgba,
    pub panel_background: Rgba,
    pub sidebar_background: Rgba,
    pub elevated_surface: Rgba,
    pub menu_background: Rgba,
    pub popup_background: Rgba,
    pub border: Rgba,
    pub border_subtle: Rgba,
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_pressed: Rgba,
    pub selection: Rgba,
    pub inactive_selection: Rgba,
    pub hover: Rgba,
    pub pressed: Rgba,
    pub error: Rgba,
    pub warning: Rgba,
    pub success: Rgba,
    pub info: Rgba,
    pub gutter_background: Rgba,
    pub gutter_text: Rgba,
    pub active_line: Rgba,
    pub scrollbar: Rgba,
    pub scrollbar_hover: Rgba,
    pub syntax_keyword: Rgba,
    pub syntax_string: Rgba,
    pub syntax_comment: Rgba,
    pub syntax_number: Rgba,
    pub syntax_type: Rgba,
    pub syntax_function: Rgba,
    pub syntax_variable: Rgba,
    pub syntax_constant: Rgba,
    pub syntax_namespace: Rgba,
    pub syntax_operator: Rgba,
    pub syntax_attribute: Rgba,
}

pub fn theme() -> &'static AxiomTheme {
    static THEME: OnceLock<AxiomTheme> = OnceLock::new();
    THEME.get_or_init(|| AxiomTheme {
        window_background: rgb(0x17191d),
        editor_background: rgb(0x1b1d22),
        panel_background: rgb(0x1e2026),
        sidebar_background: rgb(0x1a1c21),
        elevated_surface: rgb(0x262930),
        menu_background: rgb(0x22252b),
        popup_background: rgb(0x252830),
        border: rgb(0x343841),
        border_subtle: rgb(0x292c33),
        text_primary: rgb(0xd7dae0),
        text_secondary: rgb(0xaeb3bd),
        text_muted: rgb(0x747b87),
        accent: rgb(0x6f9ee8),
        accent_hover: rgb(0x82acf0),
        accent_pressed: rgb(0x5d88ca),
        selection: rgb(0x344d70),
        inactive_selection: rgb(0x2a3749),
        hover: rgb(0x292d35),
        pressed: rgb(0x323741),
        error: rgb(0xe06c75),
        warning: rgb(0xd7a65a),
        success: rgb(0x78b892),
        info: rgb(0x76a9dc),
        gutter_background: rgb(0x191b20),
        gutter_text: rgb(0x606773),
        active_line: rgb(0x20242b),
        scrollbar: rgb(0x3a3e47),
        scrollbar_hover: rgb(0x525865),
        syntax_keyword: rgb(0xc397d8),
        syntax_string: rgb(0xa8c88a),
        syntax_comment: rgb(0x7c8491),
        syntax_number: rgb(0xd9a06f),
        syntax_type: rgb(0x77b8b7),
        syntax_function: rgb(0x82aee3),
        syntax_variable: rgb(0xcdd1d8),
        syntax_constant: rgb(0xd9a06f),
        syntax_namespace: rgb(0x82b9a4),
        syntax_operator: rgb(0xaeb5c0),
        syntax_attribute: rgb(0xd4b477),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_surfaces_and_states_are_distinct() {
        let theme = theme();
        assert_ne!(theme.editor_background, theme.panel_background);
        assert_ne!(theme.hover, theme.pressed);
        assert_ne!(theme.text_primary, theme.text_muted);
        assert_ne!(theme.error, theme.warning);
    }
}
