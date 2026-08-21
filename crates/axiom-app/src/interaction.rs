//! Small Unicode-safe interaction helpers that do not depend on GPUI rendering.

use std::ops::Range;

pub fn viewport_y_to_line(
    pointer_y: f32,
    viewport_top: f32,
    scroll_offset_y: f32,
    line_height: f32,
    line_count: usize,
) -> usize {
    if line_count == 0 || line_height <= 0.0 {
        return 0;
    }
    (((pointer_y - viewport_top - scroll_offset_y).max(0.0) / line_height).floor() as usize)
        .min(line_count - 1)
}

pub fn text_local_x(pointer_x: f32, viewport_left: f32, text_origin: f32) -> f32 {
    (pointer_x - viewport_left - text_origin).max(0.0)
}

pub fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = floor_char_boundary(text, offset.min(text.len()));
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    let mut start = offset;
    while let Some((index, character)) = text[..start].char_indices().next_back() {
        if !is_word(character) {
            break;
        }
        start = index;
    }
    let mut end = offset;
    for (relative, character) in text[offset..].char_indices() {
        if !is_word(character) {
            break;
        }
        end = offset + relative + character.len_utf8();
    }
    start..end
}

fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_ascii_and_unicode_words_at_byte_offsets() {
        assert_eq!(word_range_at("hello world", 2), 0..5);
        assert_eq!(word_range_at("ação Olá 👋", 3), 0..6);
        assert_eq!(word_range_at("ação Olá 👋", 8), 7..11);
    }

    #[test]
    fn punctuation_empty_lines_and_end_are_safe() {
        assert_eq!(word_range_at("", 0), 0..0);
        assert_eq!(word_range_at("foo bar", 3), 0..3);
        assert_eq!(word_range_at("foo bar", 7), 4..7);
    }

    #[test]
    fn viewport_coordinates_include_subpixel_scroll_and_clamp() {
        assert_eq!(viewport_y_to_line(100.0, 100.0, 0.0, 22.0, 100), 0);
        assert_eq!(viewport_y_to_line(121.9, 100.0, 0.0, 22.0, 100), 0);
        assert_eq!(viewport_y_to_line(122.0, 100.0, 0.0, 22.0, 100), 1);
        assert_eq!(viewport_y_to_line(105.0, 100.0, -220.5, 22.0, 100), 10);
        assert_eq!(viewport_y_to_line(10_000.0, 100.0, 0.0, 22.0, 3), 2);
        assert_eq!(text_local_x(176.0, 100.0, 76.0), 0.0);
        assert_eq!(text_local_x(181.5, 100.0, 76.0), 5.5);
        assert_eq!(text_local_x(120.0, 100.0, 76.0), 0.0);
    }

    #[test]
    fn secondary_shortcut_is_control_outside_macos() {
        let shortcut = gpui::Keystroke::parse("secondary-c").unwrap();
        #[cfg(not(target_os = "macos"))]
        {
            assert!(shortcut.modifiers.control);
            assert!(!shortcut.modifiers.platform);
        }
        assert!(!shortcut.modifiers.alt);
    }
}
