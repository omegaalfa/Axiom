<?php
// Runtime declarations exported from extension: dom

namespace {
    function dom_import_simplexml(object $node): \DOMAttr|\DOMElement {}

    final class DOMException extends \Exception implements \Throwable, \Stringable {
        public  $code = 0;
    }

    interface DOMParentNode {
        public function append(...$nodes): void;
        public function prepend(...$nodes): void;
        public function replaceChildren(...$nodes): void;
    }

    interface DOMChildNode {
        public function remove(): void;
        public function before(...$nodes): void;
        public function after(...$nodes): void;
        public function replaceWith(...$nodes): void;
    }

    class DOMImplementation {
        public function hasFeature(string $feature, string $version) {}
        public function createDocumentType(string $qualifiedName, string $publicId = '', string $systemId = '') {}
        public function createDocument(?string $namespace = NULL, string $qualifiedName = '', ?\DOMDocumentType $doctype = NULL) {}
    }

    class DOMNode {
        public const DOCUMENT_POSITION_DISCONNECTED = 1;
        public const DOCUMENT_POSITION_PRECEDING = 2;
        public const DOCUMENT_POSITION_FOLLOWING = 4;
        public const DOCUMENT_POSITION_CONTAINS = 8;
        public const DOCUMENT_POSITION_CONTAINED_BY = 16;
        public const DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC = 32;
        public  string $nodeName;
        public  ?string $nodeValue;
        public  int $nodeType;
        public  ?\DOMNode $parentNode;
        public  ?\DOMElement $parentElement;
        public  \DOMNodeList $childNodes;
        public  ?\DOMNode $firstChild;
        public  ?\DOMNode $lastChild;
        public  ?\DOMNode $previousSibling;
        public  ?\DOMNode $nextSibling;
        public  ?\DOMNamedNodeMap $attributes;
        public  bool $isConnected;
        public  ?\DOMDocument $ownerDocument;
        public  ?string $namespaceURI;
        public  string $prefix;
        public  ?string $localName;
        public  ?string $baseURI;
        public  string $textContent;
        public function appendChild(\DOMNode $node) {}
        public function C14N(bool $exclusive = false, bool $withComments = false, ?array $xpath = NULL, ?array $nsPrefixes = NULL) {}
        public function C14NFile(string $uri, bool $exclusive = false, bool $withComments = false, ?array $xpath = NULL, ?array $nsPrefixes = NULL) {}
        public function cloneNode(bool $deep = false) {}
        public function getLineNo() {}
        public function getNodePath() {}
        public function hasAttributes() {}
        public function hasChildNodes() {}
        public function insertBefore(\DOMNode $node, ?\DOMNode $child = NULL) {}
        public function isDefaultNamespace(string $namespace) {}
        public function isSameNode(\DOMNode $otherNode) {}
        public function isEqualNode(?\DOMNode $otherNode): bool {}
        public function isSupported(string $feature, string $version) {}
        public function lookupNamespaceURI(?string $prefix) {}
        public function lookupPrefix(string $namespace) {}
        public function normalize() {}
        public function removeChild(\DOMNode $child) {}
        public function replaceChild(\DOMNode $node, \DOMNode $child) {}
        public function contains(\DOMNode|\DOMNameSpaceNode|null $other): bool {}
        public function getRootNode(?array $options = NULL): \DOMNode {}
        public function compareDocumentPosition(\DOMNode $other): int {}
        public function __sleep(): array {}
        public function __wakeup(): void {}
    }

    class DOMNameSpaceNode {
        public  string $nodeName;
        public  ?string $nodeValue;
        public  int $nodeType;
        public  string $prefix;
        public  ?string $localName;
        public  ?string $namespaceURI;
        public  bool $isConnected;
        public  ?\DOMDocument $ownerDocument;
        public  ?\DOMNode $parentNode;
        public  ?\DOMElement $parentElement;
        public function __sleep(): array {}
        public function __wakeup(): void {}
    }

    class DOMDocumentFragment extends \DOMNode implements \DOMParentNode {
        public  ?\DOMElement $firstElementChild;
        public  ?\DOMElement $lastElementChild;
        public  int $childElementCount;
        public function __construct() {}
        public function appendXML(string $data) {}
        public function append(...$nodes): void {}
        public function prepend(...$nodes): void {}
        public function replaceChildren(...$nodes): void {}
    }

