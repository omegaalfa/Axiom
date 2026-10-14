<?php
// Runtime declarations exported from extension: libxml

namespace {
    function libxml_set_streams_context($context): void {}

    function libxml_use_internal_errors(?bool $use_errors = NULL): bool {}

    function libxml_get_last_error(): \LibXMLError|false {}

    function libxml_get_errors(): array {}

    function libxml_clear_errors(): void {}

    function libxml_disable_entity_loader(bool $disable = true): bool {}

    function libxml_set_external_entity_loader(?callable $resolver_function): true {}

    function libxml_get_external_entity_loader(): ?callable {}

    class LibXMLError {
        public  int $level;
        public  int $code;
        public  int $column;
        public  string $message;
        public  string $file;
        public  int $line;
    }

    const LIBXML_VERSION = 21502;

    const LIBXML_DOTTED_VERSION = '2.15.2';

    const LIBXML_LOADED_VERSION = '21502';

    const LIBXML_RECOVER = 1;

    const LIBXML_NOENT = 2;

    const LIBXML_NO_XXE = 8388608;

    const LIBXML_DTDLOAD = 4;

    const LIBXML_DTDATTR = 8;

    const LIBXML_DTDVALID = 16;

    const LIBXML_NOERROR = 32;

    const LIBXML_NOWARNING = 64;

    const LIBXML_NOBLANKS = 256;

    const LIBXML_XINCLUDE = 1024;

    const LIBXML_NSCLEAN = 8192;

    const LIBXML_NOCDATA = 16384;

    const LIBXML_NONET = 2048;

    const LIBXML_PEDANTIC = 128;

    const LIBXML_COMPACT = 65536;

    const LIBXML_NOXMLDECL = 2;

    const LIBXML_PARSEHUGE = 524288;

    const LIBXML_BIGLINES = 4194304;

    const LIBXML_NOEMPTYTAG = 4;

    const LIBXML_SCHEMA_CREATE = 1;

    const LIBXML_HTML_NOIMPLIED = 8192;

    const LIBXML_HTML_NODEFDTD = 4;

    const LIBXML_ERR_NONE = 0;

    const LIBXML_ERR_WARNING = 1;

    const LIBXML_ERR_ERROR = 2;

    const LIBXML_ERR_FATAL = 3;

}
