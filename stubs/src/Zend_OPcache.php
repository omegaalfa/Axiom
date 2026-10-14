<?php
// Runtime declarations exported from extension: Zend OPcache

namespace {
    function opcache_reset(): bool {}

    function opcache_get_status(bool $include_scripts = true): array|false {}

    function opcache_compile_file(string $filename): bool {}

    function opcache_invalidate(string $filename, bool $force = false): bool {}

    function opcache_jit_blacklist(\Closure $closure): void {}

    function opcache_get_configuration(): array|false {}

    function opcache_is_script_cached(string $filename): bool {}

    function opcache_is_script_cached_in_file_cache(string $filename): bool {}

}