    class DOMDocument extends \DOMNode implements \DOMParentNode {
        public  ?\DOMDocumentType $doctype;
        public  \DOMImplementation $implementation;
        public  ?\DOMElement $documentElement;
        public  ?string $actualEncoding;
        public  ?string $encoding;
        public  ?string $xmlEncoding;
        public  bool $standalone;
        public  bool $xmlStandalone;
        public  ?string $version;
        public  ?string $xmlVersion;
        public  bool $strictErrorChecking;
        public  ?string $documentURI;
        public  mixed $config;
        public  bool $formatOutput;
        public  bool $validateOnParse;
        public  bool $resolveExternals;
        public  bool $preserveWhiteSpace;
        public  bool $recover;
        public  bool $substituteEntities;
        public  ?\DOMElement $firstElementChild;
        public  ?\DOMElement $lastElementChild;
        public  int $childElementCount;
        public function __construct(string $version = '1.0', string $encoding = '') {}
        public function createAttribute(string $localName) {}
        public function createAttributeNS(?string $namespace, string $qualifiedName) {}
        public function createCDATASection(string $data) {}
        public function createComment(string $data) {}
        public function createDocumentFragment() {}
        public function createElement(string $localName, string $value = '') {}
        public function createElementNS(?string $namespace, string $qualifiedName, string $value = '') {}
        public function createEntityReference(string $name) {}
        public function createProcessingInstruction(string $target, string $data = '') {}
        public function createTextNode(string $data) {}
        public function getElementById(string $elementId) {}
        public function getElementsByTagName(string $qualifiedName) {}
        public function getElementsByTagNameNS(?string $namespace, string $localName) {}
        public function importNode(\DOMNode $node, bool $deep = false) {}
        public function load(string $filename, int $options = 0) {}
        public function loadXML(string $source, int $options = 0) {}
        public function normalizeDocument() {}
        public function registerNodeClass(string $baseClass, ?string $extendedClass) {}
        public function save(string $filename, int $options = 0) {}
        public function loadHTML(string $source, int $options = 0) {}
        public function loadHTMLFile(string $filename, int $options = 0) {}
        public function saveHTML(?\DOMNode $node = NULL) {}
        public function saveHTMLFile(string $filename) {}
        public function saveXML(?\DOMNode $node = NULL, int $options = 0) {}
        public function schemaValidate(string $filename, int $flags = 0) {}
        public function schemaValidateSource(string $source, int $flags = 0) {}
        public function relaxNGValidate(string $filename) {}
        public function relaxNGValidateSource(string $source) {}
        public function validate() {}
        public function xinclude(int $options = 0) {}
        public function adoptNode(\DOMNode $node) {}
        public function append(...$nodes): void {}
        public function prepend(...$nodes): void {}
        public function replaceChildren(...$nodes): void {}
    }

    class DOMNodeList implements \IteratorAggregate, \Traversable, \Countable {
        public  int $length;
        public function count() {}
        public function getIterator(): \Iterator {}
        public function item(int $index) {}
    }

    class DOMNamedNodeMap implements \IteratorAggregate, \Traversable, \Countable {
        public  int $length;
        public function getNamedItem(string $qualifiedName) {}
        public function getNamedItemNS(?string $namespace, string $localName) {}
        public function item(int $index) {}
        public function count() {}
        public function getIterator(): \Iterator {}
    }

    class DOMCharacterData extends \DOMNode implements \DOMChildNode {
        public  string $data;
        public  int $length;
        public  ?\DOMElement $previousElementSibling;
        public  ?\DOMElement $nextElementSibling;
        public function appendData(string $data) {}
        public function substringData(int $offset, int $count) {}
        public function insertData(int $offset, string $data) {}
        public function deleteData(int $offset, int $count) {}
        public function replaceData(int $offset, int $count, string $data) {}
        public function replaceWith(...$nodes): void {}
        public function remove(): void {}
        public function before(...$nodes): void {}
        public function after(...$nodes): void {}
    }

    class DOMAttr extends \DOMNode {
        public  string $name;
        public  bool $specified;
        public  string $value;
        public  ?\DOMElement $ownerElement;
        public  mixed $schemaTypeInfo;
        public function __construct(string $name, string $value = '') {}
        public function isId() {}
    }

