use std::sync::OnceLock;

use gpui::{Font, FontFallbacks, Pixels, font, px};

pub const CODE_FONT_FAMILY: &str = "Cascadia Mono";
pub const CODE_FONT_FALLBACKS: &[&str] = &["Consolas", "DejaVu Sans Mono"];

/// The shared code/terminal font. GPUI adds its own platform fallback stack
/// after these explicitly preferred monospace families.
pub fn code_font() -> Font {
    let mut value = font(CODE_FONT_FAMILY);
    value.fallbacks = Some(FontFallbacks::from_fonts(
        CODE_FONT_FALLBACKS
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    ));
    value
}

#[allow(dead_code)] // Shared contract keeps editor interaction constants visually synchronized.
pub struct UiMetrics {
    pub menu_height: Pixels,
    pub toolbar_height: Pixels,
    pub tab_height: Pixels,
    pub status_bar_height: Pixels,
    pub activity_bar_width: Pixels,
    pub sidebar_default_width: Pixels,
    pub panel_header_height: Pixels,
    pub icon_size: Pixels,
    pub ui_font_size: Pixels,
    pub editor_font_size: Pixels,
    pub editor_line_height: Pixels,
    pub border_radius_small: Pixels,
    pub border_radius_medium: Pixels,
    pub spacing_xs: Pixels,
    pub spacing_sm: Pixels,
    pub spacing_md: Pixels,
    pub spacing_lg: Pixels,
}

pub fn metrics() -> &'static UiMetrics {
    static METRICS: OnceLock<UiMetrics> = OnceLock::new();
    METRICS.get_or_init(|| UiMetrics {
        menu_height: px(30.),
        toolbar_height: px(28.),
        tab_height: px(35.),
        status_bar_height: px(23.),
        activity_bar_width: px(42.),
        sidebar_default_width: px(244.),
        panel_header_height: px(32.),
        icon_size: px(15.),
        ui_font_size: px(12.),
        editor_font_size: px(14.),
        editor_line_height: px(22.),
        border_radius_small: px(3.),
        border_radius_medium: px(6.),
        spacing_xs: px(4.),
        spacing_sm: px(7.),
        spacing_md: px(11.),
        spacing_lg: px(16.),
    })
}
