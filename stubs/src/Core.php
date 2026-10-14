<?php
// Runtime declarations exported from extension: Core

namespace {
    function clone(object $object, array $withProperties = array ()): object {}

    function exit(string|int $status = 0): never {}

    function die(string|int $status = 0): never {}

    function zend_version(): string {}

    function func_num_args(): int {}

    function func_get_arg(int $position): mixed {}

    function func_get_args(): array {}

    function strlen(string $string): int {}

    function strcmp(string $string1, string $string2): int {}

    function strncmp(string $string1, string $string2, int $length): int {}

    function strcasecmp(string $string1, string $string2): int {}

    function strncasecmp(string $string1, string $string2, int $length): int {}

    function error_reporting(?int $error_level = NULL): int {}

    function define(string $constant_name, mixed $value, bool $case_insensitive = false): bool {}

    function defined(string $constant_name): bool {}

    function get_class(object $object = null): string {}

    function get_called_class(): string {}

    function get_parent_class(object|string $object_or_class = null): string|false {}

    function is_subclass_of(mixed $object_or_class, string $class, bool $allow_string = true): bool {}

    function is_a(mixed $object_or_class, string $class, bool $allow_string = false): bool {}

    function get_class_vars(string $class): array {}

    function get_object_vars(object $object): array {}

    function get_mangled_object_vars(object $object): array {}

    function get_class_methods(object|string $object_or_class): array {}

    function method_exists($object_or_class, string $method): bool {}

    function property_exists($object_or_class, string $property): bool {}

    function class_exists(string $class, bool $autoload = true): bool {}

    function interface_exists(string $interface, bool $autoload = true): bool {}

    function trait_exists(string $trait, bool $autoload = true): bool {}

    function enum_exists(string $enum, bool $autoload = true): bool {}

    function function_exists(string $function): bool {}

    function class_alias(string $class, string $alias, bool $autoload = true): bool {}

    function get_included_files(): array {}

    function get_required_files(): array {}

    function trigger_error(string $message, int $error_level = \E_USER_NOTICE): true {}

    function user_error(string $message, int $error_level = \E_USER_NOTICE): true {}

    function set_error_handler(?callable $callback, int $error_levels = \E_ALL) {}

    function restore_error_handler(): true {}

    function get_error_handler(): ?callable {}

    function set_exception_handler(?callable $callback) {}

    function restore_exception_handler(): true {}

    function get_exception_handler(): ?callable {}

    function get_declared_classes(): array {}

    function get_declared_traits(): array {}

    function get_declared_interfaces(): array {}

    function get_defined_functions(bool $exclude_disabled = true): array {}

    function get_defined_vars(): array {}

    function get_resource_type($resource): string {}

    function get_resource_id($resource): int {}

    function get_resources(?string $type = NULL): array {}

    function get_loaded_extensions(bool $zend_extensions = false): array {}

    function get_defined_constants(bool $categorize = false): array {}

    function debug_backtrace(int $options = \DEBUG_BACKTRACE_PROVIDE_OBJECT, int $limit = 0): array {}

    function debug_print_backtrace(int $options = 0, int $limit = 0): void {}

    function extension_loaded(string $extension): bool {}

    function get_extension_funcs(string $extension): array|false {}

    function gc_mem_caches(): int {}

    function gc_collect_cycles(): int {}

    function gc_enabled(): bool {}

    function gc_enable(): void {}

    function gc_disable(): void {}

    function gc_status(): array {}

    interface Traversable {
    }

    interface IteratorAggregate extends \Traversable {
        public function getIterator();
    }

    interface Iterator extends \Traversable {
        public function current();
        public function next();
        public function key();
        public function valid();
        public function rewind();
    }

    interface Serializable {
        public function serialize();
        public function unserialize(string $data);
    }

    interface ArrayAccess {
        public function offsetExists(mixed $offset);
        public function offsetGet(mixed $offset);
        public function offsetSet(mixed $offset, mixed $value);
        public function offsetUnset(mixed $offset);
    }