    class DOMElement extends \DOMNode implements \DOMParentNode, \DOMChildNode {
        public  string $tagName;
        public  string $className;
        public  string $id;
        public  mixed $schemaTypeInfo;
        public  ?\DOMElement $firstElementChild;
        public  ?\DOMElement $lastElementChild;
        public  int $childElementCount;
        public  ?\DOMElement $previousElementSibling;
        public  ?\DOMElement $nextElementSibling;
        public function __construct(string $qualifiedName, ?string $value = NULL, string $namespace = '') {}
        public function getAttribute(string $qualifiedName) {}
        public function getAttributeNames(): array {}
        public function getAttributeNS(?string $namespace, string $localName) {}
        public function getAttributeNode(string $qualifiedName) {}
        public function getAttributeNodeNS(?string $namespace, string $localName) {}
        public function getElementsByTagName(string $qualifiedName) {}
        public function getElementsByTagNameNS(?string $namespace, string $localName) {}
        public function hasAttribute(string $qualifiedName) {}
        public function hasAttributeNS(?string $namespace, string $localName) {}
        public function removeAttribute(string $qualifiedName) {}
        public function removeAttributeNS(?string $namespace, string $localName) {}
        public function removeAttributeNode(\DOMAttr $attr) {}
        public function setAttribute(string $qualifiedName, string $value) {}
        public function setAttributeNS(?string $namespace, string $qualifiedName, string $value) {}
        public function setAttributeNode(\DOMAttr $attr) {}
        public function setAttributeNodeNS(\DOMAttr $attr) {}
        public function setIdAttribute(string $qualifiedName, bool $isId) {}
        public function setIdAttributeNS(string $namespace, string $qualifiedName, bool $isId) {}
        public function setIdAttributeNode(\DOMAttr $attr, bool $isId) {}
        public function toggleAttribute(string $qualifiedName, ?bool $force = NULL): bool {}
        public function remove(): void {}
        public function before(...$nodes): void {}
        public function after(...$nodes): void {}
        public function replaceWith(...$nodes): void {}
        public function append(...$nodes): void {}
        public function prepend(...$nodes): void {}
        public function replaceChildren(...$nodes): void {}
        public function insertAdjacentElement(string $where, \DOMElement $element): ?\DOMElement {}
        public function insertAdjacentText(string $where, string $data): void {}
    }

    class DOMText extends \DOMCharacterData implements \DOMChildNode {
        public  string $wholeText;
        public function __construct(string $data = '') {}
        public function isWhitespaceInElementContent() {}
        public function isElementContentWhitespace() {}
        public function splitText(int $offset) {}
    }

    class DOMComment extends \DOMCharacterData implements \DOMChildNode {
        public function __construct(string $data = '') {}
    }

    class DOMCdataSection extends \DOMText implements \DOMChildNode {
        public function __construct(string $data) {}
    }

    class DOMDocumentType extends \DOMNode {
        public  string $name;
        public  \DOMNamedNodeMap $entities;
        public  \DOMNamedNodeMap $notations;
        public  string $publicId;
        public  string $systemId;
        public  ?string $internalSubset;
    }

    class DOMNotation extends \DOMNode {
        public  string $publicId;
        public  string $systemId;
    }

    class DOMEntity extends \DOMNode {
        public  ?string $publicId;
        public  ?string $systemId;
        public  ?string $notationName;
        public  ?string $actualEncoding;
        public  ?string $encoding;
        public  ?string $version;
    }

    class DOMEntityReference extends \DOMNode {
        public function __construct(string $name) {}
    }

    class DOMProcessingInstruction extends \DOMNode {
        public  string $target;
        public  string $data;
        public function __construct(string $name, string $value = '') {}
    }

    class DOMXPath {
        public  \DOMDocument $document;
        public  bool $registerNodeNamespaces;
        public function __construct(\DOMDocument $document, bool $registerNodeNS = true) {}
        public function evaluate(string $expression, ?\DOMNode $contextNode = NULL, bool $registerNodeNS = true) {}
        public function query(string $expression, ?\DOMNode $contextNode = NULL, bool $registerNodeNS = true) {}
        public function registerNamespace(string $prefix, string $namespace) {}
        public function registerPhpFunctions(array|string|null $restrict = NULL) {}
        public function registerPhpFunctionNS(string $namespaceURI, string $name, callable $callable): void {}
        public static function quote(string $str): string {}
    }

