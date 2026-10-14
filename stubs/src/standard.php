<?php
// Runtime declarations exported from extension: standard

namespace {
    function set_time_limit(int $seconds): bool {}

    function header_register_callback(callable $callback): bool {}

    function ob_start($callback = NULL, int $chunk_size = 0, int $flags = \PHP_OUTPUT_HANDLER_STDFLAGS): bool {}

    function ob_flush(): bool {}

    function ob_clean(): bool {}

    function ob_end_flush(): bool {}

    function ob_end_clean(): bool {}

    function ob_get_flush(): string|false {}

    function ob_get_clean(): string|false {}

    function ob_get_contents(): string|false {}

    function ob_get_level(): int {}

    function ob_get_length(): int|false {}

    function ob_list_handlers(): array {}

    function ob_get_status(bool $full_status = false): array {}

    function ob_implicit_flush(bool $enable = true): void {}

    function output_reset_rewrite_vars(): bool {}

    function output_add_rewrite_var(string $name, string $value): bool {}

    function stream_wrapper_register(string $protocol, string $class, int $flags = 0): bool {}

    function stream_register_wrapper(string $protocol, string $class, int $flags = 0): bool {}

    function stream_wrapper_unregister(string $protocol): bool {}

    function stream_wrapper_restore(string $protocol): bool {}

    function array_push(array &$array, mixed ...$values): int {}

    function krsort(array &$array, int $flags = \SORT_REGULAR): true {}

    function ksort(array &$array, int $flags = \SORT_REGULAR): true {}

    function count(\Countable|array $value, int $mode = \COUNT_NORMAL): int {}

    function sizeof(\Countable|array $value, int $mode = \COUNT_NORMAL): int {}

    function natsort(array &$array): true {}

    function natcasesort(array &$array): true {}

    function asort(array &$array, int $flags = \SORT_REGULAR): true {}

    function arsort(array &$array, int $flags = \SORT_REGULAR): true {}

    function sort(array &$array, int $flags = \SORT_REGULAR): true {}

    function rsort(array &$array, int $flags = \SORT_REGULAR): true {}

    function usort(array &$array, callable $callback): true {}

    function uasort(array &$array, callable $callback): true {}

    function uksort(array &$array, callable $callback): true {}

    function end(object|array &$array): mixed {}

    function prev(object|array &$array): mixed {}

    function next(object|array &$array): mixed {}

    function reset(object|array &$array): mixed {}

    function current(object|array $array): mixed {}

    function pos(object|array $array): mixed {}

    function key(object|array $array): string|int|null {}

    function min(mixed $value, mixed ...$values): mixed {}

    function max(mixed $value, mixed ...$values): mixed {}

    function array_walk(object|array &$array, callable $callback, mixed $arg = null): true {}

    function array_walk_recursive(object|array &$array, callable $callback, mixed $arg = null): true {}

    function in_array(mixed $needle, array $haystack, bool $strict = false): bool {}

    function array_search(mixed $needle, array $haystack, bool $strict = false): string|int|false {}

    function extract(array &$array, int $flags = \EXTR_OVERWRITE, string $prefix = ''): int {}

    function compact($var_name, ...$var_names): array {}

    function array_fill(int $start_index, int $count, mixed $value): array {}

    function array_fill_keys(array $keys, mixed $value): array {}

    function range(string|int|float $start, string|int|float $end, int|float $step = 1): array {}

    function shuffle(array &$array): true {}

    function array_pop(array &$array): mixed {}

    function array_shift(array &$array): mixed {}

    function array_unshift(array &$array, mixed ...$values): int {}

    function array_splice(array &$array, int $offset, ?int $length = NULL, mixed $replacement = array ()): array {}

    function array_slice(array $array, int $offset, ?int $length = NULL, bool $preserve_keys = false): array {}

    function array_merge(array ...$arrays): array {}

    function array_merge_recursive(array ...$arrays): array {}

    function array_replace(array $array, array ...$replacements): array {}

    function array_replace_recursive(array $array, array ...$replacements): array {}

    function array_keys(array $array, mixed $filter_value = null, bool $strict = false): array {}

    function array_key_first(array $array): string|int|null {}

    function array_key_last(array $array): string|int|null {}

    function array_first(array $array): mixed {}

    function array_last(array $array): mixed {}

    function array_values(array $array): array {}

    function array_count_values(array $array): array {}

    function array_column(array $array, string|int|null $column_key, string|int|null $index_key = NULL): array {}

    function array_reverse(array $array, bool $preserve_keys = false): array {}

    function array_pad(array $array, int $length, mixed $value): array {}

    function array_flip(array $array): array {}

    function array_change_key_case(array $array, int $case = \CASE_LOWER): array {}

    function array_unique(array $array, int $flags = \SORT_STRING): array {}

    function array_intersect_key(array $array, array ...$arrays): array {}

    function array_intersect_ukey(array $array, ...$rest): array {}

    function array_intersect(array $array, array ...$arrays): array {}

    function array_uintersect(array $array, ...$rest): array {}

    function array_intersect_assoc(array $array, array ...$arrays): array {}

    function array_uintersect_assoc(array $array, ...$rest): array {}

    function array_intersect_uassoc(array $array, ...$rest): array {}

    function array_uintersect_uassoc(array $array, ...$rest): array {}

    function array_diff_key(array $array, array ...$arrays): array {}

    function array_diff_ukey(array $array, ...$rest): array {}

    function array_diff(array $array, array ...$arrays): array {}

    function array_udiff(array $array, ...$rest): array {}

    function array_diff_assoc(array $array, array ...$arrays): array {}

    function array_diff_uassoc(array $array, ...$rest): array {}

    function array_udiff_assoc(array $array, ...$rest): array {}

    function array_udiff_uassoc(array $array, ...$rest): array {}

    function array_multisort(&$array, &...$rest): true {}

    function array_rand(array $array, int $num = 1): array|string|int {}

    function array_sum(array $array): int|float {}

    function array_product(array $array): int|float {}

    function array_reduce(array $array, callable $callback, mixed $initial = NULL): mixed {}

    function array_filter(array $array, ?callable $callback = NULL, int $mode = 0): array {}

    function array_find(array $array, callable $callback): mixed {}

    function array_find_key(array $array, callable $callback): mixed {}

    function array_any(array $array, callable $callback): bool {}

    function array_all(array $array, callable $callback): bool {}

    function array_map(?callable $callback, array $array, array ...$arrays): array {}

    function array_key_exists($key, array $array): bool {}

    function key_exists($key, array $array): bool {}

    function array_chunk(array $array, int $length, bool $preserve_keys = false): array {}

    function array_combine(array $keys, array $values): array {}

    function array_is_list(array $array): bool {}

    function base64_encode(string $string): string {}

