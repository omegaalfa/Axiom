<?php
// Runtime declarations exported from extension: FFI

namespace {
    final class FFI {
        public const __BIGGEST_ALIGNMENT__ = 16;
        public static function cdef(string $code = '', ?string $lib = NULL): \FFI {}
        public static function load(string $filename): ?\FFI {}
        public static function scope(string $name): \FFI {}
        public static function new(\FFI\CType|string $type, bool $owned = true, bool $persistent = false): \FFI\CData {}
        public static function free(\FFI\CData &$ptr): void {}
        public static function cast(\FFI\CType|string $type, &$ptr): \FFI\CData {}
        public static function type(string $type): \FFI\CType {}
        public static function typeof(\FFI\CData &$ptr): \FFI\CType {}
        public static function arrayType(\FFI\CType $type, array $dimensions): \FFI\CType {}
        public static function addr(\FFI\CData &$ptr): \FFI\CData {}
        public static function sizeof(\FFI\CData|\FFI\CType &$ptr): int {}
        public static function alignof(\FFI\CData|\FFI\CType &$ptr): int {}
        public static function memcpy(\FFI\CData &$to, &$from, int $size): void {}
        public static function memcmp(&$ptr1, &$ptr2, int $size): int {}
        public static function memset(\FFI\CData &$ptr, int $value, int $size): void {}
        public static function string(\FFI\CData &$ptr, ?int $size = NULL): string {}
        public static function isNull(\FFI\CData &$ptr): bool {}
    }

}

namespace FFI {
    class Exception extends \Error implements \Throwable, \Stringable {
    }

    final class ParserException extends \FFI\Exception implements \Stringable, \Throwable {
    }

    final class CData {
    }

    final class CType {
        public const TYPE_VOID = 0;
        public const TYPE_FLOAT = 1;
        public const TYPE_DOUBLE = 2;
        public const TYPE_LONGDOUBLE = 3;
        public const TYPE_UINT8 = 4;
        public const TYPE_SINT8 = 5;
        public const TYPE_UINT16 = 6;
        public const TYPE_SINT16 = 7;
        public const TYPE_UINT32 = 8;
        public const TYPE_SINT32 = 9;
        public const TYPE_UINT64 = 10;
        public const TYPE_SINT64 = 11;
        public const TYPE_ENUM = 12;
        public const TYPE_BOOL = 13;
        public const TYPE_CHAR = 14;
        public const TYPE_POINTER = 15;
        public const TYPE_FUNC = 16;
        public const TYPE_ARRAY = 17;
        public const TYPE_STRUCT = 18;
        public const ATTR_CONST = 1;
        public const ATTR_INCOMPLETE_TAG = 2;
        public const ATTR_VARIADIC = 4;
        public const ATTR_INCOMPLETE_ARRAY = 8;
        public const ATTR_VLA = 16;
        public const ATTR_UNION = 32;
        public const ATTR_PACKED = 64;
        public const ATTR_MS_STRUCT = 128;
        public const ATTR_GCC_STRUCT = 256;
        public const ABI_DEFAULT = 0;
        public const ABI_CDECL = 1;
        public const ABI_FASTCALL = 2;
        public const ABI_THISCALL = 3;
        public const ABI_STDCALL = 4;
        public const ABI_PASCAL = 5;
        public const ABI_REGISTER = 6;
        public const ABI_MS = 7;
        public const ABI_SYSV = 8;
        public const ABI_VECTORCALL = 9;
        public function getName(): string {}
        public function getKind(): int {}
        public function getSize(): int {}
        public function getAlignment(): int {}
        public function getAttributes(): int {}
        public function getEnumKind(): int {}
        public function getArrayElementType(): \FFI\CType {}
        public function getArrayLength(): int {}
        public function getPointerType(): \FFI\CType {}
        public function getStructFieldNames(): array {}
        public function getStructFieldOffset(string $name): int {}
        public function getStructFieldType(string $name): \FFI\CType {}
        public function getFuncABI(): int {}
        public function getFuncReturnType(): \FFI\CType {}
        public function getFuncParameterCount(): int {}
        public function getFuncParameterType(int $index): \FFI\CType {}
    }

}