    const XML_ELEMENT_NODE = 1;

    const XML_ATTRIBUTE_NODE = 2;

    const XML_TEXT_NODE = 3;

    const XML_CDATA_SECTION_NODE = 4;

    const XML_ENTITY_REF_NODE = 5;

    const XML_ENTITY_NODE = 6;

    const XML_PI_NODE = 7;

    const XML_COMMENT_NODE = 8;

    const XML_DOCUMENT_NODE = 9;

    const XML_DOCUMENT_TYPE_NODE = 10;

    const XML_DOCUMENT_FRAG_NODE = 11;

    const XML_NOTATION_NODE = 12;

    const XML_HTML_DOCUMENT_NODE = 13;

    const XML_DTD_NODE = 14;

    const XML_ELEMENT_DECL_NODE = 15;

    const XML_ATTRIBUTE_DECL_NODE = 16;

    const XML_ENTITY_DECL_NODE = 17;

    const XML_NAMESPACE_DECL_NODE = 18;

    const XML_LOCAL_NAMESPACE = 18;

    const XML_ATTRIBUTE_CDATA = 1;

    const XML_ATTRIBUTE_ID = 2;

    const XML_ATTRIBUTE_IDREF = 3;

    const XML_ATTRIBUTE_IDREFS = 4;

    const XML_ATTRIBUTE_ENTITY = 6;

    const XML_ATTRIBUTE_NMTOKEN = 7;

    const XML_ATTRIBUTE_NMTOKENS = 8;

    const XML_ATTRIBUTE_ENUMERATION = 9;

    const XML_ATTRIBUTE_NOTATION = 10;

    const DOM_PHP_ERR = 0;

    const DOM_INDEX_SIZE_ERR = 1;

    const DOMSTRING_SIZE_ERR = 2;

    const DOM_HIERARCHY_REQUEST_ERR = 3;

    const DOM_WRONG_DOCUMENT_ERR = 4;

    const DOM_INVALID_CHARACTER_ERR = 5;

    const DOM_NO_DATA_ALLOWED_ERR = 6;

    const DOM_NO_MODIFICATION_ALLOWED_ERR = 7;

    const DOM_NOT_FOUND_ERR = 8;

    const DOM_NOT_SUPPORTED_ERR = 9;

    const DOM_INUSE_ATTRIBUTE_ERR = 10;

    const DOM_INVALID_STATE_ERR = 11;

    const DOM_SYNTAX_ERR = 12;

    const DOM_INVALID_MODIFICATION_ERR = 13;

    const DOM_NAMESPACE_ERR = 14;

    const DOM_INVALID_ACCESS_ERR = 15;

    const DOM_VALIDATION_ERR = 16;

}

namespace Dom {
    function import_simplexml(object $node): \Dom\Attr|\Dom\Element {}

    enum AdjacentPosition: string implements \BackedEnum, \UnitEnum {
        case BeforeBegin = 'beforebegin';
        case AfterBegin = 'afterbegin';
        case BeforeEnd = 'beforeend';
        case AfterEnd = 'afterend';
        public static function cases(): array {}
        public static function from(string|int $value): static {}
        public static function tryFrom(string|int $value): ?static {}
    }

    interface ParentNode {
        public function append(\Dom\Node|string ...$nodes): void;
        public function prepend(\Dom\Node|string ...$nodes): void;
        public function replaceChildren(\Dom\Node|string ...$nodes): void;
        public function querySelector(string $selectors): ?\Dom\Element;
        public function querySelectorAll(string $selectors): \Dom\NodeList;
    }

    interface ChildNode {
        public function remove(): void;
        public function before(\Dom\Node|string ...$nodes): void;
        public function after(\Dom\Node|string ...$nodes): void;
        public function replaceWith(\Dom\Node|string ...$nodes): void;
    }

    class Implementation {
        public function createDocumentType(string $qualifiedName, string $publicId, string $systemId): \Dom\DocumentType {}
        public function createDocument(?string $namespace, string $qualifiedName, ?\Dom\DocumentType $doctype = NULL): \Dom\XMLDocument {}
        public function createHTMLDocument(?string $title = NULL): \Dom\HTMLDocument {}
    }

