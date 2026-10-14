<?php
// Runtime declarations exported from extension: SPL

namespace {
    function class_implements($object_or_class, bool $autoload = true): array|false {}

    function class_parents($object_or_class, bool $autoload = true): array|false {}

    function class_uses($object_or_class, bool $autoload = true): array|false {}

    function spl_autoload(string $class, ?string $file_extensions = NULL): void {}

    function spl_autoload_call(string $class): void {}

    function spl_autoload_extensions(?string $file_extensions = NULL): string {}

    function spl_autoload_functions(): array {}

    function spl_autoload_register(?callable $callback = NULL, bool $throw = true, bool $prepend = false): bool {}

    function spl_autoload_unregister(callable $callback): bool {}

    function spl_classes(): array {}

    function spl_object_hash(object $object): string {}

    function spl_object_id(object $object): int {}

    function iterator_apply(\Traversable $iterator, callable $callback, ?array $args = NULL): int {}

    function iterator_count(\Traversable|array $iterator): int {}

    function iterator_to_array(\Traversable|array $iterator, bool $preserve_keys = true): array {}

    class LogicException extends \Exception implements \Throwable, \Stringable {
    }

    class BadFunctionCallException extends \LogicException implements \Stringable, \Throwable {
    }

    class BadMethodCallException extends \BadFunctionCallException implements \Throwable, \Stringable {
    }

    class DomainException extends \LogicException implements \Stringable, \Throwable {
    }

    class InvalidArgumentException extends \LogicException implements \Stringable, \Throwable {
    }

    class LengthException extends \LogicException implements \Stringable, \Throwable {
    }

    class OutOfRangeException extends \LogicException implements \Stringable, \Throwable {
    }

    class RuntimeException extends \Exception implements \Throwable, \Stringable {
    }

    class OutOfBoundsException extends \RuntimeException implements \Stringable, \Throwable {
    }

    class OverflowException extends \RuntimeException implements \Stringable, \Throwable {
    }

    class RangeException extends \RuntimeException implements \Stringable, \Throwable {
    }

    class UnderflowException extends \RuntimeException implements \Stringable, \Throwable {
    }

    class UnexpectedValueException extends \RuntimeException implements \Stringable, \Throwable {
    }

    interface RecursiveIterator extends \Iterator, \Traversable {
        public function hasChildren();
        public function getChildren();
    }

    interface OuterIterator extends \Iterator, \Traversable {
        public function getInnerIterator();
    }

    class RecursiveIteratorIterator implements \OuterIterator, \Traversable, \Iterator {
        public const LEAVES_ONLY = 0;
        public const SELF_FIRST = 1;
        public const CHILD_FIRST = 2;
        public const CATCH_GET_CHILD = 16;
        public function __construct(\Traversable $iterator, int $mode = \RecursiveIteratorIterator::LEAVES_ONLY, int $flags = 0) {}
        public function rewind() {}
        public function valid() {}
        public function key() {}
        public function current() {}
        public function next() {}
        public function getDepth() {}
        public function getSubIterator(?int $level = NULL) {}
        public function getInnerIterator() {}
        public function beginIteration() {}
        public function endIteration() {}
        public function callHasChildren() {}
        public function callGetChildren() {}
        public function beginChildren() {}
        public function endChildren() {}
        public function nextElement() {}
        public function setMaxDepth(int $maxDepth = -1) {}
        public function getMaxDepth() {}
    }

    class IteratorIterator implements \OuterIterator, \Traversable, \Iterator {
        public function __construct(\Traversable $iterator, ?string $class = NULL) {}
        public function getInnerIterator() {}
        public function rewind() {}
        public function valid() {}
        public function key() {}
        public function current() {}
        public function next() {}
    }

    abstract class FilterIterator extends \IteratorIterator implements \Iterator, \Traversable, \OuterIterator {
        public abstract function accept();
        public function __construct(\Iterator $iterator) {}
        public function rewind() {}
        public function next() {}
    }

