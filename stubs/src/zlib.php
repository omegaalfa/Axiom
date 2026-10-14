<?php
// Runtime declarations exported from extension: zlib

namespace {
    function ob_gzhandler(string $data, int $flags): string|false {}

    function zlib_get_coding_type(): string|false {}

    function gzfile(string $filename, bool $use_include_path = false): array|false {}

    function gzopen(string $filename, string $mode, bool $use_include_path = false) {}

    function readgzfile(string $filename, bool $use_include_path = false): int|false {}

    function zlib_encode(string $data, int $encoding, int $level = -1): string|false {}

    function zlib_decode(string $data, int $max_length = 0): string|false {}

    function gzdeflate(string $data, int $level = -1, int $encoding = \ZLIB_ENCODING_RAW): string|false {}

    function gzencode(string $data, int $level = -1, int $encoding = \ZLIB_ENCODING_GZIP): string|false {}

    function gzcompress(string $data, int $level = -1, int $encoding = \ZLIB_ENCODING_DEFLATE): string|false {}

    function gzinflate(string $data, int $max_length = 0): string|false {}

    function gzdecode(string $data, int $max_length = 0): string|false {}

    function gzuncompress(string $data, int $max_length = 0): string|false {}

    function gzwrite($stream, string $data, ?int $length = NULL): int|false {}

    function gzputs($stream, string $data, ?int $length = NULL): int|false {}

    function gzrewind($stream): bool {}

    function gzclose($stream): bool {}

    function gzeof($stream): bool {}

    function gzgetc($stream): string|false {}

    function gzpassthru($stream): int {}

    function gzseek($stream, int $offset, int $whence = \SEEK_SET): int {}

    function gztell($stream): int|false {}

    function gzread($stream, int $length): string|false {}

    function gzgets($stream, ?int $length = NULL): string|false {}

    function deflate_init(int $encoding, object|array $options = array ()): \DeflateContext|false {}

    function deflate_add(\DeflateContext $context, string $data, int $flush_mode = \ZLIB_SYNC_FLUSH): string|false {}

    function inflate_init(int $encoding, object|array $options = array ()): \InflateContext|false {}

    function inflate_add(\InflateContext $context, string $data, int $flush_mode = \ZLIB_SYNC_FLUSH): string|false {}

    function inflate_get_status(\InflateContext $context): int {}

    function inflate_get_read_len(\InflateContext $context): int {}

    final class InflateContext {
    }

    final class DeflateContext {
    }

    const FORCE_GZIP = 31;

    const FORCE_DEFLATE = 15;

    const ZLIB_ENCODING_RAW = -15;

    const ZLIB_ENCODING_GZIP = 31;

    const ZLIB_ENCODING_DEFLATE = 15;

    const ZLIB_NO_FLUSH = 0;

    const ZLIB_PARTIAL_FLUSH = 1;

    const ZLIB_SYNC_FLUSH = 2;

    const ZLIB_FULL_FLUSH = 3;

    const ZLIB_BLOCK = 5;

    const ZLIB_FINISH = 4;

    const ZLIB_FILTERED = 1;

    const ZLIB_HUFFMAN_ONLY = 2;

    const ZLIB_RLE = 3;

    const ZLIB_FIXED = 4;

    const ZLIB_DEFAULT_STRATEGY = 0;

    const ZLIB_VERSION = '1.3.1';

    const ZLIB_VERNUM = 4880;

    const ZLIB_OK = 0;

    const ZLIB_STREAM_END = 1;

    const ZLIB_NEED_DICT = 2;

    const ZLIB_ERRNO = -1;

    const ZLIB_STREAM_ERROR = -2;

    const ZLIB_DATA_ERROR = -3;

    const ZLIB_MEM_ERROR = -4;

    const ZLIB_BUF_ERROR = -5;

    const ZLIB_VERSION_ERROR = -6;

}