    class Node {
        public const DOCUMENT_POSITION_DISCONNECTED = 1;
        public const DOCUMENT_POSITION_PRECEDING = 2;
        public const DOCUMENT_POSITION_FOLLOWING = 4;
        public const DOCUMENT_POSITION_CONTAINS = 8;
        public const DOCUMENT_POSITION_CONTAINED_BY = 16;
        public const DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC = 32;
        public  int $nodeType;
        public  string $nodeName;
        public  string $baseURI;
        public  bool $isConnected;
        public  ?\Dom\Document $ownerDocument;
        public  ?\Dom\Node $parentNode;
        public  ?\Dom\Element $parentElement;
        public  \Dom\NodeList $childNodes;
        public  ?\Dom\Node $firstChild;
        public  ?\Dom\Node $lastChild;
        public  ?\Dom\Node $previousSibling;
        public  ?\Dom\Node $nextSibling;
        public  ?string $nodeValue;
        public  ?string $textContent;
        private final function __construct() {}
        public function getRootNode(array $options = array ()): \Dom\Node {}
        public function hasChildNodes(): bool {}
        public function normalize(): void {}
        public function cloneNode(bool $deep = false): \Dom\Node {}
        public function isEqualNode(?\Dom\Node $otherNode): bool {}
        public function isSameNode(?\Dom\Node $otherNode): bool {}
        public function compareDocumentPosition(\Dom\Node $other): int {}
        public function contains(?\Dom\Node $other): bool {}
        public function lookupPrefix(?string $namespace): ?string {}
        public function lookupNamespaceURI(?string $prefix): ?string {}
        public function isDefaultNamespace(?string $namespace): bool {}
        public function insertBefore(\Dom\Node $node, ?\Dom\Node $child): \Dom\Node {}
        public function appendChild(\Dom\Node $node): \Dom\Node {}
        public function replaceChild(\Dom\Node $node, \Dom\Node $child): \Dom\Node {}
        public function removeChild(\Dom\Node $child): \Dom\Node {}
        public function getLineNo(): int {}
        public function getNodePath(): string {}
        public function C14N(bool $exclusive = false, bool $withComments = false, ?array $xpath = NULL, ?array $nsPrefixes = NULL): string|false {}
        public function C14NFile(string $uri, bool $exclusive = false, bool $withComments = false, ?array $xpath = NULL, ?array $nsPrefixes = NULL): int|false {}
        public function __sleep(): array {}
        public function __wakeup(): void {}
    }

    final readonly class NamespaceInfo {
        public  readonly protected(set) ?string $prefix;
        public  readonly protected(set) ?string $namespaceURI;
        public  readonly protected(set) \Dom\Element $element;
        private function __construct() {}
    }

    class DocumentFragment extends \Dom\Node implements \Dom\ParentNode {
        public  \Dom\HTMLCollection $children;
        public  ?\Dom\Element $firstElementChild;
        public  ?\Dom\Element $lastElementChild;
        public  int $childElementCount;
        public function appendXml(string $data): bool {}
        public function append(\Dom\Node|string ...$nodes): void {}
        public function prepend(\Dom\Node|string ...$nodes): void {}
        public function replaceChildren(\Dom\Node|string ...$nodes): void {}
        public function querySelector(string $selectors): ?\Dom\Element {}
        public function querySelectorAll(string $selectors): \Dom\NodeList {}
    }

