<?php
// Runtime declarations exported from extension: random

namespace {
    function lcg_value(): float {}

    function mt_srand(?int $seed = NULL, int $mode = \MT_RAND_MT19937): void {}

    function srand(?int $seed = NULL, int $mode = \MT_RAND_MT19937): void {}

    function rand(int $min = null, int $max = null): int {}

    function mt_rand(int $min = null, int $max = null): int {}

    function mt_getrandmax(): int {}

    function getrandmax(): int {}

    function random_bytes(int $length): string {}

    function random_int(int $min, int $max): int {}

    const MT_RAND_MT19937 = 0;

    const MT_RAND_PHP = 1;

}

namespace Random {
    interface Engine {
        public function generate(): string;
    }

    interface CryptoSafeEngine extends \Random\Engine {
    }

    class RandomError extends \Error implements \Throwable, \Stringable {
    }

    class BrokenRandomEngineError extends \Random\RandomError implements \Stringable, \Throwable {
    }

    class RandomException extends \Exception implements \Throwable, \Stringable {
    }

    final class Randomizer {
        public  readonly protected(set) \Random\Engine $engine;
        public function __construct(?\Random\Engine $engine = NULL) {}
        public function nextInt(): int {}
        public function nextFloat(): float {}
        public function getFloat(float $min, float $max, \Random\IntervalBoundary $boundary = \Random\IntervalBoundary::ClosedOpen): float {}
        public function getInt(int $min, int $max): int {}
        public function getBytes(int $length): string {}
        public function getBytesFromString(string $string, int $length): string {}
        public function shuffleArray(array $array): array {}
        public function shuffleBytes(string $bytes): string {}
        public function pickArrayKeys(array $array, int $num): array {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
    }

    enum IntervalBoundary implements \UnitEnum {
        case ClosedOpen;
        case ClosedClosed;
        case OpenClosed;
        case OpenOpen;
        public static function cases(): array {}
    }

}

namespace Random\Engine {
    final class Mt19937 implements \Random\Engine {
        public function __construct(?int $seed = NULL, int $mode = \MT_RAND_MT19937) {}
        public function generate(): string {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
        public function __debugInfo(): array {}
    }

    final class PcgOneseq128XslRr64 implements \Random\Engine {
        public function __construct(string|int|null $seed = NULL) {}
        public function generate(): string {}
        public function jump(int $advance): void {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
        public function __debugInfo(): array {}
    }

    final class Xoshiro256StarStar implements \Random\Engine {
        public function __construct(string|int|null $seed = NULL) {}
        public function generate(): string {}
        public function jump(): void {}
        public function jumpLong(): void {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
        public function __debugInfo(): array {}
    }

    final class Secure implements \Random\CryptoSafeEngine, \Random\Engine {
        public function generate(): string {}
    }

}