    abstract class RecursiveFilterIterator extends \FilterIterator implements \OuterIterator, \Traversable, \Iterator, \RecursiveIterator {
        public function __construct(\RecursiveIterator $iterator) {}
        public function hasChildren() {}
        public function getChildren() {}
    }

    class CallbackFilterIterator extends \FilterIterator implements \OuterIterator, \Traversable, \Iterator {
        public function __construct(\Iterator $iterator, callable $callback) {}
        public function accept() {}
    }

    class RecursiveCallbackFilterIterator extends \CallbackFilterIterator implements \Iterator, \Traversable, \OuterIterator, \RecursiveIterator {
        public function __construct(\RecursiveIterator $iterator, callable $callback) {}
        public function hasChildren() {}
        public function getChildren() {}
    }

    class ParentIterator extends \RecursiveFilterIterator implements \RecursiveIterator, \Iterator, \Traversable, \OuterIterator {
        public function __construct(\RecursiveIterator $iterator) {}
        public function accept() {}
    }

    interface SeekableIterator extends \Iterator, \Traversable {
        public function seek(int $offset);
    }

    class LimitIterator extends \IteratorIterator implements \Iterator, \Traversable, \OuterIterator {
        public function __construct(\Iterator $iterator, int $offset = 0, int $limit = -1) {}
        public function rewind() {}
        public function valid() {}
        public function next() {}
        public function seek(int $offset) {}
        public function getPosition() {}
    }

    class CachingIterator extends \IteratorIterator implements \Stringable, \Iterator, \Traversable, \OuterIterator, \ArrayAccess, \Countable {
        public const CALL_TOSTRING = 1;
        public const CATCH_GET_CHILD = 16;
        public const TOSTRING_USE_KEY = 2;
        public const TOSTRING_USE_CURRENT = 4;
        public const TOSTRING_USE_INNER = 8;
        public const FULL_CACHE = 256;
        public function __construct(\Iterator $iterator, int $flags = \CachingIterator::CALL_TOSTRING) {}
        public function rewind() {}
        public function valid() {}
        public function next() {}
        public function hasNext() {}
        public function __toString(): string {}
        public function getFlags() {}
        public function setFlags(int $flags) {}
        public function offsetGet($key) {}
        public function offsetSet($key, mixed $value) {}
        public function offsetUnset($key) {}
        public function offsetExists($key) {}
        public function getCache() {}
        public function count() {}
    }

    class RecursiveCachingIterator extends \CachingIterator implements \Countable, \ArrayAccess, \OuterIterator, \Traversable, \Iterator, \Stringable, \RecursiveIterator {
        public function __construct(\Iterator $iterator, int $flags = \RecursiveCachingIterator::CALL_TOSTRING) {}
        public function hasChildren() {}
        public function getChildren() {}
    }

    class NoRewindIterator extends \IteratorIterator implements \Iterator, \Traversable, \OuterIterator {
        public function __construct(\Iterator $iterator) {}
        public function rewind() {}
        public function valid() {}
        public function key() {}
        public function current() {}
        public function next() {}
    }

    class AppendIterator extends \IteratorIterator implements \Iterator, \Traversable, \OuterIterator {
        public function __construct() {}
        public function append(\Iterator $iterator) {}
        public function rewind() {}
        public function valid() {}
        public function current() {}
        public function next() {}
        public function getIteratorIndex() {}
        public function getArrayIterator() {}
    }

    class InfiniteIterator extends \IteratorIterator implements \Iterator, \Traversable, \OuterIterator {
        public function __construct(\Iterator $iterator) {}
        public function next() {}
    }

    class RegexIterator extends \FilterIterator implements \OuterIterator, \Traversable, \Iterator {
        public const USE_KEY = 1;
        public const INVERT_MATCH = 2;
        public const MATCH = 0;
        public const GET_MATCH = 1;
        public const ALL_MATCHES = 2;
        public const SPLIT = 3;
        public const REPLACE = 4;
        public  ?string $replacement = NULL;
        public function __construct(\Iterator $iterator, string $pattern, int $mode = \RegexIterator::MATCH, int $flags = 0, int $pregFlags = 0) {}
        public function accept() {}
        public function getMode() {}
        public function setMode(int $mode) {}
        public function getFlags() {}
        public function setFlags(int $flags) {}
        public function getRegex() {}
        public function getPregFlags() {}
        public function setPregFlags(int $pregFlags) {}
    }