    abstract class Document extends \Dom\Node implements \Dom\ParentNode {
        public  \Dom\HTMLCollection $children;
        public  ?\Dom\Element $firstElementChild;
        public  ?\Dom\Element $lastElementChild;
        public  int $childElementCount;
        public  \Dom\Implementation $implementation;
        public  string $URL;
        public  string $documentURI;
        public  string $characterSet;
        public  string $charset;
        public  string $inputEncoding;
        public  ?\Dom\DocumentType $doctype;
        public  ?\Dom\Element $documentElement;
        public  ?\Dom\HTMLElement $body;
        public  ?\Dom\HTMLElement $head;
        public  string $title;
        public function getElementsByTagName(string $qualifiedName): \Dom\HTMLCollection {}
        public function getElementsByTagNameNS(?string $namespace, string $localName): \Dom\HTMLCollection {}
        public function getElementsByClassName(string $classNames): \Dom\HTMLCollection {}
        public function createElement(string $localName): \Dom\Element {}
        public function createElementNS(?string $namespace, string $qualifiedName): \Dom\Element {}
        public function createDocumentFragment(): \Dom\DocumentFragment {}
        public function createTextNode(string $data): \Dom\Text {}
        public function createCDATASection(string $data): \Dom\CDATASection {}
        public function createComment(string $data): \Dom\Comment {}
        public function createProcessingInstruction(string $target, string $data): \Dom\ProcessingInstruction {}
        public function importNode(?\Dom\Node $node, bool $deep = false): \Dom\Node {}
        public function adoptNode(\Dom\Node $node): \Dom\Node {}
        public function createAttribute(string $localName): \Dom\Attr {}
        public function createAttributeNS(?string $namespace, string $qualifiedName): \Dom\Attr {}
        public function getElementById(string $elementId): ?\Dom\Element {}
        public function registerNodeClass(string $baseClass, ?string $extendedClass): void {}
        public function schemaValidate(string $filename, int $flags = 0): bool {}
        public function schemaValidateSource(string $source, int $flags = 0): bool {}
        public function relaxNgValidate(string $filename): bool {}
        public function relaxNgValidateSource(string $source): bool {}
        public function append(\Dom\Node|string ...$nodes): void {}
        public function prepend(\Dom\Node|string ...$nodes): void {}
        public function replaceChildren(\Dom\Node|string ...$nodes): void {}
        public function importLegacyNode(\DOMNode $node, bool $deep = false): \Dom\Node {}
        public function querySelector(string $selectors): ?\Dom\Element {}
        public function querySelectorAll(string $selectors): \Dom\NodeList {}
    }

    final class HTMLDocument extends \Dom\Document implements \Dom\ParentNode {
        public static function createEmpty(string $encoding = 'UTF-8'): \Dom\HTMLDocument {}
        public static function createFromFile(string $path, int $options = 0, ?string $overrideEncoding = NULL): \Dom\HTMLDocument {}
        public static function createFromString(string $source, int $options = 0, ?string $overrideEncoding = NULL): \Dom\HTMLDocument {}
        public function saveXml(?\Dom\Node $node = NULL, int $options = 0): string|false {}
        public function saveXmlFile(string $filename, int $options = 0): int|false {}
        public function saveHtml(?\Dom\Node $node = NULL): string {}
        public function saveHtmlFile(string $filename): int|false {}
    }

    final class XMLDocument extends \Dom\Document implements \Dom\ParentNode {
        public  string $xmlEncoding;
        public  bool $xmlStandalone;
        public  string $xmlVersion;
        public  bool $formatOutput;
        public static function createEmpty(string $version = '1.0', string $encoding = 'UTF-8'): \Dom\XMLDocument {}
        public static function createFromFile(string $path, int $options = 0, ?string $overrideEncoding = NULL): \Dom\XMLDocument {}
        public static function createFromString(string $source, int $options = 0, ?string $overrideEncoding = NULL): \Dom\XMLDocument {}
        public function createEntityReference(string $name): \Dom\EntityReference {}
        public function validate(): bool {}
        public function xinclude(int $options = 0): int {}
        public function saveXml(?\Dom\Node $node = NULL, int $options = 0): string|false {}
        public function saveXmlFile(string $filename, int $options = 0): int|false {}
    }

    class NodeList implements \IteratorAggregate, \Traversable, \Countable {
        public  int $length;
        public function count(): int {}
        public function getIterator(): \Iterator {}
        public function item(int $index): ?\Dom\Node {}
    }

    class NamedNodeMap implements \IteratorAggregate, \Traversable, \Countable {
        public  int $length;
        public function item(int $index): ?\Dom\Attr {}
        public function getNamedItem(string $qualifiedName): ?\Dom\Attr {}
        public function getNamedItemNS(?string $namespace, string $localName): ?\Dom\Attr {}
        public function count(): int {}
        public function getIterator(): \Iterator {}
    }

    class DtdNamedNodeMap implements \IteratorAggregate, \Traversable, \Countable {
        public  int $length;
        public function item(int $index): \Dom\Entity|\Dom\Notation|null {}
        public function getNamedItem(string $qualifiedName): \Dom\Entity|\Dom\Notation|null {}
        public function getNamedItemNS(?string $namespace, string $localName): \Dom\Entity|\Dom\Notation|null {}
        public function count(): int {}
        public function getIterator(): \Iterator {}
    }

