<?php
// Runtime declarations exported from extension: fileinfo

namespace {
    function finfo_open(int $flags = \FILEINFO_NONE, ?string $magic_database = NULL): \finfo|false {}

    function finfo_close(\finfo $finfo): true {}

    function finfo_set_flags(\finfo $finfo, int $flags): true {}

    function finfo_file(\finfo $finfo, string $filename, int $flags = \FILEINFO_NONE, $context = NULL): string|false {}

    function finfo_buffer(\finfo $finfo, string $string, int $flags = \FILEINFO_NONE, $context = NULL): string|false {}

    function mime_content_type($filename): string|false {}

    class finfo {
        public function __construct(int $flags = \FILEINFO_NONE, ?string $magic_database = NULL) {}
        public function file(string $filename, int $flags = \FILEINFO_NONE, $context = NULL) {}
        public function buffer(string $string, int $flags = \FILEINFO_NONE, $context = NULL) {}
        public function set_flags(int $flags) {}
    }

    const FILEINFO_NONE = 0;

    const FILEINFO_SYMLINK = 2;

    const FILEINFO_MIME = 1040;

    const FILEINFO_MIME_TYPE = 16;

    const FILEINFO_MIME_ENCODING = 1024;

    const FILEINFO_DEVICES = 8;

    const FILEINFO_CONTINUE = 32;

    const FILEINFO_PRESERVE_ATIME = 128;

    const FILEINFO_RAW = 256;

    const FILEINFO_APPLE = 2048;

    const FILEINFO_EXTENSION = 16777216;

}
