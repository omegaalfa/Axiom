<?php

namespace PhpPlayground\Php74;

class ArrowFunctionsExample
{
    public function testBasic($x)
    {
        $fn = fn($val) => $val * 2;
        return $fn($x);
    }

    public function testTypes(int $a, int $b): int
    {
        $adder = fn(int $x, int $y): int => $x + $y;
        return $adder($a, $b);
    }

    public function testArrayFunctions(array $numbers): array
    {
        $filtered = array_filter($numbers, fn($n) => $n > 0);
        return array_map(fn($n) => $n * 10, $filtered);
    }
}
