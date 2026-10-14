<?php
// Runtime declarations exported from extension: readline

namespace {
    function readline(?string $prompt = NULL): string|false {}

    function readline_info(?string $var_name = NULL, $value = NULL): mixed {}

    function readline_add_history(string $prompt): true {}

    function readline_clear_history(): true {}

    function readline_list_history(): array {}

    function readline_read_history(?string $filename = NULL): bool {}

    function readline_write_history(?string $filename = NULL): bool {}

    function readline_completion_function(callable $callback): bool {}

    function readline_callback_handler_install(string $prompt, callable $callback): true {}

    function readline_callback_read_char(): void {}

    function readline_callback_handler_remove(): bool {}

    function readline_redisplay(): void {}

    function readline_on_new_line(): void {}

    const READLINE_LIB = 'libedit';

}
