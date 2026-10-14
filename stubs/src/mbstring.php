<?php
// Runtime declarations exported from extension: mbstring

namespace {
    function mb_language(?string $language = NULL): string|bool {}

    function mb_internal_encoding(?string $encoding = NULL): string|bool {}

    function mb_http_input(?string $type = NULL): array|string|false {}

    function mb_http_output(?string $encoding = NULL): string|bool {}

    function mb_detect_order(array|string|null $encoding = NULL): array|bool {}

    function mb_substitute_character(string|int|null $substitute_character = NULL): string|int|bool {}

    function mb_preferred_mime_name(string $encoding): string|false {}

    function mb_parse_str(string $string, &$result): bool {}

    function mb_output_handler(string $string, int $status): string {}

    function mb_str_split(string $string, int $length = 1, ?string $encoding = NULL): array {}

    function mb_strlen(string $string, ?string $encoding = NULL): int {}

    function mb_strpos(string $haystack, string $needle, int $offset = 0, ?string $encoding = NULL): int|false {}

    function mb_strrpos(string $haystack, string $needle, int $offset = 0, ?string $encoding = NULL): int|false {}

    function mb_stripos(string $haystack, string $needle, int $offset = 0, ?string $encoding = NULL): int|false {}

    function mb_strripos(string $haystack, string $needle, int $offset = 0, ?string $encoding = NULL): int|false {}

    function mb_strstr(string $haystack, string $needle, bool $before_needle = false, ?string $encoding = NULL): string|false {}

    function mb_strrchr(string $haystack, string $needle, bool $before_needle = false, ?string $encoding = NULL): string|false {}

    function mb_stristr(string $haystack, string $needle, bool $before_needle = false, ?string $encoding = NULL): string|false {}

    function mb_strrichr(string $haystack, string $needle, bool $before_needle = false, ?string $encoding = NULL): string|false {}

    function mb_substr_count(string $haystack, string $needle, ?string $encoding = NULL): int {}

    function mb_substr(string $string, int $start, ?int $length = NULL, ?string $encoding = NULL): string {}

    function mb_strcut(string $string, int $start, ?int $length = NULL, ?string $encoding = NULL): string {}

    function mb_strwidth(string $string, ?string $encoding = NULL): int {}

    function mb_strimwidth(string $string, int $start, int $width, string $trim_marker = '', ?string $encoding = NULL): string {}

    function mb_convert_encoding(array|string $string, string $to_encoding, array|string|null $from_encoding = NULL): array|string|false {}

    function mb_convert_case(string $string, int $mode, ?string $encoding = NULL): string {}

    function mb_strtoupper(string $string, ?string $encoding = NULL): string {}

    function mb_strtolower(string $string, ?string $encoding = NULL): string {}

    function mb_ucfirst(string $string, ?string $encoding = NULL): string {}

    function mb_lcfirst(string $string, ?string $encoding = NULL): string {}

    function mb_trim(string $string, ?string $characters = NULL, ?string $encoding = NULL): string {}

    function mb_ltrim(string $string, ?string $characters = NULL, ?string $encoding = NULL): string {}

    function mb_rtrim(string $string, ?string $characters = NULL, ?string $encoding = NULL): string {}

    function mb_detect_encoding(string $string, array|string|null $encodings = NULL, bool $strict = false): string|false {}

    function mb_list_encodings(): array {}

    function mb_encoding_aliases(string $encoding): array {}

    function mb_encode_mimeheader(string $string, ?string $charset = NULL, ?string $transfer_encoding = NULL, string $newline = '
    ', int $indent = 0): string {}

    function mb_decode_mimeheader(string $string): string {}

    function mb_convert_kana(string $string, string $mode = 'KV', ?string $encoding = NULL): string {}

    function mb_convert_variables(string $to_encoding, array|string $from_encoding, mixed &$var, mixed &...$vars): string|false {}

    function mb_encode_numericentity(string $string, array $map, ?string $encoding = NULL, bool $hex = false): string {}

    function mb_decode_numericentity(string $string, array $map, ?string $encoding = NULL): string {}

    function mb_send_mail(string $to, string $subject, string $message, array|string $additional_headers = array (), ?string $additional_params = NULL): bool {}

    function mb_get_info(string $type = 'all'): array|string|int|false|null {}

    function mb_check_encoding(array|string|null $value = NULL, ?string $encoding = NULL): bool {}

    function mb_scrub(string $string, ?string $encoding = NULL): string {}

    function mb_ord(string $string, ?string $encoding = NULL): int|false {}

    function mb_chr(int $codepoint, ?string $encoding = NULL): string|false {}

    function mb_str_pad(string $string, int $length, string $pad_string = ' ', int $pad_type = \STR_PAD_RIGHT, ?string $encoding = NULL): string {}

    function mb_regex_encoding(?string $encoding = NULL): string|bool {}

    function mb_ereg(string $pattern, string $string, &$matches = NULL): bool {}

    function mb_eregi(string $pattern, string $string, &$matches = NULL): bool {}

    function mb_ereg_replace(string $pattern, string $replacement, string $string, ?string $options = NULL): string|false|null {}

    function mb_eregi_replace(string $pattern, string $replacement, string $string, ?string $options = NULL): string|false|null {}

    function mb_ereg_replace_callback(string $pattern, callable $callback, string $string, ?string $options = NULL): string|false|null {}

    function mb_split(string $pattern, string $string, int $limit = -1): array|false {}

    function mb_ereg_match(string $pattern, string $string, ?string $options = NULL): bool {}

    function mb_ereg_search(?string $pattern = NULL, ?string $options = NULL): bool {}

    function mb_ereg_search_pos(?string $pattern = NULL, ?string $options = NULL): array|false {}

    function mb_ereg_search_regs(?string $pattern = NULL, ?string $options = NULL): array|false {}

    function mb_ereg_search_init(string $string, ?string $pattern = NULL, ?string $options = NULL): bool {}

    function mb_ereg_search_getregs(): array|false {}

    function mb_ereg_search_getpos(): int {}

    function mb_ereg_search_setpos(int $offset): bool {}

    function mb_regex_set_options(?string $options = NULL): string {}

    const MB_ONIGURUMA_VERSION = '6.9.10';

    const MB_CASE_UPPER = 0;

    const MB_CASE_LOWER = 1;

    const MB_CASE_TITLE = 2;

    const MB_CASE_FOLD = 3;

    const MB_CASE_UPPER_SIMPLE = 4;

    const MB_CASE_LOWER_SIMPLE = 5;

    const MB_CASE_TITLE_SIMPLE = 6;

    const MB_CASE_FOLD_SIMPLE = 7;

}