    interface Countable {
        public function count();
    }

    interface Stringable {
        public function __toString(): string;
    }

    final class InternalIterator implements \Iterator, \Traversable {
        private function __construct() {}
        public function current(): mixed {}
        public function key(): mixed {}
        public function next(): void {}
        public function valid(): bool {}
        public function rewind(): void {}
    }

    interface Throwable extends \Stringable {
        public function getMessage(): string;
        public function getCode();
        public function getFile(): string;
        public function getLine(): int;
        public function getTrace(): array;
        public function getPrevious(): ?\Throwable;
        public function getTraceAsString(): string;
    }

    class Exception implements \Stringable, \Throwable {
        protected  $message = '';
        private  string $string = '';
        protected  $code = 0;
        protected  string $file = '';
        protected  int $line = 0;
        private  array $trace = array ();
        private  ?\Throwable $previous = NULL;
        private function __clone(): void {}
        public function __construct(string $message = '', int $code = 0, ?\Throwable $previous = NULL) {}
        public function __wakeup() {}
        public final function getMessage(): string {}
        public final function getCode() {}
        public final function getFile(): string {}
        public final function getLine(): int {}
        public final function getTrace(): array {}
        public final function getPrevious(): ?\Throwable {}
        public final function getTraceAsString(): string {}
        public function __toString(): string {}
    }

    class ErrorException extends \Exception implements \Throwable, \Stringable {
        protected  int $severity = 1;
        public function __construct(string $message = '', int $code = 0, int $severity = \E_ERROR, ?string $filename = NULL, ?int $line = NULL, ?\Throwable $previous = NULL) {}
        public final function getSeverity(): int {}
    }

    class Error implements \Stringable, \Throwable {
        protected  $message = '';
        private  string $string = '';
        protected  $code = 0;
        protected  string $file = '';
        protected  int $line;
        private  array $trace = array ();
        private  ?\Throwable $previous = NULL;
        private function __clone(): void {}
        public function __construct(string $message = '', int $code = 0, ?\Throwable $previous = NULL) {}
        public function __wakeup() {}
        public final function getMessage(): string {}
        public final function getCode() {}
        public final function getFile(): string {}
        public final function getLine(): int {}
        public final function getTrace(): array {}
        public final function getPrevious(): ?\Throwable {}
        public final function getTraceAsString(): string {}
        public function __toString(): string {}
    }

    class CompileError extends \Error implements \Throwable, \Stringable {
    }

    class ParseError extends \CompileError implements \Stringable, \Throwable {
    }

    class TypeError extends \Error implements \Throwable, \Stringable {
    }

    class ArgumentCountError extends \TypeError implements \Stringable, \Throwable {
    }

    class ValueError extends \Error implements \Throwable, \Stringable {
    }

    class ArithmeticError extends \Error implements \Throwable, \Stringable {
    }

    class DivisionByZeroError extends \ArithmeticError implements \Stringable, \Throwable {
    }

    class UnhandledMatchError extends \Error implements \Throwable, \Stringable {
    }

    class RequestParseBodyException extends \Exception implements \Throwable, \Stringable {
    }

    final class Closure {
        private function __construct() {}
        public static function bind(\Closure $closure, ?object $newThis, object|string|null $newScope = 'static'): ?\Closure {}
        public function bindTo(?object $newThis, object|string|null $newScope = 'static'): ?\Closure {}
        public function call(object $newThis, mixed ...$args): mixed {}
        public static function fromCallable(callable $callback): \Closure {}
        public static function getCurrent(): \Closure {}
        public function __invoke() {}
    }

    final class Generator implements \Iterator, \Traversable {
        public function rewind(): void {}
        public function valid(): bool {}
        public function current(): mixed {}
        public function key(): mixed {}
        public function next(): void {}
        public function send(mixed $value): mixed {}
        public function throw(\Throwable $exception): mixed {}
        public function getReturn(): mixed {}
        public function __debugInfo(): array {}
    }

