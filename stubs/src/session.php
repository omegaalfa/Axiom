<?php
// Runtime declarations exported from extension: session

namespace {
    function session_name(?string $name = NULL): string|false {}

    function session_module_name(?string $module = NULL): string|false {}

    function session_save_path(?string $path = NULL): string|false {}

    function session_id(?string $id = NULL): string|false {}

    function session_create_id(string $prefix = ''): string|false {}

    function session_regenerate_id(bool $delete_old_session = false): bool {}

    function session_decode(string $data): bool {}

    function session_encode(): string|false {}

    function session_destroy(): bool {}

    function session_unset(): bool {}

    function session_gc(): int|false {}

    function session_get_cookie_params(): array {}

    function session_write_close(): bool {}

    function session_abort(): bool {}

    function session_reset(): bool {}

    function session_status(): int {}

    function session_register_shutdown(): void {}

    function session_commit(): bool {}

    function session_set_save_handler($open, $close = null, callable $read = null, callable $write = null, callable $destroy = null, callable $gc = null, ?callable $create_sid = NULL, ?callable $validate_sid = NULL, ?callable $update_timestamp = NULL): bool {}

    function session_cache_limiter(?string $value = NULL): string|false {}

    function session_cache_expire(?int $value = NULL): int|false {}

    function session_set_cookie_params(array|int $lifetime_or_options, ?string $path = NULL, ?string $domain = NULL, ?bool $secure = NULL, ?bool $httponly = NULL): bool {}

    function session_start(array $options = array ()): bool {}

    interface SessionHandlerInterface {
        public function open(string $path, string $name);
        public function close();
        public function read(string $id);
        public function write(string $id, string $data);
        public function destroy(string $id);
        public function gc(int $max_lifetime);
    }

    interface SessionIdInterface {
        public function create_sid();
    }

    interface SessionUpdateTimestampHandlerInterface {
        public function validateId(string $id);
        public function updateTimestamp(string $id, string $data);
    }

    class SessionHandler implements \SessionHandlerInterface, \SessionIdInterface {
        public function open(string $path, string $name) {}
        public function close() {}
        public function read(string $id) {}
        public function write(string $id, string $data) {}
        public function destroy(string $id) {}
        public function gc(int $max_lifetime) {}
        public function create_sid() {}
    }

    const PHP_SESSION_DISABLED = 0;

    const PHP_SESSION_NONE = 1;

    const PHP_SESSION_ACTIVE = 2;

}