    class RecursiveRegexIterator extends \RegexIterator implements \Iterator, \Traversable, \OuterIterator, \RecursiveIterator {
        public function __construct(\RecursiveIterator $iterator, string $pattern, int $mode = \RecursiveRegexIterator::MATCH, int $flags = 0, int $pregFlags = 0) {}
        public function accept() {}
        public function hasChildren() {}
        public function getChildren() {}
    }

    class EmptyIterator implements \Iterator, \Traversable {
        public function current() {}
        public function next() {}
        public function key() {}
        public function valid() {}
        public function rewind() {}
    }

    class RecursiveTreeIterator extends \RecursiveIteratorIterator implements \Iterator, \Traversable, \OuterIterator {
        public const BYPASS_CURRENT = 4;
        public const BYPASS_KEY = 8;
        public const PREFIX_LEFT = 0;
        public const PREFIX_MID_HAS_NEXT = 1;
        public const PREFIX_MID_LAST = 2;
        public const PREFIX_END_HAS_NEXT = 3;
        public const PREFIX_END_LAST = 4;
        public const PREFIX_RIGHT = 5;
        public function __construct(\RecursiveIterator|\IteratorAggregate $iterator, int $flags = \RecursiveTreeIterator::BYPASS_KEY, int $cachingIteratorFlags = \CachingIterator::CATCH_GET_CHILD, int $mode = \RecursiveTreeIterator::SELF_FIRST) {}
        public function key() {}
        public function current() {}
        public function getPrefix() {}
        public function setPostfix(string $postfix) {}
        public function setPrefixPart(int $part, string $value) {}
        public function getEntry() {}
        public function getPostfix() {}
    }

    class ArrayObject implements \IteratorAggregate, \Traversable, \ArrayAccess, \Serializable, \Countable {
        public const STD_PROP_LIST = 1;
        public const ARRAY_AS_PROPS = 2;
        public function __construct(object|array $array = array (), int $flags = 0, string $iteratorClass = 'ArrayIterator') {}
        public function offsetExists(mixed $key) {}
        public function offsetGet(mixed $key) {}
        public function offsetSet(mixed $key, mixed $value) {}
        public function offsetUnset(mixed $key) {}
        public function append(mixed $value) {}
        public function getArrayCopy() {}
        public function count() {}
        public function getFlags() {}
        public function setFlags(int $flags) {}
        public function asort(int $flags = \SORT_REGULAR) {}
        public function ksort(int $flags = \SORT_REGULAR) {}
        public function uasort(callable $callback) {}
        public function uksort(callable $callback) {}
        public function natsort() {}
        public function natcasesort() {}
        public function unserialize(string $data) {}
        public function serialize() {}
        public function __serialize() {}
        public function __unserialize(array $data) {}
        public function getIterator() {}
        public function exchangeArray(object|array $array) {}
        public function setIteratorClass(string $iteratorClass) {}
        public function getIteratorClass() {}
        public function __debugInfo() {}
    }

    class ArrayIterator implements \SeekableIterator, \Traversable, \Iterator, \ArrayAccess, \Serializable, \Countable {
        public const STD_PROP_LIST = 1;
        public const ARRAY_AS_PROPS = 2;
        public function __construct(object|array $array = array (), int $flags = 0) {}
        public function offsetExists(mixed $key) {}
        public function offsetGet(mixed $key) {}
        public function offsetSet(mixed $key, mixed $value) {}
        public function offsetUnset(mixed $key) {}
        public function append(mixed $value) {}
        public function getArrayCopy() {}
        public function count() {}
        public function getFlags() {}
        public function setFlags(int $flags) {}
        public function asort(int $flags = \SORT_REGULAR) {}
        public function ksort(int $flags = \SORT_REGULAR) {}
        public function uasort(callable $callback) {}
        public function uksort(callable $callback) {}
        public function natsort() {}
        public function natcasesort() {}
        public function unserialize(string $data) {}
        public function serialize() {}
        public function __serialize() {}
        public function __unserialize(array $data) {}
        public function rewind() {}
        public function current() {}
        public function key() {}
        public function next() {}
        public function valid() {}
        public function seek(int $offset) {}
        public function __debugInfo() {}
    }