    class ClosedGeneratorException extends \Exception implements \Throwable, \Stringable {
    }

    final class WeakReference {
        public function __construct() {}
        public static function create(object $object): \WeakReference {}
        public function get(): ?object {}
    }

    final class WeakMap implements \ArrayAccess, \Countable, \IteratorAggregate, \Traversable {
        public function offsetGet($object): mixed {}
        public function offsetSet($object, mixed $value): void {}
        public function offsetExists($object): bool {}
        public function offsetUnset($object): void {}
        public function count(): int {}
        public function getIterator(): \Iterator {}
    }

    final class Attribute {
        public const TARGET_CLASS = 1;
        public const TARGET_FUNCTION = 2;
        public const TARGET_METHOD = 4;
        public const TARGET_PROPERTY = 8;
        public const TARGET_CLASS_CONSTANT = 16;
        public const TARGET_PARAMETER = 32;
        public const TARGET_CONSTANT = 64;
        public const TARGET_ALL = 127;
        public const IS_REPEATABLE = 128;
        public  int $flags;
        public function __construct(int $flags = \Attribute::TARGET_ALL) {}
    }

    final class ReturnTypeWillChange {
        public function __construct() {}
    }

    final class AllowDynamicProperties {
        public function __construct() {}
    }

    final class SensitiveParameter {
        public function __construct() {}
    }

    final class SensitiveParameterValue {
        private  readonly mixed $value;
        public function __construct(mixed $value) {}
        public function getValue(): mixed {}
        public function __debugInfo(): array {}
    }

    final class Override {
        public function __construct() {}
    }

    final class Deprecated {
        public  readonly protected(set) ?string $message;
        public  readonly protected(set) ?string $since;
        public function __construct(?string $message = NULL, ?string $since = NULL) {}
    }

    final class NoDiscard {
        public  readonly protected(set) ?string $message;
        public function __construct(?string $message = NULL) {}
    }

    final class DelayedTargetValidation {
    }

    interface UnitEnum {
        public static function cases(): array;
    }

    interface BackedEnum extends \UnitEnum {
        public static function from(string|int $value): static;
        public static function tryFrom(string|int $value): ?static;
    }

    final class Fiber {
        public function __construct(callable $callback) {}
        public function start(mixed ...$args): mixed {}
        public function resume(mixed $value = NULL): mixed {}
        public function throw(\Throwable $exception): mixed {}
        public function isStarted(): bool {}
        public function isSuspended(): bool {}
        public function isRunning(): bool {}
        public function isTerminated(): bool {}
        public function getReturn(): mixed {}
        public static function getCurrent(): ?\Fiber {}
        public static function suspend(mixed $value = NULL): mixed {}
    }

    final class FiberError extends \Error implements \Throwable, \Stringable {
        public function __construct() {}
    }

    class stdClass {
    }

    const E_ERROR = 1;

    const E_WARNING = 2;

    const E_PARSE = 4;

    const E_NOTICE = 8;

    const E_CORE_ERROR = 16;

    const E_CORE_WARNING = 32;

    const E_COMPILE_ERROR = 64;

    const E_COMPILE_WARNING = 128;

    const E_USER_ERROR = 256;

    const E_USER_WARNING = 512;

    const E_USER_NOTICE = 1024;

    const E_STRICT = 2048;

    const E_RECOVERABLE_ERROR = 4096;

    const E_DEPRECATED = 8192;

    const E_USER_DEPRECATED = 16384;

    const E_ALL = 30719;

    const DEBUG_BACKTRACE_PROVIDE_OBJECT = 1;

    const DEBUG_BACKTRACE_IGNORE_ARGS = 2;

    const ZEND_THREAD_SAFE = false;

    const ZEND_DEBUG_BUILD = false;

    const ZEND_VM_KIND = 'ZEND_VM_KIND_HYBRID';

    const TRUE = true;

    const FALSE = false;