    class HTMLCollection implements \IteratorAggregate, \Traversable, \Countable {
        public  int $length;
        public function item(int $index): ?\Dom\Element {}
        public function namedItem(string $key): ?\Dom\Element {}
        public function count(): int {}
        public function getIterator(): \Iterator {}
    }

    class CharacterData extends \Dom\Node implements \Dom\ChildNode {
        public  ?\Dom\Element $previousElementSibling;
        public  ?\Dom\Element $nextElementSibling;
        public  string $data;
        public  int $length;
        public function substringData(int $offset, int $count): string {}
        public function appendData(string $data): void {}
        public function insertData(int $offset, string $data): void {}
        public function deleteData(int $offset, int $count): void {}
        public function replaceData(int $offset, int $count, string $data): void {}
        public function remove(): void {}
        public function before(\Dom\Node|string ...$nodes): void {}
        public function after(\Dom\Node|string ...$nodes): void {}
        public function replaceWith(\Dom\Node|string ...$nodes): void {}
    }

    class Attr extends \Dom\Node {
        public  ?string $namespaceURI;
        public  ?string $prefix;
        public  string $localName;
        public  string $name;
        public  string $value;
        public  ?\Dom\Element $ownerElement;
        public  bool $specified;
        public function isId(): bool {}
        public function rename(?string $namespaceURI, string $qualifiedName): void {}
    }

    class Element extends \Dom\Node implements \Dom\ParentNode, \Dom\ChildNode {
        public  ?string $namespaceURI;
        public  ?string $prefix;
        public  string $localName;
        public  string $tagName;
        public  \Dom\HTMLCollection $children;
        public  ?\Dom\Element $firstElementChild;
        public  ?\Dom\Element $lastElementChild;
        public  int $childElementCount;
        public  ?\Dom\Element $previousElementSibling;
        public  ?\Dom\Element $nextElementSibling;
        public  string $id;
        public  string $className;
        public  \Dom\TokenList $classList;
        public  \Dom\NamedNodeMap $attributes;
        public  string $innerHTML;
        public  string $outerHTML;
        public  string $substitutedNodeValue;
        public function hasAttributes(): bool {}
        public function getAttributeNames(): array {}
        public function getAttribute(string $qualifiedName): ?string {}
        public function getAttributeNS(?string $namespace, string $localName): ?string {}
        public function setAttribute(string $qualifiedName, string $value): void {}
        public function setAttributeNS(?string $namespace, string $qualifiedName, string $value): void {}
        public function removeAttribute(string $qualifiedName): void {}
        public function removeAttributeNS(?string $namespace, string $localName): void {}
        public function toggleAttribute(string $qualifiedName, ?bool $force = NULL): bool {}
        public function hasAttribute(string $qualifiedName): bool {}
        public function hasAttributeNS(?string $namespace, string $localName): bool {}
        public function getAttributeNode(string $qualifiedName): ?\Dom\Attr {}
        public function getAttributeNodeNS(?string $namespace, string $localName): ?\Dom\Attr {}
        public function setAttributeNode(\Dom\Attr $attr): ?\Dom\Attr {}
        public function setAttributeNodeNS(\Dom\Attr $attr): ?\Dom\Attr {}
        public function removeAttributeNode(\Dom\Attr $attr): \Dom\Attr {}
        public function getElementsByTagName(string $qualifiedName): \Dom\HTMLCollection {}
        public function getElementsByTagNameNS(?string $namespace, string $localName): \Dom\HTMLCollection {}
        public function getElementsByClassName(string $classNames): \Dom\HTMLCollection {}
        public function insertAdjacentElement(\Dom\AdjacentPosition $where, \Dom\Element $element): ?\Dom\Element {}
        public function insertAdjacentText(\Dom\AdjacentPosition $where, string $data): void {}
        public function insertAdjacentHTML(\Dom\AdjacentPosition $where, string $string): void {}
        public function setIdAttribute(string $qualifiedName, bool $isId): void {}
        public function setIdAttributeNS(?string $namespace, string $qualifiedName, bool $isId): void {}
        public function setIdAttributeNode(\Dom\Attr $attr, bool $isId): void {}
        public function remove(): void {}
        public function before(\Dom\Node|string ...$nodes): void {}
        public function after(\Dom\Node|string ...$nodes): void {}
        public function replaceWith(\Dom\Node|string ...$nodes): void {}
        public function append(\Dom\Node|string ...$nodes): void {}
        public function prepend(\Dom\Node|string ...$nodes): void {}
        public function replaceChildren(\Dom\Node|string ...$nodes): void {}
        public function querySelector(string $selectors): ?\Dom\Element {}
        public function querySelectorAll(string $selectors): \Dom\NodeList {}
        public function closest(string $selectors): ?\Dom\Element {}
        public function matches(string $selectors): bool {}
        public function getInScopeNamespaces(): array {}
        public function getDescendantNamespaces(): array {}
        public function rename(?string $namespaceURI, string $qualifiedName): void {}
    }