    class RecursiveArrayIterator extends \ArrayIterator implements \Countable, \Serializable, \ArrayAccess, \Iterator, \Traversable, \SeekableIterator, \RecursiveIterator {
        public const CHILD_ARRAYS_ONLY = 4;
        public function hasChildren() {}
        public function getChildren() {}
    }

    class SplFileInfo implements \Stringable {
        public function __construct(string $filename) {}
        public function getPath() {}
        public function getFilename() {}
        public function getExtension() {}
        public function getBasename(string $suffix = '') {}
        public function getPathname() {}
        public function getPerms() {}
        public function getInode() {}
        public function getSize() {}
        public function getOwner() {}
        public function getGroup() {}
        public function getATime() {}
        public function getMTime() {}
        public function getCTime() {}
        public function getType() {}
        public function isWritable() {}
        public function isReadable() {}
        public function isExecutable() {}
        public function isFile() {}
        public function isDir() {}
        public function isLink() {}
        public function getLinkTarget() {}
        public function getRealPath() {}
        public function getFileInfo(?string $class = NULL) {}
        public function getPathInfo(?string $class = NULL) {}
        public function openFile(string $mode = 'r', bool $useIncludePath = false, $context = NULL) {}
        public function setFileClass(string $class = 'SplFileObject') {}
        public function setInfoClass(string $class = 'SplFileInfo') {}
        public function __toString(): string {}
        public function __debugInfo() {}
        public final function _bad_state_ex(): void {}
    }

    class DirectoryIterator extends \SplFileInfo implements \Stringable, \SeekableIterator, \Traversable, \Iterator {
        public function __construct(string $directory) {}
        public function getFilename() {}
        public function getExtension() {}
        public function getBasename(string $suffix = '') {}
        public function isDot() {}
        public function rewind() {}
        public function valid() {}
        public function key() {}
        public function current() {}
        public function next() {}
        public function seek(int $offset) {}
        public function __toString(): string {}
    }

    class FilesystemIterator extends \DirectoryIterator implements \Iterator, \Traversable, \SeekableIterator, \Stringable {
        public const CURRENT_MODE_MASK = 240;
        public const CURRENT_AS_PATHNAME = 32;
        public const CURRENT_AS_FILEINFO = 0;
        public const CURRENT_AS_SELF = 16;
        public const KEY_MODE_MASK = 3840;
        public const KEY_AS_PATHNAME = 0;
        public const FOLLOW_SYMLINKS = 16384;
        public const KEY_AS_FILENAME = 256;
        public const NEW_CURRENT_AND_KEY = 256;
        public const OTHER_MODE_MASK = 28672;
        public const SKIP_DOTS = 4096;
        public const UNIX_PATHS = 8192;
        public function __construct(string $directory, int $flags = 4096) {}
        public function rewind() {}
        public function key() {}
        public function current() {}
        public function getFlags() {}
        public function setFlags(int $flags) {}
    }

    class RecursiveDirectoryIterator extends \FilesystemIterator implements \Stringable, \SeekableIterator, \Traversable, \Iterator, \RecursiveIterator {
        public function __construct(string $directory, int $flags = 0) {}
        public function hasChildren(bool $allowLinks = false) {}
        public function getChildren() {}
        public function getSubPath() {}
        public function getSubPathname() {}
    }