    function base64_decode(string $string, bool $strict = false): string|false {}

    function constant(string $name): mixed {}

    function ip2long(string $ip): int|false {}

    function long2ip(int $ip): string {}

    function getenv(?string $name = NULL, bool $local_only = false): array|string|false {}

    function putenv(string $assignment): bool {}

    function getopt(string $short_options, array $long_options = array (), &$rest_index = NULL): array|false {}

    function flush(): void {}

    function sleep(int $seconds): int {}

    function usleep(int $microseconds): void {}

    function time_nanosleep(int $seconds, int $nanoseconds): array|bool {}

    function time_sleep_until(float $timestamp): bool {}

    function get_current_user(): string {}

    function get_cfg_var(string $option): array|string|false {}

    function error_log(string $message, int $message_type = 0, ?string $destination = NULL, ?string $additional_headers = NULL): bool {}

    function error_get_last(): ?array {}

    function error_clear_last(): void {}

    function call_user_func(callable $callback, mixed ...$args): mixed {}

    function call_user_func_array(callable $callback, array $args): mixed {}

    function forward_static_call(callable $callback, mixed ...$args): mixed {}

    function forward_static_call_array(callable $callback, array $args): mixed {}

    function register_shutdown_function(callable $callback, mixed ...$args): void {}

    function highlight_file(string $filename, bool $return = false): string|bool {}

    function show_source(string $filename, bool $return = false): string|bool {}

    function php_strip_whitespace(string $filename): string {}

    function highlight_string(string $string, bool $return = false): string|true {}

    function ini_get(string $option): string|false {}

    function ini_get_all(?string $extension = NULL, bool $details = true): array|false {}

    function ini_set(string $option, string|int|float|bool|null $value): string|false {}

    function ini_alter(string $option, string|int|float|bool|null $value): string|false {}

    function ini_restore(string $option): void {}

    function ini_parse_quantity(string $shorthand): int {}

    function set_include_path(string $include_path): string|false {}

    function get_include_path(): string|false {}

    function print_r(mixed $value, bool $return = false): string|true {}

    function connection_aborted(): int {}

    function connection_status(): int {}

    function ignore_user_abort(?bool $enable = NULL): int {}

    function getservbyname(string $service, string $protocol): int|false {}

    function getservbyport(int $port, string $protocol): string|false {}

    function getprotobyname(string $protocol): int|false {}

    function getprotobynumber(int $protocol): string|false {}

    function register_tick_function(callable $callback, mixed ...$args): bool {}

    function unregister_tick_function(callable $callback): void {}

    function is_uploaded_file(string $filename): bool {}

    function move_uploaded_file(string $from, string $to): bool {}

    function parse_ini_file(string $filename, bool $process_sections = false, int $scanner_mode = \INI_SCANNER_NORMAL): array|false {}

    function parse_ini_string(string $ini_string, bool $process_sections = false, int $scanner_mode = \INI_SCANNER_NORMAL): array|false {}

    function sys_getloadavg(): array|false {}

    function get_browser(?string $user_agent = NULL, bool $return_array = false): object|array|false {}

    function crc32(string $string): int {}

    function crypt(string $string, string $salt): string {}

    function strptime(string $timestamp, string $format): array|false {}

    function gethostname(): string|false {}

    function gethostbyaddr(string $ip): string|false {}

    function gethostbyname(string $hostname): string {}

    function gethostbynamel(string $hostname): array|false {}

    function dns_check_record(string $hostname, string $type = 'MX'): bool {}

    function checkdnsrr(string $hostname, string $type = 'MX'): bool {}

    function dns_get_record(string $hostname, int $type = \DNS_ANY, &$authoritative_name_servers = NULL, &$additional_records = NULL, bool $raw = false): array|false {}

    function dns_get_mx(string $hostname, &$hosts, &$weights = NULL): bool {}

    function getmxrr(string $hostname, &$hosts, &$weights = NULL): bool {}

    function net_get_interfaces(): array|false {}

    function ftok(string $filename, string $project_id): int {}

    function hrtime(bool $as_number = false): array|int|float|false {}

    function md5(string $string, bool $binary = false): string {}

    function md5_file(string $filename, bool $binary = false): string|false {}

    function getmyuid(): int|false {}

    function getmygid(): int|false {}

    function getmypid(): int|false {}

    function getmyinode(): int|false {}

    function getlastmod(): int|false {}

    function sha1(string $string, bool $binary = false): string {}

    function sha1_file(string $filename, bool $binary = false): string|false {}

    function openlog(string $prefix, int $flags, int $facility): true {}

    function closelog(): true {}

    function syslog(int $priority, string $message): true {}

    function inet_ntop(string $ip): string|false {}

    function inet_pton(string $ip): string|false {}

    function metaphone(string $string, int $max_phonemes = 0): string {}

    function header(string $header, bool $replace = true, int $response_code = 0): void {}

    function header_remove(?string $name = NULL): void {}

    function setrawcookie(string $name, string $value = '', array|int $expires_or_options = 0, string $path = '', string $domain = '', bool $secure = false, bool $httponly = false): bool {}

    function setcookie(string $name, string $value = '', array|int $expires_or_options = 0, string $path = '', string $domain = '', bool $secure = false, bool $httponly = false): bool {}

    function http_response_code(int $response_code = 0): int|bool {}

    function headers_sent(&$filename = NULL, &$line = NULL): bool {}

    function headers_list(): array {}

    function htmlspecialchars(string $string, int $flags = 11, ?string $encoding = NULL, bool $double_encode = true): string {}

    function htmlspecialchars_decode(string $string, int $flags = 11): string {}

    function html_entity_decode(string $string, int $flags = 11, ?string $encoding = NULL): string {}

    function htmlentities(string $string, int $flags = 11, ?string $encoding = NULL, bool $double_encode = true): string {}

    function get_html_translation_table(int $table = \HTML_SPECIALCHARS, int $flags = 11, string $encoding = 'UTF-8'): array {}

    function assert(mixed $assertion, \Throwable|string|null $description = NULL): bool {}

    function assert_options(int $option, mixed $value = null): mixed {}

    function bin2hex(string $string): string {}

    function hex2bin(string $string): string|false {}

    function strspn(string $string, string $characters, int $offset = 0, ?int $length = NULL): int {}

    function strcspn(string $string, string $characters, int $offset = 0, ?int $length = NULL): int {}

    function nl_langinfo(int $item): string|false {}

    function strcoll(string $string1, string $string2): int {}