    class HTMLElement extends \Dom\Element implements \Dom\ChildNode, \Dom\ParentNode {
    }

    class Text extends \Dom\CharacterData implements \Dom\ChildNode {
        public  string $wholeText;
        public function splitText(int $offset): \Dom\Text {}
    }

    class Comment extends \Dom\CharacterData implements \Dom\ChildNode {
    }

    class CDATASection extends \Dom\Text implements \Dom\ChildNode {
    }

    class DocumentType extends \Dom\Node implements \Dom\ChildNode {
        public  string $name;
        public  \Dom\DtdNamedNodeMap $entities;
        public  \Dom\DtdNamedNodeMap $notations;
        public  string $publicId;
        public  string $systemId;
        public  ?string $internalSubset;
        public function remove(): void {}
        public function before(\Dom\Node|string ...$nodes): void {}
        public function after(\Dom\Node|string ...$nodes): void {}
        public function replaceWith(\Dom\Node|string ...$nodes): void {}
    }

    class Notation extends \Dom\Node {
        public  string $publicId;
        public  string $systemId;
    }

    class Entity extends \Dom\Node {
        public  ?string $publicId;
        public  ?string $systemId;
        public  ?string $notationName;
    }

    class EntityReference extends \Dom\Node {
    }

    class ProcessingInstruction extends \Dom\CharacterData implements \Dom\ChildNode {
        public  string $target;
    }

    final class XPath {
        public  \Dom\Document $document;
        public  bool $registerNodeNamespaces;
        public function __construct(\Dom\Document $document, bool $registerNodeNS = true) {}
        public function evaluate(string $expression, ?\Dom\Node $contextNode = NULL, bool $registerNodeNS = true): \Dom\NodeList|string|float|bool|null {}
        public function query(string $expression, ?\Dom\Node $contextNode = NULL, bool $registerNodeNS = true): \Dom\NodeList {}
        public function registerNamespace(string $prefix, string $namespace): bool {}
        public function registerPhpFunctions(array|string|null $restrict = NULL): void {}
        public function registerPhpFunctionNS(string $namespaceURI, string $name, callable $callable): void {}
        public static function quote(string $str): string {}
    }

    final class TokenList implements \IteratorAggregate, \Traversable, \Countable {
        public  int $length;
        public  string $value;
        private function __construct() {}
        public function item(int $index): ?string {}
        public function contains(string $token): bool {}
        public function add(string ...$tokens): void {}
        public function remove(string ...$tokens): void {}
        public function toggle(string $token, ?bool $force = NULL): bool {}
        public function replace(string $token, string $newToken): bool {}
        public function supports(string $token): bool {}
        public function count(): int {}
        public function getIterator(): \Iterator {}
    }

    const INDEX_SIZE_ERR = 1;

    const STRING_SIZE_ERR = 2;

    const HIERARCHY_REQUEST_ERR = 3;

    const WRONG_DOCUMENT_ERR = 4;

    const INVALID_CHARACTER_ERR = 5;

    const NO_DATA_ALLOWED_ERR = 6;

    const NO_MODIFICATION_ALLOWED_ERR = 7;

    const NOT_FOUND_ERR = 8;

    const NOT_SUPPORTED_ERR = 9;

    const INUSE_ATTRIBUTE_ERR = 10;

    const INVALID_STATE_ERR = 11;

    const SYNTAX_ERR = 12;

    const INVALID_MODIFICATION_ERR = 13;

    const NAMESPACE_ERR = 14;

    const VALIDATION_ERR = 16;

    const HTML_NO_DEFAULT_NS = 2147483648;

}