    class GlobIterator extends \FilesystemIterator implements \Stringable, \SeekableIterator, \Traversable, \Iterator, \Countable {
        public function __construct(string $pattern, int $flags = 0) {}
        public function count() {}
    }

    class SplFileObject extends \SplFileInfo implements \Stringable, \RecursiveIterator, \Traversable, \Iterator, \SeekableIterator {
        public const DROP_NEW_LINE = 1;
        public const READ_AHEAD = 2;
        public const SKIP_EMPTY = 4;
        public const READ_CSV = 8;
        public function __construct(string $filename, string $mode = 'r', bool $useIncludePath = false, $context = NULL) {}
        public function rewind() {}
        public function eof() {}
        public function valid() {}
        public function fgets() {}
        public function fread(int $length) {}
        public function fgetcsv(string $separator = ',', string $enclosure = '"', string $escape = '\\') {}
        public function fputcsv(array $fields, string $separator = ',', string $enclosure = '"', string $escape = '\\', string $eol = '
    ') {}
        public function setCsvControl(string $separator = ',', string $enclosure = '"', string $escape = '\\') {}
        public function getCsvControl() {}
        public function flock(int $operation, &$wouldBlock = NULL) {}
        public function fflush() {}
        public function ftell() {}
        public function fseek(int $offset, int $whence = \SEEK_SET) {}
        public function fgetc() {}
        public function fpassthru() {}
        public function fscanf(string $format, mixed &...$vars) {}
        public function fwrite(string $data, ?int $length = NULL) {}
        public function fstat() {}
        public function ftruncate(int $size) {}
        public function current() {}
        public function key() {}
        public function next() {}
        public function setFlags(int $flags) {}
        public function getFlags() {}
        public function setMaxLineLen(int $maxLength) {}
        public function getMaxLineLen() {}
        public function hasChildren() {}
        public function getChildren() {}
        public function seek(int $line) {}
        public function getCurrentLine() {}
        public function __toString(): string {}
    }

    class SplTempFileObject extends \SplFileObject implements \SeekableIterator, \Iterator, \Traversable, \RecursiveIterator, \Stringable {
        public function __construct(int $maxMemory = 2097152) {}
    }

    class SplDoublyLinkedList implements \Iterator, \Traversable, \Countable, \ArrayAccess, \Serializable {
        public const IT_MODE_LIFO = 2;
        public const IT_MODE_FIFO = 0;
        public const IT_MODE_DELETE = 1;
        public const IT_MODE_KEEP = 0;
        public function add(int $index, mixed $value) {}
        public function pop() {}
        public function shift() {}
        public function push(mixed $value) {}
        public function unshift(mixed $value) {}
        public function top() {}
        public function bottom() {}
        public function __debugInfo() {}
        public function count() {}
        public function isEmpty() {}
        public function setIteratorMode(int $mode) {}
        public function getIteratorMode() {}
        public function offsetExists($index) {}
        public function offsetGet($index) {}
        public function offsetSet($index, mixed $value) {}
        public function offsetUnset($index) {}
        public function rewind() {}
        public function current() {}
        public function key() {}
        public function prev() {}
        public function next() {}
        public function valid() {}
        public function unserialize(string $data) {}
        public function serialize() {}
        public function __serialize() {}
        public function __unserialize(array $data) {}
    }

    class SplQueue extends \SplDoublyLinkedList implements \Serializable, \ArrayAccess, \Countable, \Traversable, \Iterator {
        public function enqueue(mixed $value) {}
        public function dequeue() {}
    }

    class SplStack extends \SplDoublyLinkedList implements \Serializable, \ArrayAccess, \Countable, \Traversable, \Iterator {
    }

    abstract class SplHeap implements \Iterator, \Traversable, \Countable {
        public function extract() {}
        public function insert(mixed $value) {}
        public function top() {}
        public function count() {}
        public function isEmpty() {}
        public function rewind() {}
        public function current() {}
        public function key() {}
        public function next() {}
        public function valid() {}
        public function recoverFromCorruption() {}
        protected abstract function compare(mixed $value1, mixed $value2);
        public function isCorrupted() {}
        public function __debugInfo() {}
        public function __serialize() {}
        public function __unserialize(array $data) {}
    }