    function trim(string $string, string $characters = ' 
    	' . "\0" . ''): string {}

    function rtrim(string $string, string $characters = ' 
    	' . "\0" . ''): string {}

    function chop(string $string, string $characters = ' 
    	' . "\0" . ''): string {}

    function ltrim(string $string, string $characters = ' 
    	' . "\0" . ''): string {}

    function wordwrap(string $string, int $width = 75, string $break = '
    ', bool $cut_long_words = false): string {}

    function explode(string $separator, string $string, int $limit = \PHP_INT_MAX): array {}

    function implode(array|string $separator, ?array $array = NULL): string {}

    function join(array|string $separator, ?array $array = NULL): string {}

    function strtok(string $string, ?string $token = NULL): string|false {}

    function strtoupper(string $string): string {}

    function strtolower(string $string): string {}

    function str_increment(string $string): string {}

    function str_decrement(string $string): string {}

    function basename(string $path, string $suffix = ''): string {}

    function dirname(string $path, int $levels = 1): string {}

    function pathinfo(string $path, int $flags = \PATHINFO_ALL): array|string {}

    function stristr(string $haystack, string $needle, bool $before_needle = false): string|false {}

    function strstr(string $haystack, string $needle, bool $before_needle = false): string|false {}

    function strchr(string $haystack, string $needle, bool $before_needle = false): string|false {}

    function strpos(string $haystack, string $needle, int $offset = 0): int|false {}

    function stripos(string $haystack, string $needle, int $offset = 0): int|false {}

    function strrpos(string $haystack, string $needle, int $offset = 0): int|false {}

    function strripos(string $haystack, string $needle, int $offset = 0): int|false {}

    function strrchr(string $haystack, string $needle, bool $before_needle = false): string|false {}

    function str_contains(string $haystack, string $needle): bool {}

    function str_starts_with(string $haystack, string $needle): bool {}

    function str_ends_with(string $haystack, string $needle): bool {}

    function chunk_split(string $string, int $length = 76, string $separator = '
    '): string {}

    function substr(string $string, int $offset, ?int $length = NULL): string {}

    function substr_replace(array|string $string, array|string $replace, array|int $offset, array|int|null $length = NULL): array|string {}

    function quotemeta(string $string): string {}

    function ord(string $character): int {}

    function chr(int $codepoint): string {}

    function ucfirst(string $string): string {}

    function lcfirst(string $string): string {}

    function ucwords(string $string, string $separators = ' 	
    '): string {}

    function strtr(string $string, array|string $from, ?string $to = NULL): string {}

    function strrev(string $string): string {}

    function similar_text(string $string1, string $string2, &$percent = NULL): int {}

    function addcslashes(string $string, string $characters): string {}

    function addslashes(string $string): string {}

    function stripcslashes(string $string): string {}

    function stripslashes(string $string): string {}

    function str_replace(array|string $search, array|string $replace, array|string $subject, &$count = NULL): array|string {}

    function str_ireplace(array|string $search, array|string $replace, array|string $subject, &$count = NULL): array|string {}

    function hebrev(string $string, int $max_chars_per_line = 0): string {}

    function nl2br(string $string, bool $use_xhtml = true): string {}

    function strip_tags(string $string, array|string|null $allowed_tags = NULL): string {}

    function setlocale(int $category, $locales, ...$rest): string|false {}

    function parse_str(string $string, &$result): void {}

    function str_getcsv(string $string, string $separator = ',', string $enclosure = '"', string $escape = '\\'): array {}

    function str_repeat(string $string, int $times): string {}

    function count_chars(string $string, int $mode = 0): array|string {}

    function strnatcmp(string $string1, string $string2): int {}

    function localeconv(): array {}

    function strnatcasecmp(string $string1, string $string2): int {}

    function substr_count(string $haystack, string $needle, int $offset = 0, ?int $length = NULL): int {}

    function str_pad(string $string, int $length, string $pad_string = ' ', int $pad_type = \STR_PAD_RIGHT): string {}

    function sscanf(string $string, string $format, mixed &...$vars): array|int|null {}

    function str_rot13(string $string): string {}

    function str_shuffle(string $string): string {}

    function str_word_count(string $string, int $format = 0, ?string $characters = NULL): array|int {}

    function str_split(string $string, int $length = 1): array {}

    function strpbrk(string $string, string $characters): string|false {}

    function substr_compare(string $haystack, string $needle, int $offset, ?int $length = NULL, bool $case_insensitive = false): int {}

    function utf8_encode(string $string): string {}

    function utf8_decode(string $string): string {}

    function opendir(string $directory, $context = NULL) {}

    function dir(string $directory, $context = NULL): \Directory|false {}

    function closedir($dir_handle = NULL): void {}

    function chdir(string $directory): bool {}

    function chroot(string $directory): bool {}

    function getcwd(): string|false {}

    function rewinddir($dir_handle = NULL): void {}

    function readdir($dir_handle = NULL): string|false {}

    function scandir(string $directory, int $sorting_order = \SCANDIR_SORT_ASCENDING, $context = NULL): array|false {}

    function glob(string $pattern, int $flags = 0): array|false {}

    function exec(string $command, &$output = NULL, &$result_code = NULL): string|false {}

    function system(string $command, &$result_code = NULL): string|false {}

    function passthru(string $command, &$result_code = NULL): false {}

    function escapeshellcmd(string $command): string {}

    function escapeshellarg(string $arg): string {}

    function shell_exec(string $command): string|false|null {}

    function proc_nice(int $priority): bool {}

    function flock($stream, int $operation, &$would_block = NULL): bool {}

    function get_meta_tags(string $filename, bool $use_include_path = false): array|false {}

    function pclose($handle): int {}

    function popen(string $command, string $mode) {}

    function readfile(string $filename, bool $use_include_path = false, $context = NULL): int|false {}

    function rewind($stream): bool {}

    function rmdir(string $directory, $context = NULL): bool {}

    function umask(?int $mask = NULL): int {}

    function fclose($stream): bool {}

    function feof($stream): bool {}

    function fgetc($stream): string|false {}

    function fgets($stream, ?int $length = NULL): string|false {}

    function fread($stream, int $length): string|false {}

    function fopen(string $filename, string $mode, bool $use_include_path = false, $context = NULL) {}

    function fscanf($stream, string $format, mixed &...$vars): array|int|false|null {}

    function fpassthru($stream): int {}

    function ftruncate($stream, int $size): bool {}

    function fstat($stream): array|false {}

    function fseek($stream, int $offset, int $whence = \SEEK_SET): int {}

    function ftell($stream): int|false {}

    function fflush($stream): bool {}

    function fsync($stream): bool {}

    function fdatasync($stream): bool {}

    function fwrite($stream, string $data, ?int $length = NULL): int|false {}

    function fputs($stream, string $data, ?int $length = NULL): int|false {}

    function mkdir(string $directory, int $permissions = 511, bool $recursive = false, $context = NULL): bool {}

    function rename(string $from, string $to, $context = NULL): bool {}

    function copy(string $from, string $to, $context = NULL): bool {}

    function tempnam(string $directory, string $prefix): string|false {}

    function tmpfile() {}

    function file(string $filename, int $flags = 0, $context = NULL): array|false {}

    function file_get_contents(string $filename, bool $use_include_path = false, $context = NULL, int $offset = 0, ?int $length = NULL): string|false {}

    function unlink(string $filename, $context = NULL): bool {}

    function file_put_contents(string $filename, mixed $data, int $flags = 0, $context = NULL): int|false {}

    function fputcsv($stream, array $fields, string $separator = ',', string $enclosure = '"', string $escape = '\\', string $eol = '
    '): int|false {}

    function fgetcsv($stream, ?int $length = NULL, string $separator = ',', string $enclosure = '"', string $escape = '\\'): array|false {}

    function realpath(string $path): string|false {}

    function fnmatch(string $pattern, string $filename, int $flags = 0): bool {}

    function sys_get_temp_dir(): string {}

    function fileatime(string $filename): int|false {}

    function filectime(string $filename): int|false {}

    function filegroup(string $filename): int|false {}

    function fileinode(string $filename): int|false {}

    function filemtime(string $filename): int|false {}

    function fileowner(string $filename): int|false {}

    function fileperms(string $filename): int|false {}

    function filesize(string $filename): int|false {}

    function filetype(string $filename): string|false {}

    function file_exists(string $filename): bool {}

    function is_writable(string $filename): bool {}

    function is_writeable(string $filename): bool {}

    function is_readable(string $filename): bool {}

    function is_executable(string $filename): bool {}

    function is_file(string $filename): bool {}

    function is_dir(string $filename): bool {}

    function is_link(string $filename): bool {}

    function stat(string $filename): array|false {}

    function lstat(string $filename): array|false {}

    function chown(string $filename, string|int $user): bool {}

    function chgrp(string $filename, string|int $group): bool {}

    function lchown(string $filename, string|int $user): bool {}

    function lchgrp(string $filename, string|int $group): bool {}

    function chmod(string $filename, int $permissions): bool {}

    function touch(string $filename, ?int $mtime = NULL, ?int $atime = NULL): bool {}

    function clearstatcache(bool $clear_realpath_cache = false, string $filename = ''): void {}

    function disk_total_space(string $directory): float|false {}

    function disk_free_space(string $directory): float|false {}

    function diskfreespace(string $directory): float|false {}

    function realpath_cache_get(): array {}

    function realpath_cache_size(): int {}

    function sprintf(string $format, mixed ...$values): string {}

    function printf(string $format, mixed ...$values): int {}

    function vprintf(string $format, array $values): int {}

    function vsprintf(string $format, array $values): string {}

    function fprintf($stream, string $format, mixed ...$values): int {}

    function vfprintf($stream, string $format, array $values): int {}

    function fsockopen(string $hostname, int $port = -1, &$error_code = NULL, &$error_message = NULL, ?float $timeout = NULL) {}

    function pfsockopen(string $hostname, int $port = -1, &$error_code = NULL, &$error_message = NULL, ?float $timeout = NULL) {}

    function http_build_query(object|array $data, string $numeric_prefix = '', ?string $arg_separator = NULL, int $encoding_type = \PHP_QUERY_RFC1738): string {}

    function http_get_last_response_headers(): ?array {}

    function http_clear_last_response_headers(): void {}

    function request_parse_body(?array $options = NULL): array {}

    function image_type_to_mime_type(int $image_type): string {}

    function image_type_to_extension(int $image_type, bool $include_dot = true): string|false {}

    function getimagesize(string $filename, &$image_info = NULL): array|false {}

    function getimagesizefromstring(string $string, &$image_info = NULL): array|false {}

    function phpinfo(int $flags = \INFO_ALL): true {}

    function phpversion(?string $extension = NULL): string|false {}

    function phpcredits(int $flags = \CREDITS_ALL): true {}

    function php_sapi_name(): string|false {}

    function php_uname(string $mode = 'a'): string {}

    function php_ini_scanned_files(): string|false {}

    function php_ini_loaded_file(): string|false {}

    function iptcembed(string $iptc_data, string $filename, int $spool = 0): string|bool {}

    function iptcparse(string $iptc_block): array|false {}

    function levenshtein(string $string1, string $string2, int $insertion_cost = 1, int $replacement_cost = 1, int $deletion_cost = 1): int {}

    function readlink(string $path): string|false {}

    function linkinfo(string $path): int|false {}

    function symlink(string $target, string $link): bool {}

    function link(string $target, string $link): bool {}

    function mail(string $to, string $subject, string $message, array|string $additional_headers = array (), string $additional_params = ''): bool {}

    function abs(int|float $num): int|float {}

    function ceil(int|float $num): float {}

    function floor(int|float $num): float {}

    function round(int|float $num, int $precision = 0, \RoundingMode|int $mode = \RoundingMode::HalfAwayFromZero): float {}

    function sin(float $num): float {}

    function cos(float $num): float {}

    function tan(float $num): float {}

    function asin(float $num): float {}

    function acos(float $num): float {}

    function atan(float $num): float {}

    function atanh(float $num): float {}

    function atan2(float $y, float $x): float {}

    function sinh(float $num): float {}

    function cosh(float $num): float {}

    function tanh(float $num): float {}

    function asinh(float $num): float {}

    function acosh(float $num): float {}

    function expm1(float $num): float {}

    function log1p(float $num): float {}

    function pi(): float {}

    function is_finite(float $num): bool {}

    function is_nan(float $num): bool {}

    function intdiv(int $num1, int $num2): int {}

    function is_infinite(float $num): bool {}

    function pow(mixed $num, mixed $exponent): object|int|float {}

    function exp(float $num): float {}

    function log(float $num, float $base = \M_E): float {}

    function log10(float $num): float {}

    function sqrt(float $num): float {}

    function hypot(float $x, float $y): float {}

    function deg2rad(float $num): float {}

    function rad2deg(float $num): float {}

    function bindec(string $binary_string): int|float {}

    function hexdec(string $hex_string): int|float {}

    function octdec(string $octal_string): int|float {}

    function decbin(int $num): string {}

    function decoct(int $num): string {}

    function dechex(int $num): string {}

    function base_convert(string $num, int $from_base, int $to_base): string {}

    function number_format(float $num, int $decimals = 0, ?string $decimal_separator = '.', ?string $thousands_separator = ','): string {}

    function fmod(float $num1, float $num2): float {}

    function fdiv(float $num1, float $num2): float {}

    function fpow(float $num, float $exponent): float {}

    function microtime(bool $as_float = false): string|float {}

    function gettimeofday(bool $as_float = false): array|float {}

    function getrusage(int $mode = 0): array|false {}

    function pack(string $format, mixed ...$values): string {}

    function unpack(string $format, string $string, int $offset = 0): array|false {}

    function password_get_info(string $hash): array {}

    function password_hash(string $password, string|int|null $algo, array $options = array ()): string {}

    function password_needs_rehash(string $hash, string|int|null $algo, array $options = array ()): bool {}

    function password_verify(string $password, string $hash): bool {}

    function password_algos(): array {}

    function proc_open(array|string $command, array $descriptor_spec, &$pipes, ?string $cwd = NULL, ?array $env_vars = NULL, ?array $options = NULL) {}

    function proc_close($process): int {}

    function proc_terminate($process, int $signal = 15): bool {}

    function proc_get_status($process): array {}

    function quoted_printable_decode(string $string): string {}

    function quoted_printable_encode(string $string): string {}

    function soundex(string $string): string {}

    function stream_select(?array &$read, ?array &$write, ?array &$except, ?int $seconds, ?int $microseconds = NULL): int|false {}

    function stream_context_create(?array $options = NULL, ?array $params = NULL) {}

    function stream_context_set_params($context, array $params): true {}

    function stream_context_get_params($context): array {}

    function stream_context_set_option($context, array|string $wrapper_or_options, ?string $option_name = NULL, mixed $value = null): true {}

    function stream_context_set_options($context, array $options): true {}

    function stream_context_get_options($stream_or_context): array {}

    function stream_context_get_default(?array $options = NULL) {}

    function stream_context_set_default(array $options) {}

    function stream_filter_prepend($stream, string $filter_name, int $mode = 0, mixed $params = null) {}

    function stream_filter_append($stream, string $filter_name, int $mode = 0, mixed $params = null) {}

    function stream_filter_remove($stream_filter): bool {}

    function stream_socket_client(string $address, &$error_code = NULL, &$error_message = NULL, ?float $timeout = NULL, int $flags = \STREAM_CLIENT_CONNECT, $context = NULL) {}

    function stream_socket_server(string $address, &$error_code = NULL, &$error_message = NULL, int $flags = 12, $context = NULL) {}

    function stream_socket_accept($socket, ?float $timeout = NULL, &$peer_name = NULL) {}

    function stream_socket_get_name($socket, bool $remote): string|false {}

    function stream_socket_recvfrom($socket, int $length, int $flags = 0, &$address = NULL): string|false {}

    function stream_socket_sendto($socket, string $data, int $flags = 0, string $address = ''): int|false {}

    function stream_socket_enable_crypto($stream, bool $enable, ?int $crypto_method = NULL, $session_stream = NULL): int|bool {}

    function stream_socket_shutdown($stream, int $mode): bool {}

    function stream_socket_pair(int $domain, int $type, int $protocol): array|false {}

    function stream_copy_to_stream($from, $to, ?int $length = NULL, int $offset = 0): int|false {}

    function stream_get_contents($stream, ?int $length = NULL, int $offset = -1): string|false {}

    function stream_supports_lock($stream): bool {}

    function stream_set_write_buffer($stream, int $size): int {}

    function set_file_buffer($stream, int $size): int {}

    function stream_set_read_buffer($stream, int $size): int {}

    function stream_set_blocking($stream, bool $enable): bool {}

    function socket_set_blocking($stream, bool $enable): bool {}

    function stream_get_meta_data($stream): array {}

    function socket_get_status($stream): array {}

    function stream_get_line($stream, int $length, string $ending = ''): string|false {}

    function stream_resolve_include_path(string $filename): string|false {}

    function stream_get_wrappers(): array {}

    function stream_get_transports(): array {}

    function stream_is_local($stream): bool {}

    function stream_isatty($stream): bool {}

    function stream_set_chunk_size($stream, int $size): int {}

    function stream_set_timeout($stream, int $seconds, int $microseconds = 0): bool {}

    function socket_set_timeout($stream, int $seconds, int $microseconds = 0): bool {}

    function gettype(mixed $value): string {}

    function get_debug_type(mixed $value): string {}

    function settype(mixed &$var, string $type): bool {}

    function intval(mixed $value, int $base = 10): int {}

    function floatval(mixed $value): float {}

    function doubleval(mixed $value): float {}

    function boolval(mixed $value): bool {}

    function strval(mixed $value): string {}

    function is_null(mixed $value): bool {}

    function is_resource(mixed $value): bool {}

    function is_bool(mixed $value): bool {}

    function is_int(mixed $value): bool {}

    function is_integer(mixed $value): bool {}

    function is_long(mixed $value): bool {}

    function is_float(mixed $value): bool {}

    function is_double(mixed $value): bool {}

    function is_numeric(mixed $value): bool {}

    function is_string(mixed $value): bool {}

    function is_array(mixed $value): bool {}

    function is_object(mixed $value): bool {}

    function is_scalar(mixed $value): bool {}

    function is_callable(mixed $value, bool $syntax_only = false, &$callable_name = NULL): bool {}

    function is_iterable(mixed $value): bool {}

    function is_countable(mixed $value): bool {}

    function uniqid(string $prefix = '', bool $more_entropy = false): string {}

    function parse_url(string $url, int $component = -1): array|string|int|false|null {}

    function urlencode(string $string): string {}

    function urldecode(string $string): string {}

    function rawurlencode(string $string): string {}

    function rawurldecode(string $string): string {}

    function get_headers(string $url, bool $associative = false, $context = NULL): array|false {}

    function stream_bucket_make_writeable($brigade): ?\StreamBucket {}

    function stream_bucket_prepend($brigade, \StreamBucket $bucket): void {}

    function stream_bucket_append($brigade, \StreamBucket $bucket): void {}

    function stream_bucket_new($stream, string $buffer): \StreamBucket {}

    function stream_get_filters(): array {}

    function stream_filter_register(string $filter_name, string $class): bool {}

    function convert_uuencode(string $string): string {}

    function convert_uudecode(string $string): string|false {}

    function var_dump(mixed $value, mixed ...$values): void {}

    function var_export(mixed $value, bool $return = false): ?string {}

    function debug_zval_dump(mixed $value, mixed ...$values): void {}

    function serialize(mixed $value): string {}

    function unserialize(string $data, array $options = array ()): mixed {}

    function memory_get_usage(bool $real_usage = false): int {}

    function memory_get_peak_usage(bool $real_usage = false): int {}

    function memory_reset_peak_usage(): void {}

    function version_compare(string $version1, string $version2, ?string $operator = NULL): int|bool {}

    function dl(string $extension_filename): bool {}

    function cli_set_process_title(string $title): bool {}

    function cli_get_process_title(): ?string {}

    final class __PHP_Incomplete_Class {
    }

    class AssertionError extends \Error implements \Throwable, \Stringable {
    }

    enum RoundingMode implements \UnitEnum {
        case HalfAwayFromZero;
        case HalfTowardsZero;
        case HalfEven;
        case HalfOdd;
        case TowardsZero;
        case AwayFromZero;
        case NegativeInfinity;
        case PositiveInfinity;
        public static function cases(): array {}
    }

    class php_user_filter {
        public  string $filtername = '';
        public  mixed $params = '';
        public  $stream = NULL;
        public function filter($in, $out, &$consumed, bool $closing) {}
        public function onCreate() {}
        public function onClose() {}
    }

    final class StreamBucket {
        public  $bucket = NULL;
        public  string $data;
        public  int $datalen;
        public  int $dataLength;
    }

    final class Directory {
        public  readonly protected(set) string $path;
        public  readonly protected(set) mixed $handle;
        public function close(): void {}
        public function rewind(): void {}
        public function read(): string|false {}
    }

    const EXTR_OVERWRITE = 0;

    const EXTR_SKIP = 1;

    const EXTR_PREFIX_SAME = 2;

    const EXTR_PREFIX_ALL = 3;

    const EXTR_PREFIX_INVALID = 4;

    const EXTR_PREFIX_IF_EXISTS = 5;

    const EXTR_IF_EXISTS = 6;

    const EXTR_REFS = 256;

    const SORT_ASC = 4;

    const SORT_DESC = 3;

    const SORT_REGULAR = 0;

    const SORT_NUMERIC = 1;

    const SORT_STRING = 2;

    const SORT_LOCALE_STRING = 5;

    const SORT_NATURAL = 6;

    const SORT_FLAG_CASE = 8;

    const CASE_LOWER = 0;

    const CASE_UPPER = 1;

    const COUNT_NORMAL = 0;

    const COUNT_RECURSIVE = 1;

    const ARRAY_FILTER_USE_BOTH = 1;

    const ARRAY_FILTER_USE_KEY = 2;

    const ASSERT_ACTIVE = 1;

    const ASSERT_CALLBACK = 2;

    const ASSERT_BAIL = 3;

    const ASSERT_WARNING = 4;

    const ASSERT_EXCEPTION = 5;

    const CONNECTION_ABORTED = 1;

    const CONNECTION_NORMAL = 0;

    const CONNECTION_TIMEOUT = 2;

    const INI_USER = 1;

    const INI_PERDIR = 2;

    const INI_SYSTEM = 4;

    const INI_ALL = 7;

    const INI_SCANNER_NORMAL = 0;

    const INI_SCANNER_RAW = 1;

    const INI_SCANNER_TYPED = 2;

    const PHP_URL_SCHEME = 0;

    const PHP_URL_HOST = 1;

    const PHP_URL_PORT = 2;

    const PHP_URL_USER = 3;

    const PHP_URL_PASS = 4;

    const PHP_URL_PATH = 5;

    const PHP_URL_QUERY = 6;

    const PHP_URL_FRAGMENT = 7;

    const PHP_QUERY_RFC1738 = 1;

    const PHP_QUERY_RFC3986 = 2;

    const M_E = 2.718281828459045;

    const M_LOG2E = 1.4426950408889634;

    const M_LOG10E = 0.4342944819032518;

    const M_LN2 = 0.6931471805599453;

    const M_LN10 = 2.302585092994046;

    const M_PI = 3.141592653589793;

    const M_PI_2 = 1.5707963267948966;

    const M_PI_4 = 0.7853981633974483;

    const M_1_PI = 0.3183098861837907;

    const M_2_PI = 0.6366197723675814;

    const M_SQRTPI = 1.772453850905516;

    const M_2_SQRTPI = 1.1283791670955126;

    const M_LNPI = 1.1447298858494002;

    const M_EULER = 0.5772156649015329;

    const M_SQRT2 = 1.4142135623730951;

    const M_SQRT1_2 = 0.7071067811865476;

    const M_SQRT3 = 1.7320508075688772;

    const INF = INF;

    const NAN = NAN;

    const PHP_ROUND_HALF_UP = 1;

    const PHP_ROUND_HALF_DOWN = 2;

    const PHP_ROUND_HALF_EVEN = 3;

    const PHP_ROUND_HALF_ODD = 4;

    const CRYPT_SALT_LENGTH = 123;

    const CRYPT_STD_DES = 1;

    const CRYPT_EXT_DES = 1;

    const CRYPT_MD5 = 1;

    const CRYPT_BLOWFISH = 1;

    const CRYPT_SHA256 = 1;

    const CRYPT_SHA512 = 1;

    const DNS_A = 1;

    const DNS_NS = 2;

    const DNS_CNAME = 16;

    const DNS_SOA = 32;

    const DNS_PTR = 2048;

    const DNS_HINFO = 4096;

    const DNS_CAA = 8192;

    const DNS_MX = 16384;

    const DNS_TXT = 32768;

    const DNS_SRV = 33554432;

    const DNS_NAPTR = 67108864;

    const DNS_AAAA = 134217728;

    const DNS_A6 = 16777216;

    const DNS_ANY = 268435456;

    const DNS_ALL = 251721779;

    const HTML_SPECIALCHARS = 0;

    const HTML_ENTITIES = 1;

    const ENT_COMPAT = 2;

    const ENT_QUOTES = 3;

    const ENT_NOQUOTES = 0;

    const ENT_IGNORE = 4;

    const ENT_SUBSTITUTE = 8;

    const ENT_DISALLOWED = 128;

    const ENT_HTML401 = 0;

    const ENT_XML1 = 16;

    const ENT_XHTML = 32;

    const ENT_HTML5 = 48;

    const IMAGETYPE_GIF = 1;

    const IMAGETYPE_JPEG = 2;

    const IMAGETYPE_PNG = 3;

    const IMAGETYPE_SWF = 4;

    const IMAGETYPE_PSD = 5;

    const IMAGETYPE_BMP = 6;

    const IMAGETYPE_TIFF_II = 7;

    const IMAGETYPE_TIFF_MM = 8;

    const IMAGETYPE_JPC = 9;

    const IMAGETYPE_JP2 = 10;

    const IMAGETYPE_JPX = 11;

    const IMAGETYPE_JB2 = 12;

    const IMAGETYPE_SWC = 13;

    const IMAGETYPE_IFF = 14;

    const IMAGETYPE_WBMP = 15;

    const IMAGETYPE_JPEG2000 = 9;

    const IMAGETYPE_XBM = 16;

    const IMAGETYPE_ICO = 17;

    const IMAGETYPE_WEBP = 18;

    const IMAGETYPE_AVIF = 19;

    const IMAGETYPE_HEIF = 20;

    const IMAGETYPE_UNKNOWN = 0;

    const IMAGETYPE_COUNT = 22;

    const INFO_GENERAL = 1;

    const INFO_CREDITS = 2;

    const INFO_CONFIGURATION = 4;

    const INFO_MODULES = 8;

    const INFO_ENVIRONMENT = 16;

    const INFO_VARIABLES = 32;

    const INFO_LICENSE = 64;

    const INFO_ALL = 4294967295;

    const CREDITS_GROUP = 1;

    const CREDITS_GENERAL = 2;

    const CREDITS_SAPI = 4;

    const CREDITS_MODULES = 8;

    const CREDITS_DOCS = 16;

    const CREDITS_FULLPAGE = 32;

    const CREDITS_QA = 64;

    const CREDITS_ALL = 4294967295;

    const LOG_EMERG = 0;

    const LOG_ALERT = 1;

    const LOG_CRIT = 2;

    const LOG_ERR = 3;

    const LOG_WARNING = 4;

    const LOG_NOTICE = 5;

    const LOG_INFO = 6;

    const LOG_DEBUG = 7;

    const LOG_KERN = 0;

    const LOG_USER = 8;

    const LOG_MAIL = 16;

    const LOG_DAEMON = 24;

    const LOG_AUTH = 32;

    const LOG_SYSLOG = 40;

    const LOG_LPR = 48;

    const LOG_NEWS = 56;

    const LOG_UUCP = 64;

    const LOG_CRON = 72;

    const LOG_AUTHPRIV = 80;

    const LOG_LOCAL0 = 128;

    const LOG_LOCAL1 = 136;

    const LOG_LOCAL2 = 144;

    const LOG_LOCAL3 = 152;

    const LOG_LOCAL4 = 160;

    const LOG_LOCAL5 = 168;

    const LOG_LOCAL6 = 176;

    const LOG_LOCAL7 = 184;

    const LOG_PID = 1;

    const LOG_CONS = 2;

    const LOG_ODELAY = 4;

    const LOG_NDELAY = 8;

    const LOG_NOWAIT = 16;

    const LOG_PERROR = 32;

    const STR_PAD_LEFT = 0;

    const STR_PAD_RIGHT = 1;

    const STR_PAD_BOTH = 2;

    const PATHINFO_DIRNAME = 1;

    const PATHINFO_BASENAME = 2;

    const PATHINFO_EXTENSION = 4;

    const PATHINFO_FILENAME = 8;

    const PATHINFO_ALL = 15;

    const CHAR_MAX = 127;

    const LC_CTYPE = 0;

    const LC_NUMERIC = 1;

    const LC_TIME = 2;

    const LC_COLLATE = 3;

    const LC_MONETARY = 4;

    const LC_ALL = 6;

    const LC_MESSAGES = 5;

    const ABDAY_1 = 131072;

    const ABDAY_2 = 131073;

    const ABDAY_3 = 131074;

    const ABDAY_4 = 131075;

    const ABDAY_5 = 131076;

    const ABDAY_6 = 131077;

    const ABDAY_7 = 131078;

    const DAY_1 = 131079;

    const DAY_2 = 131080;

    const DAY_3 = 131081;

    const DAY_4 = 131082;

    const DAY_5 = 131083;

    const DAY_6 = 131084;

    const DAY_7 = 131085;

    const ABMON_1 = 131086;

    const ABMON_2 = 131087;

    const ABMON_3 = 131088;

    const ABMON_4 = 131089;

    const ABMON_5 = 131090;

    const ABMON_6 = 131091;

    const ABMON_7 = 131092;

    const ABMON_8 = 131093;

    const ABMON_9 = 131094;

    const ABMON_10 = 131095;

    const ABMON_11 = 131096;

    const ABMON_12 = 131097;

    const MON_1 = 131098;

    const MON_2 = 131099;

    const MON_3 = 131100;

    const MON_4 = 131101;

    const MON_5 = 131102;

    const MON_6 = 131103;

    const MON_7 = 131104;

    const MON_8 = 131105;

    const MON_9 = 131106;

    const MON_10 = 131107;

    const MON_11 = 131108;

    const MON_12 = 131109;

    const AM_STR = 131110;

    const PM_STR = 131111;

    const D_T_FMT = 131112;

    const D_FMT = 131113;

    const T_FMT = 131114;

    const T_FMT_AMPM = 131115;

    const ERA = 131116;

    const ERA_YEAR = 131117;

    const ERA_D_T_FMT = 131120;

    const ERA_D_FMT = 131118;

    const ERA_T_FMT = 131121;

    const ALT_DIGITS = 131119;

    const INT_CURR_SYMBOL = 262144;

    const CURRENCY_SYMBOL = 262145;

    const CRNCYSTR = 262159;

    const MON_DECIMAL_POINT = 262146;

    const MON_THOUSANDS_SEP = 262147;

    const MON_GROUPING = 262148;

    const POSITIVE_SIGN = 262149;

    const NEGATIVE_SIGN = 262150;

    const INT_FRAC_DIGITS = 262151;

    const FRAC_DIGITS = 262152;

    const P_CS_PRECEDES = 262153;

    const P_SEP_BY_SPACE = 262154;

    const N_CS_PRECEDES = 262155;

    const N_SEP_BY_SPACE = 262156;

    const P_SIGN_POSN = 262157;

    const N_SIGN_POSN = 262158;

    const DECIMAL_POINT = 65536;

    const RADIXCHAR = 65536;

    const THOUSANDS_SEP = 65537;

    const THOUSEP = 65537;

    const GROUPING = 65538;

    const YESEXPR = 327680;

    const NOEXPR = 327681;

    const YESSTR = 327682;

    const NOSTR = 327683;

    const CODESET = 14;

    const SEEK_SET = 0;

    const SEEK_CUR = 1;

    const SEEK_END = 2;

    const LOCK_SH = 1;

    const LOCK_EX = 2;

    const LOCK_UN = 3;

    const LOCK_NB = 4;

    const STREAM_NOTIFY_CONNECT = 2;

    const STREAM_NOTIFY_AUTH_REQUIRED = 3;

    const STREAM_NOTIFY_AUTH_RESULT = 10;

    const STREAM_NOTIFY_MIME_TYPE_IS = 4;

    const STREAM_NOTIFY_FILE_SIZE_IS = 5;

    const STREAM_NOTIFY_REDIRECTED = 6;

    const STREAM_NOTIFY_PROGRESS = 7;

    const STREAM_NOTIFY_FAILURE = 9;

    const STREAM_NOTIFY_COMPLETED = 8;

    const STREAM_NOTIFY_RESOLVE = 1;

    const STREAM_NOTIFY_SEVERITY_INFO = 0;

    const STREAM_NOTIFY_SEVERITY_WARN = 1;

    const STREAM_NOTIFY_SEVERITY_ERR = 2;

    const STREAM_FILTER_READ = 1;

    const STREAM_FILTER_WRITE = 2;

    const STREAM_FILTER_ALL = 3;

    const STREAM_CLIENT_PERSISTENT = 1;

    const STREAM_CLIENT_ASYNC_CONNECT = 2;

    const STREAM_CLIENT_CONNECT = 4;

    const STREAM_CRYPTO_METHOD_ANY_CLIENT = 127;

    const STREAM_CRYPTO_METHOD_SSLv2_CLIENT = 3;

    const STREAM_CRYPTO_METHOD_SSLv3_CLIENT = 5;

    const STREAM_CRYPTO_METHOD_SSLv23_CLIENT = 57;

    const STREAM_CRYPTO_METHOD_TLS_CLIENT = 121;

    const STREAM_CRYPTO_METHOD_TLSv1_0_CLIENT = 9;

    const STREAM_CRYPTO_METHOD_TLSv1_1_CLIENT = 17;

    const STREAM_CRYPTO_METHOD_TLSv1_2_CLIENT = 33;

    const STREAM_CRYPTO_METHOD_TLSv1_3_CLIENT = 65;

    const STREAM_CRYPTO_METHOD_ANY_SERVER = 126;

    const STREAM_CRYPTO_METHOD_SSLv2_SERVER = 2;

    const STREAM_CRYPTO_METHOD_SSLv3_SERVER = 4;

    const STREAM_CRYPTO_METHOD_SSLv23_SERVER = 120;

    const STREAM_CRYPTO_METHOD_TLS_SERVER = 120;

    const STREAM_CRYPTO_METHOD_TLSv1_0_SERVER = 8;

    const STREAM_CRYPTO_METHOD_TLSv1_1_SERVER = 16;

    const STREAM_CRYPTO_METHOD_TLSv1_2_SERVER = 32;

    const STREAM_CRYPTO_METHOD_TLSv1_3_SERVER = 64;

    const STREAM_CRYPTO_PROTO_SSLv3 = 4;

    const STREAM_CRYPTO_PROTO_TLSv1_0 = 8;

    const STREAM_CRYPTO_PROTO_TLSv1_1 = 16;

    const STREAM_CRYPTO_PROTO_TLSv1_2 = 32;

    const STREAM_CRYPTO_PROTO_TLSv1_3 = 64;

    const STREAM_SHUT_RD = 0;

    const STREAM_SHUT_WR = 1;

    const STREAM_SHUT_RDWR = 2;

    const STREAM_PF_INET = 2;

    const STREAM_PF_INET6 = 10;

    const STREAM_PF_UNIX = 1;

    const STREAM_IPPROTO_IP = 0;

    const STREAM_IPPROTO_TCP = 6;

    const STREAM_IPPROTO_UDP = 17;

    const STREAM_IPPROTO_ICMP = 1;

    const STREAM_IPPROTO_RAW = 255;

    const STREAM_SOCK_STREAM = 1;

    const STREAM_SOCK_DGRAM = 2;

    const STREAM_SOCK_RAW = 3;

    const STREAM_SOCK_SEQPACKET = 5;

    const STREAM_SOCK_RDM = 4;

    const STREAM_PEEK = 2;

    const STREAM_OOB = 1;

    const STREAM_SERVER_BIND = 4;

    const STREAM_SERVER_LISTEN = 8;

    const FILE_USE_INCLUDE_PATH = 1;

    const FILE_IGNORE_NEW_LINES = 2;

    const FILE_SKIP_EMPTY_LINES = 4;

    const FILE_APPEND = 8;

    const FILE_NO_DEFAULT_CONTEXT = 16;

    const FILE_TEXT = 0;

    const FILE_BINARY = 0;

    const FNM_NOESCAPE = 2;

    const FNM_PATHNAME = 1;

    const FNM_PERIOD = 4;

    const FNM_CASEFOLD = 16;

    const PSFS_PASS_ON = 2;

    const PSFS_FEED_ME = 1;

    const PSFS_ERR_FATAL = 0;

    const PSFS_FLAG_NORMAL = 0;

    const PSFS_FLAG_FLUSH_INC = 1;

    const PSFS_FLAG_FLUSH_CLOSE = 2;

    const PASSWORD_DEFAULT = '2y';

    const PASSWORD_BCRYPT = '2y';

    const PASSWORD_BCRYPT_DEFAULT_COST = 12;

    const PASSWORD_ARGON2I = 'argon2i';

    const PASSWORD_ARGON2ID = 'argon2id';

    const PASSWORD_ARGON2_PROVIDER = 'standard';

    const PASSWORD_ARGON2_DEFAULT_MEMORY_COST = 65536;

    const PASSWORD_ARGON2_DEFAULT_TIME_COST = 4;

    const PASSWORD_ARGON2_DEFAULT_THREADS = 1;

    const DIRECTORY_SEPARATOR = '/';

    const PATH_SEPARATOR = ':';

    const GLOB_BRACE = 128;

    const GLOB_ERR = 4;

    const GLOB_MARK = 8;

    const GLOB_NOCHECK = 16;

    const GLOB_NOESCAPE = 4096;

    const GLOB_NOSORT = 32;

    const GLOB_ONLYDIR = 1073741824;

    const GLOB_AVAILABLE_FLAGS = 1073746108;

    const SCANDIR_SORT_ASCENDING = 0;

    const SCANDIR_SORT_DESCENDING = 1;

    const SCANDIR_SORT_NONE = 2;

    const STREAM_USE_PATH = 1;

    const STREAM_IGNORE_URL = 2;

    const STREAM_REPORT_ERRORS = 8;

    const STREAM_MUST_SEEK = 16;

    const STREAM_URL_STAT_LINK = 1;

    const STREAM_URL_STAT_QUIET = 2;

    const STREAM_MKDIR_RECURSIVE = 1;

    const STREAM_IS_URL = 1;

    const STREAM_OPTION_BLOCKING = 1;

    const STREAM_OPTION_READ_TIMEOUT = 4;

    const STREAM_OPTION_READ_BUFFER = 2;

    const STREAM_OPTION_WRITE_BUFFER = 3;

    const STREAM_BUFFER_NONE = 0;

    const STREAM_BUFFER_LINE = 1;

    const STREAM_BUFFER_FULL = 2;

    const STREAM_CAST_AS_STREAM = 0;

    const STREAM_CAST_FOR_SELECT = 3;

    const STREAM_META_TOUCH = 1;

    const STREAM_META_OWNER = 3;

    const STREAM_META_OWNER_NAME = 2;

    const STREAM_META_GROUP = 5;

    const STREAM_META_GROUP_NAME = 4;

    const STREAM_META_ACCESS = 6;

    const IMAGETYPE_SVG = 21;

}
