<?php
// Runtime declarations exported from extension: Phar

namespace {
    class PharException extends \Exception implements \Throwable, \Stringable {
    }

    class Phar extends \RecursiveDirectoryIterator implements \RecursiveIterator, \Iterator, \Traversable, \SeekableIterator, \Stringable, \Countable, \ArrayAccess {
        public const BZ2 = 8192;
        public const GZ = 4096;
        public const NONE = 0;
        public const PHAR = 1;
        public const TAR = 2;
        public const ZIP = 3;
        public const COMPRESSED = 61440;
        public const PHP = 0;
        public const PHPS = 1;
        public const MD5 = 1;
        public const OPENSSL = 16;
        public const OPENSSL_SHA256 = 17;
        public const OPENSSL_SHA512 = 18;
        public const SHA1 = 2;
        public const SHA256 = 3;
        public const SHA512 = 4;
        public function __construct(string $filename, int $flags = 12288, ?string $alias = NULL) {}
        public function __destruct() {}
        public function addEmptyDir(string $directory) {}
        public function addFile(string $filename, ?string $localName = NULL) {}
        public function addFromString(string $localName, string $contents) {}
        public function buildFromDirectory(string $directory, string $pattern = '') {}
        public function buildFromIterator(\Traversable $iterator, ?string $baseDirectory = NULL) {}
        public function compressFiles(int $compression) {}
        public function decompressFiles() {}
        public function compress(int $compression, ?string $extension = NULL) {}
        public function decompress(?string $extension = NULL) {}
        public function convertToExecutable(?int $format = NULL, ?int $compression = NULL, ?string $extension = NULL) {}
        public function convertToData(?int $format = NULL, ?int $compression = NULL, ?string $extension = NULL) {}
        public function copy(string $from, string $to) {}
        public function count(int $mode = \COUNT_NORMAL) {}
        public function delete(string $localName) {}
        public function delMetadata() {}
        public function extractTo(string $directory, array|string|null $files = NULL, bool $overwrite = false) {}
        public function getAlias() {}
        public function getPath() {}
        public function getMetadata(array $unserializeOptions = array ()) {}
        public function getModified() {}
        public function getSignature() {}
        public function getStub() {}
        public function getVersion() {}
        public function hasMetadata() {}
        public function isBuffering() {}
        public function isCompressed() {}
        public function isFileFormat(int $format) {}
        public function isWritable() {}
        public function offsetExists($localName) {}
        public function offsetGet($localName) {}
        public function offsetSet($localName, $value) {}
        public function offsetUnset($localName) {}
        public function setAlias(string $alias) {}
        public function setDefaultStub(?string $index = NULL, ?string $webIndex = NULL) {}
        public function setMetadata(mixed $metadata) {}
        public function setSignatureAlgorithm(int $algo, ?string $privateKey = NULL) {}
        public function setStub($stub, int $length = null) {}
        public function startBuffering() {}
        public function stopBuffering() {}
        public static final function apiVersion(): string {}
        public static final function canCompress(int $compression = 0): bool {}
        public static final function canWrite(): bool {}
        public static final function createDefaultStub(?string $index = NULL, ?string $webIndex = NULL): string {}
        public static final function getSupportedCompression(): array {}
        public static final function getSupportedSignatures(): array {}
        public static final function interceptFileFuncs(): void {}
        public static final function isValidPharFilename(string $filename, bool $executable = true): bool {}
        public static final function loadPhar(string $filename, ?string $alias = NULL): bool {}
        public static final function mapPhar(?string $alias = NULL, int $offset = 0): bool {}
        public static final function running(bool $returnPhar = true): string {}
        public static final function mount(string $pharPath, string $externalPath): void {}
        public static final function mungServer(array $variables): void {}
        public static final function unlinkArchive(string $filename): true {}
        public static final function webPhar(?string $alias = NULL, ?string $index = NULL, ?string $fileNotFoundScript = NULL, array $mimeTypes = array (), ?callable $rewrite = NULL): void {}
    }