    class SplMinHeap extends \SplHeap implements \Countable, \Traversable, \Iterator {
        protected function compare(mixed $value1, mixed $value2) {}
    }

    class SplMaxHeap extends \SplHeap implements \Countable, \Traversable, \Iterator {
        protected function compare(mixed $value1, mixed $value2) {}
    }

    class SplPriorityQueue implements \Iterator, \Traversable, \Countable {
        public const EXTR_BOTH = 3;
        public const EXTR_PRIORITY = 2;
        public const EXTR_DATA = 1;
        public function compare(mixed $priority1, mixed $priority2) {}
        public function insert(mixed $value, mixed $priority) {}
        public function setExtractFlags(int $flags) {}
        public function top() {}
        public function extract() {}
        public function count() {}
        public function isEmpty() {}
        public function rewind() {}
        public function current() {}
        public function key() {}
        public function next() {}
        public function valid() {}
        public function recoverFromCorruption() {}
        public function isCorrupted() {}
        public function getExtractFlags() {}
        public function __debugInfo() {}
        public function __serialize() {}
        public function __unserialize(array $data) {}
    }

    class SplFixedArray implements \IteratorAggregate, \Traversable, \ArrayAccess, \Countable, \JsonSerializable {
        public function __construct(int $size = 0) {}
        public function __wakeup() {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
        public function count() {}
        public function toArray() {}
        public static function fromArray(array $array, bool $preserveKeys = true) {}
        public function getSize() {}
        public function setSize(int $size) {}
        public function offsetExists($index) {}
        public function offsetGet($index) {}
        public function offsetSet($index, mixed $value) {}
        public function offsetUnset($index) {}
        public function getIterator(): \Iterator {}
        public function jsonSerialize(): array {}
    }

    interface SplObserver {
        public function update(\SplSubject $subject);
    }

    interface SplSubject {
        public function attach(\SplObserver $observer);
        public function detach(\SplObserver $observer);
        public function notify();
    }

    class SplObjectStorage implements \Countable, \SeekableIterator, \Traversable, \Iterator, \Serializable, \ArrayAccess {
        public function attach(object $object, mixed $info = NULL) {}
        public function detach(object $object) {}
        public function contains(object $object) {}
        public function addAll(\SplObjectStorage $storage) {}
        public function removeAll(\SplObjectStorage $storage) {}
        public function removeAllExcept(\SplObjectStorage $storage) {}
        public function getInfo() {}
        public function setInfo(mixed $info) {}
        public function count(int $mode = \COUNT_NORMAL) {}
        public function rewind() {}
        public function valid() {}
        public function key() {}
        public function current() {}
        public function next() {}
        public function seek(int $offset): void {}
        public function unserialize(string $data) {}
        public function serialize() {}
        public function offsetExists($object) {}
        public function offsetGet($object) {}
        public function offsetSet($object, mixed $info = NULL) {}
        public function offsetUnset($object) {}
        public function getHash(object $object) {}
        public function __serialize() {}
        public function __unserialize(array $data) {}
        public function __debugInfo() {}
    }

    class MultipleIterator implements \Iterator, \Traversable {
        public const MIT_NEED_ANY = 0;
        public const MIT_NEED_ALL = 1;
        public const MIT_KEYS_NUMERIC = 0;
        public const MIT_KEYS_ASSOC = 2;
        public function __construct(int $flags = 1) {}
        public function getFlags() {}
        public function setFlags(int $flags) {}
        public function attachIterator(\Iterator $iterator, string|int|null $info = NULL) {}
        public function detachIterator(\Iterator $iterator) {}
        public function containsIterator(\Iterator $iterator) {}
        public function countIterators() {}
        public function rewind() {}
        public function valid() {}
        public function key() {}
        public function current() {}
        public function next() {}
        public function __debugInfo() {}
    }

}
