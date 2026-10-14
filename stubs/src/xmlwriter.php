<?php
// Runtime declarations exported from extension: xmlwriter

namespace {
    function xmlwriter_open_uri(string $uri): \XMLWriter|false {}

    function xmlwriter_open_memory(): \XMLWriter|false {}

    function xmlwriter_set_indent(\XMLWriter $writer, bool $enable): bool {}

    function xmlwriter_set_indent_string(\XMLWriter $writer, string $indentation): bool {}

    function xmlwriter_start_comment(\XMLWriter $writer): bool {}

    function xmlwriter_end_comment(\XMLWriter $writer): bool {}

    function xmlwriter_start_attribute(\XMLWriter $writer, string $name): bool {}

    function xmlwriter_end_attribute(\XMLWriter $writer): bool {}

    function xmlwriter_write_attribute(\XMLWriter $writer, string $name, string $value): bool {}

    function xmlwriter_start_attribute_ns(\XMLWriter $writer, ?string $prefix, string $name, ?string $namespace): bool {}

    function xmlwriter_write_attribute_ns(\XMLWriter $writer, ?string $prefix, string $name, ?string $namespace, string $value): bool {}

    function xmlwriter_start_element(\XMLWriter $writer, string $name): bool {}

    function xmlwriter_end_element(\XMLWriter $writer): bool {}

    function xmlwriter_full_end_element(\XMLWriter $writer): bool {}

    function xmlwriter_start_element_ns(\XMLWriter $writer, ?string $prefix, string $name, ?string $namespace): bool {}

    function xmlwriter_write_element(\XMLWriter $writer, string $name, ?string $content = NULL): bool {}

    function xmlwriter_write_element_ns(\XMLWriter $writer, ?string $prefix, string $name, ?string $namespace, ?string $content = NULL): bool {}

    function xmlwriter_start_pi(\XMLWriter $writer, string $target): bool {}

    function xmlwriter_end_pi(\XMLWriter $writer): bool {}

    function xmlwriter_write_pi(\XMLWriter $writer, string $target, string $content): bool {}

    function xmlwriter_start_cdata(\XMLWriter $writer): bool {}

    function xmlwriter_end_cdata(\XMLWriter $writer): bool {}

    function xmlwriter_write_cdata(\XMLWriter $writer, string $content): bool {}

    function xmlwriter_text(\XMLWriter $writer, string $content): bool {}

    function xmlwriter_write_raw(\XMLWriter $writer, string $content): bool {}

    function xmlwriter_start_document(\XMLWriter $writer, ?string $version = '1.0', ?string $encoding = NULL, ?string $standalone = NULL): bool {}

    function xmlwriter_end_document(\XMLWriter $writer): bool {}

    function xmlwriter_write_comment(\XMLWriter $writer, string $content): bool {}

    function xmlwriter_start_dtd(\XMLWriter $writer, string $qualifiedName, ?string $publicId = NULL, ?string $systemId = NULL): bool {}

    function xmlwriter_end_dtd(\XMLWriter $writer): bool {}

    function xmlwriter_write_dtd(\XMLWriter $writer, string $name, ?string $publicId = NULL, ?string $systemId = NULL, ?string $content = NULL): bool {}

    function xmlwriter_start_dtd_element(\XMLWriter $writer, string $qualifiedName): bool {}

    function xmlwriter_end_dtd_element(\XMLWriter $writer): bool {}

    function xmlwriter_write_dtd_element(\XMLWriter $writer, string $name, string $content): bool {}

    function xmlwriter_start_dtd_attlist(\XMLWriter $writer, string $name): bool {}

    function xmlwriter_end_dtd_attlist(\XMLWriter $writer): bool {}

    function xmlwriter_write_dtd_attlist(\XMLWriter $writer, string $name, string $content): bool {}

    function xmlwriter_start_dtd_entity(\XMLWriter $writer, string $name, bool $isParam): bool {}

    function xmlwriter_end_dtd_entity(\XMLWriter $writer): bool {}

    function xmlwriter_write_dtd_entity(\XMLWriter $writer, string $name, string $content, bool $isParam = false, ?string $publicId = NULL, ?string $systemId = NULL, ?string $notationData = NULL): bool {}

    function xmlwriter_output_memory(\XMLWriter $writer, bool $flush = true): string {}

    function xmlwriter_flush(\XMLWriter $writer, bool $empty = true): string|int {}

    class XMLWriter {
        public function openUri(string $uri) {}
        public static function toUri(string $uri): static {}
        public function openMemory() {}
        public static function toMemory(): static {}
        public static function toStream($stream): static {}
        public function setIndent(bool $enable) {}
        public function setIndentString(string $indentation) {}
        public function startComment() {}
        public function endComment() {}
        public function startAttribute(string $name) {}
        public function endAttribute() {}
        public function writeAttribute(string $name, string $value) {}
        public function startAttributeNs(?string $prefix, string $name, ?string $namespace) {}
        public function writeAttributeNs(?string $prefix, string $name, ?string $namespace, string $value) {}
        public function startElement(string $name) {}
        public function endElement() {}
        public function fullEndElement() {}
        public function startElementNs(?string $prefix, string $name, ?string $namespace) {}
        public function writeElement(string $name, ?string $content = NULL) {}
        public function writeElementNs(?string $prefix, string $name, ?string $namespace, ?string $content = NULL) {}
        public function startPi(string $target) {}
        public function endPi() {}
        public function writePi(string $target, string $content) {}
        public function startCdata() {}
        public function endCdata() {}
        public function writeCdata(string $content) {}
        public function text(string $content) {}
        public function writeRaw(string $content) {}
        public function startDocument(?string $version = '1.0', ?string $encoding = NULL, ?string $standalone = NULL) {}
        public function endDocument() {}
        public function writeComment(string $content) {}
        public function startDtd(string $qualifiedName, ?string $publicId = NULL, ?string $systemId = NULL) {}
        public function endDtd() {}
        public function writeDtd(string $name, ?string $publicId = NULL, ?string $systemId = NULL, ?string $content = NULL) {}
        public function startDtdElement(string $qualifiedName) {}
        public function endDtdElement() {}
        public function writeDtdElement(string $name, string $content) {}
        public function startDtdAttlist(string $name) {}
        public function endDtdAttlist() {}
        public function writeDtdAttlist(string $name, string $content) {}
        public function startDtdEntity(string $name, bool $isParam) {}
        public function endDtdEntity() {}
        public function writeDtdEntity(string $name, string $content, bool $isParam = false, ?string $publicId = NULL, ?string $systemId = NULL, ?string $notationData = NULL) {}
        public function outputMemory(bool $flush = true) {}
        public function flush(bool $empty = true) {}
    }

}