    class PharData extends \RecursiveDirectoryIterator implements \RecursiveIterator, \Iterator, \Traversable, \SeekableIterator, \Stringable, \Countable, \ArrayAccess {
        public function __construct(string $filename, int $flags = 12288, ?string $alias = NULL, int $format = 0) {}
        public function __destruct() {}
        public function addEmptyDir(string $directory) {}
        public function addFile(string $filename, ?string $localName = NULL) {}
        public function addFromString(string $localName, string $contents) {}
        public function buildFromDirectory(string $directory, string $pattern = '') {}
        public function buildFromIterator(\Traversable $iterator, ?string $baseDirectory = NULL) {}
        public function compressFiles(int $compression) {}
        public function decompressFiles() {}
        public function compress(int $compression, ?string $extension = NULL) {}
        public function decompress(?string $extension = NULL) {}
        public function convertToExecutable(?int $format = NULL, ?int $compression = NULL, ?string $extension = NULL) {}
        public function convertToData(?int $format = NULL, ?int $compression = NULL, ?string $extension = NULL) {}
        public function copy(string $from, string $to) {}
        public function count(int $mode = \COUNT_NORMAL) {}
        public function delete(string $localName) {}
        public function delMetadata() {}
        public function extractTo(string $directory, array|string|null $files = NULL, bool $overwrite = false) {}
        public function getAlias() {}
        public function getPath() {}
        public function getMetadata(array $unserializeOptions = array ()) {}
        public function getModified() {}
        public function getSignature() {}
        public function getStub() {}
        public function getVersion() {}
        public function hasMetadata() {}
        public function isBuffering() {}
        public function isCompressed() {}
        public function isFileFormat(int $format) {}
        public function isWritable() {}
        public function offsetExists($localName) {}
        public function offsetGet($localName) {}
        public function offsetSet($localName, $value) {}
        public function offsetUnset($localName) {}
        public function setAlias(string $alias) {}
        public function setDefaultStub(?string $index = NULL, ?string $webIndex = NULL) {}
        public function setMetadata(mixed $metadata) {}
        public function setSignatureAlgorithm(int $algo, ?string $privateKey = NULL) {}
        public function setStub($stub, int $length = null): true {}
        public function startBuffering() {}
        public function stopBuffering() {}
        public static final function apiVersion(): string {}
        public static final function canCompress(int $compression = 0): bool {}
        public static final function canWrite(): bool {}
        public static final function createDefaultStub(?string $index = NULL, ?string $webIndex = NULL): string {}
        public static final function getSupportedCompression(): array {}
        public static final function getSupportedSignatures(): array {}
        public static final function interceptFileFuncs(): void {}
        public static final function isValidPharFilename(string $filename, bool $executable = true): bool {}
        public static final function loadPhar(string $filename, ?string $alias = NULL): bool {}
        public static final function mapPhar(?string $alias = NULL, int $offset = 0): bool {}
        public static final function running(bool $returnPhar = true): string {}
        public static final function mount(string $pharPath, string $externalPath): void {}
        public static final function mungServer(array $variables): void {}
        public static final function unlinkArchive(string $filename): true {}
        public static final function webPhar(?string $alias = NULL, ?string $index = NULL, ?string $fileNotFoundScript = NULL, array $mimeTypes = array (), ?callable $rewrite = NULL): void {}
    }

    class PharFileInfo extends \SplFileInfo implements \Stringable {
        public function __construct(string $filename) {}
        public function __destruct() {}
        public function chmod(int $perms) {}
        public function compress(int $compression) {}
        public function decompress() {}
        public function delMetadata() {}
        public function getCompressedSize() {}
        public function getCRC32() {}
        public function getContent() {}
        public function getMetadata(array $unserializeOptions = array ()) {}
        public function getPharFlags() {}
        public function hasMetadata() {}
        public function isCompressed(?int $compression = NULL) {}
        public function isCRCChecked() {}
        public function setMetadata(mixed $metadata) {}
    }

}