    const NULL = NULL;

    const PHP_VERSION = '8.5.4';

    const PHP_MAJOR_VERSION = 8;

    const PHP_MINOR_VERSION = 5;

    const PHP_RELEASE_VERSION = 4;

    const PHP_EXTRA_VERSION = '';

    const PHP_VERSION_ID = 80504;

    const PHP_BUILD_DATE = 'Sep  2 2026 14:42:18';

    const PHP_BUILD_PROVIDER = 'Ubuntu';

    const PHP_ZTS = false;

    const PHP_DEBUG = false;

    const PHP_OS = 'Linux';

    const PHP_OS_FAMILY = 'Linux';

    const DEFAULT_INCLUDE_PATH = '.:/usr/share/php';

    const PEAR_INSTALL_DIR = '/usr/share/php';

    const PEAR_EXTENSION_DIR = '/usr/lib/php/20250925';

    const PHP_EXTENSION_DIR = '/usr/lib/php/20250925';

    const PHP_PREFIX = '/usr';

    const PHP_BINDIR = '/usr/bin';

    const PHP_SBINDIR = '/usr/sbin';

    const PHP_MANDIR = '/usr/share/man';

    const PHP_LIBDIR = '/usr/lib/php';

    const PHP_DATADIR = '/usr/share/php/8.5';

    const PHP_SYSCONFDIR = '/etc';

    const PHP_LOCALSTATEDIR = '/var';

    const PHP_CONFIG_FILE_PATH = '/etc/php/8.5/cli';

    const PHP_CONFIG_FILE_SCAN_DIR = '/etc/php/8.5/cli/conf.d';

    const PHP_SHLIB_SUFFIX = 'so';

    const PHP_EOL = '
    ';

    const PHP_MAXPATHLEN = 4096;

    const PHP_INT_MAX = 9223372036854775807;

    const PHP_INT_MIN = -9223372036854775807-1;

    const PHP_INT_SIZE = 8;

    const PHP_FD_SETSIZE = 1024;

    const PHP_FLOAT_DIG = 15;

    const PHP_FLOAT_EPSILON = 2.220446049250313E-16;

    const PHP_FLOAT_MAX = 1.7976931348623157E+308;

    const PHP_FLOAT_MIN = 2.2250738585072014E-308;

    const PHP_OUTPUT_HANDLER_START = 1;

    const PHP_OUTPUT_HANDLER_WRITE = 0;

    const PHP_OUTPUT_HANDLER_FLUSH = 4;

    const PHP_OUTPUT_HANDLER_CLEAN = 2;

    const PHP_OUTPUT_HANDLER_FINAL = 8;

    const PHP_OUTPUT_HANDLER_CONT = 0;

    const PHP_OUTPUT_HANDLER_END = 8;

    const PHP_OUTPUT_HANDLER_CLEANABLE = 16;

    const PHP_OUTPUT_HANDLER_FLUSHABLE = 32;

    const PHP_OUTPUT_HANDLER_REMOVABLE = 64;

    const PHP_OUTPUT_HANDLER_STDFLAGS = 112;

    const PHP_OUTPUT_HANDLER_STARTED = 4096;

    const PHP_OUTPUT_HANDLER_DISABLED = 8192;

    const PHP_OUTPUT_HANDLER_PROCESSED = 16384;

    const UPLOAD_ERR_OK = 0;

    const UPLOAD_ERR_INI_SIZE = 1;

    const UPLOAD_ERR_FORM_SIZE = 2;

    const UPLOAD_ERR_PARTIAL = 3;

    const UPLOAD_ERR_NO_FILE = 4;

    const UPLOAD_ERR_NO_TMP_DIR = 6;

    const UPLOAD_ERR_CANT_WRITE = 7;

    const UPLOAD_ERR_EXTENSION = 8;

    const PHP_SAPI = 'cli';

    const PHP_BINARY = '/usr/bin/php8.5';

    const PHP_CLI_PROCESS_TITLE = true;

}
