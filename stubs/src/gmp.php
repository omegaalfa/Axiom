<?php
// Runtime declarations exported from extension: gmp

namespace {
    function gmp_init(string|int $num, int $base = 0): \GMP {}

    function gmp_import(string $data, int $word_size = 1, int $flags = 17): \GMP {}

    function gmp_export(\GMP|string|int $num, int $word_size = 1, int $flags = 17): string {}

    function gmp_intval(\GMP|string|int $num): int {}

    function gmp_strval(\GMP|string|int $num, int $base = 10): string {}

    function gmp_add(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_sub(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_mul(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_div_qr(\GMP|string|int $num1, \GMP|string|int $num2, int $rounding_mode = \GMP_ROUND_ZERO): array {}

    function gmp_div_q(\GMP|string|int $num1, \GMP|string|int $num2, int $rounding_mode = \GMP_ROUND_ZERO): \GMP {}

    function gmp_div_r(\GMP|string|int $num1, \GMP|string|int $num2, int $rounding_mode = \GMP_ROUND_ZERO): \GMP {}

    function gmp_div(\GMP|string|int $num1, \GMP|string|int $num2, int $rounding_mode = \GMP_ROUND_ZERO): \GMP {}

    function gmp_mod(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_divexact(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_neg(\GMP|string|int $num): \GMP {}

    function gmp_abs(\GMP|string|int $num): \GMP {}

    function gmp_fact(\GMP|string|int $num): \GMP {}

    function gmp_sqrt(\GMP|string|int $num): \GMP {}

    function gmp_sqrtrem(\GMP|string|int $num): array {}

    function gmp_root(\GMP|string|int $num, int $nth): \GMP {}

    function gmp_rootrem(\GMP|string|int $num, int $nth): array {}

    function gmp_pow(\GMP|string|int $num, int $exponent): \GMP {}

    function gmp_powm(\GMP|string|int $num, \GMP|string|int $exponent, \GMP|string|int $modulus): \GMP {}

    function gmp_perfect_square(\GMP|string|int $num): bool {}

    function gmp_perfect_power(\GMP|string|int $num): bool {}

    function gmp_prob_prime(\GMP|string|int $num, int $repetitions = 10): int {}

    function gmp_gcd(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_gcdext(\GMP|string|int $num1, \GMP|string|int $num2): array {}

    function gmp_lcm(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_invert(\GMP|string|int $num1, \GMP|string|int $num2): \GMP|false {}

    function gmp_jacobi(\GMP|string|int $num1, \GMP|string|int $num2): int {}

    function gmp_legendre(\GMP|string|int $num1, \GMP|string|int $num2): int {}

    function gmp_kronecker(\GMP|string|int $num1, \GMP|string|int $num2): int {}

    function gmp_cmp(\GMP|string|int $num1, \GMP|string|int $num2): int {}

    function gmp_sign(\GMP|string|int $num): int {}

    function gmp_random_seed(\GMP|string|int $seed): void {}

    function gmp_random_bits(int $bits): \GMP {}

    function gmp_random_range(\GMP|string|int $min, \GMP|string|int $max): \GMP {}

    function gmp_and(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_or(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_com(\GMP|string|int $num): \GMP {}

    function gmp_xor(\GMP|string|int $num1, \GMP|string|int $num2): \GMP {}

    function gmp_setbit(\GMP $num, int $index, bool $value = true): void {}

    function gmp_clrbit(\GMP $num, int $index): void {}

    function gmp_testbit(\GMP|string|int $num, int $index): bool {}

    function gmp_scan0(\GMP|string|int $num1, int $start): int {}

    function gmp_scan1(\GMP|string|int $num1, int $start): int {}

    function gmp_popcount(\GMP|string|int $num): int {}

    function gmp_hamdist(\GMP|string|int $num1, \GMP|string|int $num2): int {}

    function gmp_nextprime(\GMP|string|int $num): \GMP {}

    function gmp_binomial(\GMP|string|int $n, int $k): \GMP {}

    final class GMP {
        public function __construct(string|int $num = 0, int $base = 0) {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
    }

    const GMP_ROUND_ZERO = 0;

    const GMP_ROUND_PLUSINF = 1;

    const GMP_ROUND_MINUSINF = 2;

    const GMP_VERSION = '6.3.0';

    const GMP_MSW_FIRST = 1;

    const GMP_LSW_FIRST = 2;

    const GMP_LITTLE_ENDIAN = 4;

    const GMP_BIG_ENDIAN = 8;

    const GMP_NATIVE_ENDIAN = 16;

}
