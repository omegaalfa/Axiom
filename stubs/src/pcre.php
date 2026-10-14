<?php
// Runtime declarations exported from extension: pcre

namespace {
    function preg_match(string $pattern, string $subject, &$matches = NULL, int $flags = 0, int $offset = 0): int|false {}

    function preg_match_all(string $pattern, string $subject, &$matches = NULL, int $flags = 0, int $offset = 0): int|false {}

    function preg_replace(array|string $pattern, array|string $replacement, array|string $subject, int $limit = -1, &$count = NULL): array|string|null {}

    function preg_filter(array|string $pattern, array|string $replacement, array|string $subject, int $limit = -1, &$count = NULL): array|string|null {}

    function preg_replace_callback(array|string $pattern, callable $callback, array|string $subject, int $limit = -1, &$count = NULL, int $flags = 0): array|string|null {}

    function preg_replace_callback_array(array $pattern, array|string $subject, int $limit = -1, &$count = NULL, int $flags = 0): array|string|null {}

    function preg_split(string $pattern, string $subject, int $limit = -1, int $flags = 0): array|false {}

    function preg_quote(string $str, ?string $delimiter = NULL): string {}

    function preg_grep(string $pattern, array $array, int $flags = 0): array|false {}

    function preg_last_error(): int {}

    function preg_last_error_msg(): string {}

    const PREG_PATTERN_ORDER = 1;

    const PREG_SET_ORDER = 2;

    const PREG_OFFSET_CAPTURE = 256;

    const PREG_UNMATCHED_AS_NULL = 512;

    const PREG_SPLIT_NO_EMPTY = 1;

    const PREG_SPLIT_DELIM_CAPTURE = 2;

    const PREG_SPLIT_OFFSET_CAPTURE = 4;

    const PREG_GREP_INVERT = 1;

    const PREG_NO_ERROR = 0;

    const PREG_INTERNAL_ERROR = 1;

    const PREG_BACKTRACK_LIMIT_ERROR = 2;

    const PREG_RECURSION_LIMIT_ERROR = 3;

    const PREG_BAD_UTF8_ERROR = 4;

    const PREG_BAD_UTF8_OFFSET_ERROR = 5;

    const PREG_JIT_STACKLIMIT_ERROR = 6;

    const PCRE_VERSION = '10.46 2025-08-27';

    const PCRE_VERSION_MAJOR = 10;

    const PCRE_VERSION_MINOR = 46;

    const PCRE_JIT_SUPPORT = true;

}
