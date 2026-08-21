<?php

/**
 * Returns the length of a string.
 * @param string $string Input value.
 * @return int<0, max>
 * @throws ValueError
 * @deprecated Demonstration fixture only.
 */
function strlen(string $string): int {}

#[Since('7.0')]
function array_map(?callable $callback, array $array, array ...$arrays): array {}

const PHP_VERSION_ID = 80400;
