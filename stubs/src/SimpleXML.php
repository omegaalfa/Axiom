<?php
// Runtime declarations exported from extension: SimpleXML

namespace {
    function simplexml_load_file(string $filename, ?string $class_name = 'SimpleXMLElement', int $options = 0, string $namespace_or_prefix = '', bool $is_prefix = false): \SimpleXMLElement|false {}

    function simplexml_load_string(string $data, ?string $class_name = 'SimpleXMLElement', int $options = 0, string $namespace_or_prefix = '', bool $is_prefix = false): \SimpleXMLElement|false {}

    function simplexml_import_dom(object $node, ?string $class_name = 'SimpleXMLElement'): ?\SimpleXMLElement {}

    class SimpleXMLElement implements \Stringable, \Countable, \RecursiveIterator, \Traversable, \Iterator {
        public function xpath(string $expression) {}
        public function registerXPathNamespace(string $prefix, string $namespace) {}
        public function asXML(?string $filename = NULL) {}
        public function saveXML(?string $filename = NULL) {}
        public function getNamespaces(bool $recursive = false) {}
        public function getDocNamespaces(bool $recursive = false, bool $fromRoot = true) {}
        public function children(?string $namespaceOrPrefix = NULL, bool $isPrefix = false) {}
        public function attributes(?string $namespaceOrPrefix = NULL, bool $isPrefix = false) {}
        public function __construct(string $data, int $options = 0, bool $dataIsURL = false, string $namespaceOrPrefix = '', bool $isPrefix = false) {}
        public function addChild(string $qualifiedName, ?string $value = NULL, ?string $namespace = NULL) {}
        public function addAttribute(string $qualifiedName, string $value, ?string $namespace = NULL) {}
        public function getName() {}
        public function __toString(): string {}
        public function __debugInfo(): ?array {}
        public function count() {}
        public function rewind() {}
        public function valid() {}
        public function current() {}
        public function key() {}
        public function next() {}
        public function hasChildren() {}
        public function getChildren() {}
    }

    class SimpleXMLIterator extends \SimpleXMLElement implements \Iterator, \Traversable, \RecursiveIterator, \Countable, \Stringable {
    }

}
