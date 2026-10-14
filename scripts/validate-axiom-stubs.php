<?php
declare(strict_types=1);
// Usage: php validate-axiom-stubs.php /path/to/stubs [minimum-functions] [minimum-classes] [minimum-constants]
$root = $argv[1] ?? (__DIR__ . '/stubs');
$mapFile = rtrim($root, '/\\') . '/AxiomStubsMap.php';
if (!is_file($mapFile)) { fwrite(STDERR, "Map missing: $mapFile\n"); exit(1); }
require $mapFile;
$groups = ['CLASSES' => CLASSES, 'FUNCTIONS' => FUNCTIONS, 'CONSTANTS' => CONSTANTS];
$fail = false;
foreach (array_values(array_keys($groups)) as $i => $index) {
    $symbols = $groups[$index];
    $count = count($symbols);
    $min = isset($argv[$i + 2]) ? (int) $argv[$i + 2] : 0;
    printf("%-12s %6d (minimum %d)\n", $index, $count, $min);
    if ($count < $min) { fwrite(STDERR, "REGRESSION $index\n"); $fail = true; }
    foreach ($symbols as $symbol => $file) {
        if (!is_string($symbol) || $symbol === '' || !is_string($file) || !str_starts_with($file, 'src/') || !is_file($root . '/' . $file)) {
            fwrite(STDERR, "Invalid $index entry: " . var_export($symbol, true) . " => " . var_export($file, true) . "\n");
            $fail = true;
        }
    }
}
$manifestFile = rtrim($root, '/\\') . '/AxiomLanguageConstructs.json';
if (is_file($manifestFile)) {
    $manifest = json_decode((string) file_get_contents($manifestFile), true, flags: JSON_THROW_ON_ERROR);
    $tokens = array_column($manifest['constructs'] ?? [], 'token');
    printf("Language construct entries: %d (NOT included in runtime-stubs.bin by default)\n", count($tokens));
    if (!in_array('array', $tokens, true)) { fwrite(STDERR, "Missing array construct\n"); $fail = true; }
}
exit($fail ? 1 : 0);
