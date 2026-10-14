<?php
declare(strict_types=1);

/**
 * Export declarations visible to the PHP CLI into Axiom's source + map format.
 * Run: php scripts/generate-runtime-php-stubs.php [output-directory]
 * Requires PHP 8.1+. Does not execute generated stubs (they shadow internal symbols).
 * Generates an additional language-construct manifest for Rust-side integration.
 */

if (PHP_VERSION_ID < 80100) {
    fwrite(STDERR, "PHP 8.1 or newer is required (running " . PHP_VERSION . ").\n");
    exit(2);
}

$repoRoot = dirname(__DIR__);
$requestedOutDir = $argv[1] ?? ($repoRoot . '/stubs');
$outParent = dirname($requestedOutDir);
if (!is_dir($outParent) && !mkdir($outParent, 0777, true) && !is_dir($outParent)) {
    throw new RuntimeException("Cannot create output parent directory: {$outParent}");
}
$outDir = rtrim((string) realpath($outParent), DIRECTORY_SEPARATOR) . DIRECTORY_SEPARATOR . basename($requestedOutDir);
$srcDir = $outDir . '/src';
if (!is_dir($srcDir) && !mkdir($srcDir, 0777, true) && !is_dir($srcDir)) {
    throw new RuntimeException("Cannot create output directory: {$srcDir}");
}

// Preserve all symbols returned by Reflection, including enum-provided methods.
// Language constructs are separate: they are not PHP functions.
$classes = [];
$functions = [];
$constants = [];
$usedFiles = [];
$extensions = get_loaded_extensions();
sort($extensions, SORT_STRING);

foreach ($extensions as $extensionName) {
    try {
        $extension = new ReflectionExtension($extensionName);
    } catch (ReflectionException) {
        continue;
    }

    $relativeFile = 'src/' . safeFileName($extensionName) . '.php';
    $namespaces = [];
    $add = static function (string $namespace, string $declaration) use (&$namespaces): void {
        $namespaces[$namespace][] = $declaration;
    };

    foreach ($extension->getFunctions() as $function) {
        $name = $function->getName();
        if (isset($functions[$name])) {
            continue;
        }
        $namespace = $function->getNamespaceName();
        $add($namespace, renderFunction($function));
        $functions[$name] = $relativeFile;
        $usedFiles[$relativeFile] = true;
    }

    foreach ($extension->getClasses() as $class) {
        $name = ltrim($class->getName(), '\\');
        if (isset($classes[$name]) || $class->isAnonymous()) {
            continue;
        }
        $namespace = $class->getNamespaceName();
        $add($namespace, renderClass($class));
        $classes[$name] = $relativeFile;
        $usedFiles[$relativeFile] = true;
    }

    foreach ($extension->getConstants() as $name => $value) {
        if (isset($constants[$name]) || !preg_match('~^[A-Za-z_][A-Za-z0-9_]*(?:[\\\\][A-Za-z_][A-Za-z0-9_]*)*$~D', (string) $name)) {
            continue;
        }
        $literal = literal($value);
        if ($literal === null) {
            continue;
        }
        $separator = strrpos((string) $name, '\\');
        $constantNamespace = $separator === false ? '' : substr((string) $name, 0, $separator);
        $constantShortName = $separator === false ? (string) $name : substr((string) $name, $separator + 1);
        $add($constantNamespace, 'const ' . $constantShortName . ' = ' . $literal . ';');
        $constants[$name] = $relativeFile;
        $usedFiles[$relativeFile] = true;
    }

    if (!$namespaces) {
        continue;
    }

    $source = "<?php\n// Runtime declarations exported from extension: {$extensionName}\n";
    ksort($namespaces, SORT_STRING);
    foreach ($namespaces as $namespace => $declarations) {
        $source .= $namespace === '' ? "\nnamespace {\n" : "\nnamespace {$namespace} {\n";
        foreach ($declarations as $declaration) {
            $source .= indentBlock($declaration, 4) . "\n\n";
        }
        $source .= "}\n";
    }
    writeFileAtomically($outDir . '/' . $relativeFile, $source);
}

// Avoid stale PHP files being accidentally included after regenerating into an existing folder.
foreach (glob($srcDir . '/*.php') ?: [] as $oldFile) {
    $relative = 'src/' . basename($oldFile);
    if (!isset($usedFiles[$relative])) {
        unlink($oldFile);
    }
}

// Register synthetic PHP language constructs for Axiom.
$constructFile = 'AxiomLanguageConstructs.php';

if (is_file($outDir . '/' . $constructFile)) {
    foreach ([
                     'array', 'isset', 'empty', 'unset', 'list',
                     'echo', 'print', 'include', 'include_once',
                     'require', 'require_once', 'eval', 'clone',
                     'match', 'yield', 'exit', 'die',
                     'if', 'else', 'elseif', 'foreach', 'for',
                     'while', 'switch', 'try', 'catch', 'finally'
             ] as $construct) {
        if (!isset($functions[$construct])) {
            $functions[$construct] = $constructFile;
        }
    }
}

$map = "<?php\nconst CLASSES = " . var_export($classes, true) . ";\n";
$map .= "const FUNCTIONS = " . var_export($functions, true) . ";\n";
$map .= "const CONSTANTS = " . var_export($constants, true) . ";\n";
writeFileAtomically($outDir . '/AxiomStubsMap.php', $map);

// Syntax constructs do not exist as ordinary PHP functions and cannot be declared
// inside valid PHP source as function array(), function isset(), etc.
// Emit a separate completion catalogue and an embeddable Rust module. The Rust
// completion provider must load/register it to expose these labels to users.
$constructSpecs = [
    // Expressions and constructs callable with parentheses.
        ['array', 'array()', 'array(${1})', 'expression'],
        ['isset', 'isset()', 'isset(${1:$variable})', 'expression'],
        ['empty', 'empty()', 'empty(${1:$value})', 'expression'],
        ['unset', 'unset()', 'unset(${1:$variable});', 'statement'],
        ['list', 'list()', 'list(${1:$a}, ${2:$b})', 'destructuring'],
        ['eval', 'eval()', 'eval(${1:$code});', 'expression'],
        ['exit', 'exit()', 'exit(${1:0});', 'expression'],
        ['die', 'die()', 'die(${1:0});', 'expression'],
        ['print', 'print', 'print ${1:$value};', 'expression'],
        ['echo', 'echo', 'echo ${1:$value};', 'statement'],
        ['include', 'include', 'include ${1:$file};', 'expression'],
        ['include_once', 'include_once', 'include_once ${1:$file};', 'expression'],
        ['require', 'require', 'require ${1:$file};', 'expression'],
        ['require_once', 'require_once', 'require_once ${1:$file};', 'expression'],
        ['new', 'new', 'new ${1:ClassName}(${2})', 'expression'],
        ['clone', 'clone', 'clone ${1:$object}', 'expression'],
        ['match', 'match', "match (\${1:\$value}) {\n    \${2:default} => \${3:null},\n}", 'expression'],
        ['yield', 'yield', 'yield ${1:$value}', 'expression'],
        ['yield from', 'yield from', 'yield from ${1:$iterable}', 'expression'],
        ['throw', 'throw', 'throw new ${1:Exception}(${2});', 'statement'],
        ['return', 'return', 'return ${1:$value};', 'statement'],
        ['break', 'break', 'break;', 'statement'],
        ['continue', 'continue', 'continue;', 'statement'],
        ['global', 'global', 'global ${1:$variable};', 'statement'],
        ['static', 'static', 'static ${1:$variable};', 'statement'],
        ['instanceof', 'instanceof', '${1:$value} instanceof ${2:ClassName}', 'operator'],
        ['fn', 'fn', 'fn(${1:$value}) => ${2:$value}', 'expression'],
        ['function', 'function', 'function ${1:name}(${2}): ${3:void} {\n    ${4}\n}', 'declaration'],
        ['use', 'use', 'use ${1:Vendor\\Package\\ClassName};', 'declaration'],
        ['namespace', 'namespace', 'namespace ${1:App\\Example};', 'declaration'],
        ['class', 'class', 'class ${1:Name} {\n    ${2}\n}', 'declaration'],
        ['interface', 'interface', 'interface ${1:Name} {\n    ${2}\n}', 'declaration'],
        ['trait', 'trait', 'trait ${1:Name} {\n    ${2}\n}', 'declaration'],
        ['enum', 'enum', 'enum ${1:Name} {\n    ${2}\n}', 'declaration'],
        ['if', 'if', 'if (${1:condition}) {\n    ${2}\n}', 'control'],
        ['elseif', 'elseif', 'elseif (${1:condition}) {\n    ${2}\n}', 'control'],
        ['else', 'else', 'else {\n    ${1}\n}', 'control'],
        ['foreach', 'foreach', 'foreach (${1:$items} as ${2:$item}) {\n    ${3}\n}', 'control'],
        ['for', 'for', 'for (${1:$i = 0}; ${2:$i < 10}; ${3:$i++}) {\n    ${4}\n}', 'control'],
        ['while', 'while', 'while (${1:condition}) {\n    ${2}\n}', 'control'],
        ['do', 'do', 'do {\n    ${1}\n} while (${2:condition});', 'control'],
        ['switch', 'switch', 'switch (${1:$value}) {\n    case ${2:1}:\n        ${3:break;}\n}', 'control'],
        ['try', 'try', 'try {\n    ${1}\n} catch (${2:\\Throwable} ${3:$e}) {\n    ${4}\n}', 'control'],
        ['catch', 'catch', 'catch (${1:\\Throwable} ${2:$e}) {\n    ${3}\n}', 'control'],
        ['finally', 'finally', 'finally {\n    ${1}\n}', 'control'],
        ['declare', 'declare', 'declare(strict_types=1);', 'statement'],
        ['array_key_exists', 'array_key_exists()', 'array_key_exists(${1:$key}, ${2:$array})', 'function'],
];
$constructs = [];
$seenTokens = [];
foreach ($constructSpecs as [$token, $label, $snippet, $kind]) {
    // Real functions already exist in Reflection and must not be duplicated.
    if ($kind === 'function' || isset($seenTokens[$token])) continue;
    $seenTokens[$token] = true;
    $snippet = str_replace('\\n', "\n", $snippet);
    // Snippets follow LSP snippet syntax (dollar-sign placeholders).
    // Use a plain-text fallback for providers that don't implement snippets.
    $plainText = preg_replace('/\\$\\{\\d+(?::([^}]*))?\\}/', '$1', $snippet);
    $constructs[] = [
            'token' => $token,
            'label' => $label,
            'insertText' => $plainText,
            'insertTextSnippet' => $snippet,
            'kind' => $kind,
    ];
}
$manifest = [
        'schemaVersion' => 2,
        'phpVersion' => PHP_VERSION,
        'note' => 'This catalogue requires Rust completion-provider integration; the updater does not load it.',
        'constructs' => $constructs,
];
writeFileAtomically($outDir . '/AxiomLanguageConstructs.json', json_encode($manifest, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR) . "\n");

// A Rust source module can be included through mod / include! in the provider.
// No extra Rust dependencies are needed. This does NOT automatically register completions.
$rust = "// Generated by scripts/generate-runtime-php-stubs.php; do not edit.\n";
$rust .= "// Register LANGUAGE_CONSTRUCTS in the Axiom completion provider.\n";
$rust .= "pub struct LanguageConstruct {\n";
$rust .= "    pub token: &'static str,\n    pub label: &'static str,\n    pub insert_text: &'static str,\n    pub snippet: &'static str,\n    pub kind: &'static str,\n}\n";
$rust .= "pub const LANGUAGE_CONSTRUCTS: &[LanguageConstruct] = &[\n";
foreach ($constructs as $construct) {
    $values = [];
    foreach (['token', 'label', 'insertText', 'insertTextSnippet', 'kind'] as $field) {
        $values[] = json_encode($construct[$field], JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR);
    }
    $rust .= '    LanguageConstruct { token: ' . $values[0] . ', label: ' . $values[1]
            . ', insert_text: ' . $values[2] . ', snippet: ' . $values[3]
            . ', kind: ' . $values[4] . " },\n";
}
$rust .= "];\n";
writeFileAtomically($outDir . '/AxiomLanguageConstructs.rs', $rust);


printf(
        "Generated %d classes, %d functions, and %d global constants in %s\n",
        count($classes), count($functions), count($constants), $outDir
);

function safeFileName(string $name): string
{
    return preg_replace('/[^A-Za-z0-9_.-]+/', '_', $name) ?: 'extension';
}

function renderFunction(ReflectionFunction $function): string
{
    $returnByReference = $function->returnsReference() ? '&' : '';
    $returnType = reflectionType($function->getReturnType());
    return 'function ' . $returnByReference . $function->getShortName() . '(' . renderParameters($function->getParameters()) . ')'
            . ($returnType !== '' ? ': ' . $returnType : '') . ' {}';
}

function renderClass(ReflectionClass $class): string
{
    $name = $class->getShortName();
    $prefix = '';
    if ($class->isInterface()) {
        $kind = 'interface';
    } elseif ($class->isTrait()) {
        $kind = 'trait';
    } elseif (method_exists($class, 'isEnum') && $class->isEnum()) {
        $kind = 'enum';
    } else {
        $kind = 'class';
        if ($class->isAbstract()) $prefix .= 'abstract ';
        if ($class->isFinal()) $prefix .= 'final ';
        if (method_exists($class, 'isReadOnly') && $class->isReadOnly()) $prefix .= 'readonly ';
    }

    $header = $prefix . $kind . ' ' . $name;
    if ($kind === 'enum' && $class instanceof ReflectionEnum && ($backing = $class->getBackingType())) {
        $header .= ': ' . reflectionType($backing);
    } elseif ($kind === 'class' && ($parent = $class->getParentClass())) {
        $header .= ' extends \\' . ltrim($parent->getName(), '\\');
    }

    $interfaces = $class->getInterfaceNames();
    if ($interfaces) {
        $keyword = $kind === 'interface' ? ' extends ' : ' implements ';
        $header .= $keyword . implode(', ', array_map(static fn(string $i): string => '\\' . ltrim($i, '\\'), $interfaces));
    }

    $body = $header . " {\n";
    $enumCases = [];
    if ($kind === 'enum' && $class instanceof ReflectionEnum) {
        foreach ($class->getCases() as $case) {
            $enumCases[$case->getName()] = true;
            $value = $case instanceof ReflectionEnumBackedCase ? ' = ' . literal($case->getBackingValue()) : '';
            $body .= '    case ' . $case->getName() . $value . ";\n";
        }
    }

    foreach ($class->getReflectionConstants() as $constant) {
        if ($constant->getDeclaringClass()->getName() !== $class->getName() || isset($enumCases[$constant->getName()])) continue;
        $literal = literal($constant->getValue());
        if ($literal === null) continue;
        $modifiers = Reflection::getModifierNames($constant->getModifiers());
        $visibility = firstVisibility($modifiers);
        $final = in_array('final', $modifiers, true) ? 'final ' : '';
        $body .= "    {$final}{$visibility}const {$constant->getName()} = {$literal};\n";
    }

    foreach ($class->getProperties() as $property) {
        if ($property->getDeclaringClass()->getName() !== $class->getName()) continue;
        // Engine-created enum properties are not legal declarations in an enum body.
        if ($kind === 'enum' && in_array($property->getName(), ['name', 'value'], true)) continue;
        $modifiers = Reflection::getModifierNames($property->getModifiers());
        $parts = [firstVisibility($modifiers)];
        foreach (['static', 'readonly'] as $modifier) {
            if (in_array($modifier, $modifiers, true)) $parts[] = $modifier;
        }
        $writeVisibility = propertyWriteVisibility($property);
        if ($writeVisibility !== null && visibilityRank($writeVisibility) > visibilityRank(firstVisibility($modifiers))) {
            $parts[] = $writeVisibility . '(set)';
        }
        $type = reflectionType($property->getType());
        $declaration = '    ' . implode(' ', array_filter($parts)) . ' ' . ($type !== '' ? $type . ' ' : '') . '$' . $property->getName();
        if (method_exists($property, 'hasDefaultValue') && $property->hasDefaultValue()) {
            $value = literal($property->getDefaultValue());
            if ($value !== null) $declaration .= ' = ' . $value;
        }
        $declaration .= ';';
        $hooks = method_exists($property, 'getHooks') ? array_keys($property->getHooks()) : [];
        if ($hooks) {
            $declaration = rtrim($declaration, ';') . " {\n";
            foreach ($hooks as $hook) {
                if ($hook === 'get') $declaration .= '        get { throw new \\LogicException(); }' . "\n";
                if ($hook === 'set') $declaration .= '        set { throw new \\LogicException(); }' . "\n";
            }
            $declaration .= '    }';
        }
        $body .= $declaration . "\n";
    }

    foreach ($class->getMethods() as $method) {
        if ($method->getDeclaringClass()->getName() !== $class->getName()) continue;
        $mods = Reflection::getModifierNames($method->getModifiers());
        $methodPrefix = firstVisibility($mods);
        foreach (['static', 'final', 'abstract'] as $modifier) {
            if (in_array($modifier, $mods, true) && !($kind === 'interface' && $modifier === 'abstract')) $methodPrefix .= $modifier . ' ';
        }
        $byRef = $method->returnsReference() ? '&' : '';
        $returnType = reflectionType($method->getReturnType());
        $signature = '    ' . $methodPrefix . 'function ' . $byRef . $method->getName() . '(' . renderParameters($method->getParameters()) . ')'
                . ($returnType !== '' ? ': ' . $returnType : '');
        if ($kind === 'interface' || $method->isAbstract()) $body .= $signature . ";\n";
        else $body .= $signature . " {}\n";
    }
    return $body . '}';
}

function renderParameters(array $parameters): string
{
    $rendered = [];
    foreach ($parameters as $parameter) {
        $part = reflectionType($parameter->getType());
        if ($part !== '') $part .= ' ';
        if ($parameter->isPassedByReference()) $part .= '&';
        if ($parameter->isVariadic()) $part .= '...';
        $part .= '$' . $parameter->getName();
        if ($parameter->isOptional() && !$parameter->isVariadic()) {
            $default = null;
            if ($parameter->isDefaultValueAvailable()) {
                try {
                    $default = $parameter->isDefaultValueConstant()
                            ? '\\' . ltrim((string) $parameter->getDefaultValueConstantName(), '\\')
                            : literal($parameter->getDefaultValue());
                } catch (Throwable) {
                    $default = null;
                }
            }
            $part .= ' = ' . ($default ?? 'null');
        }
        $rendered[] = $part;
    }
    return implode(', ', $rendered);
}

function reflectionType(?ReflectionType $type): string
{
    if ($type === null) return '';
    if ($type instanceof ReflectionUnionType) {
        return implode('|', array_map(static function (ReflectionType $member): string {
            $value = reflectionType($member);
            return $member instanceof ReflectionIntersectionType ? '(' . $value . ')' : $value;
        }, $type->getTypes()));
    }
    if ($type instanceof ReflectionIntersectionType) {
        return implode('&', array_map(static fn(ReflectionType $member): string => reflectionType($member), $type->getTypes()));
    }
    if (!$type instanceof ReflectionNamedType) return '';
    $name = $type->getName();
    $builtins = ['array', 'bool', 'callable', 'false', 'float', 'int', 'iterable', 'mixed', 'never', 'null', 'object', 'string', 'true', 'void'];
    if (!in_array(strtolower($name), $builtins, true) && !in_array($name, ['self', 'parent', 'static'], true)) {
        $name = '\\' . ltrim($name, '\\');
    }
    if ($type->allowsNull() && strtolower($name) !== 'mixed' && strtolower($name) !== 'null' && strtolower($name) !== 'void' && strtolower($name) !== 'never' && strtolower($name) !== 'false' && strtolower($name) !== 'true') {
        return '?' . $name;
    }
    return $name;
}

function literal(mixed $value): ?string
{
    if ($value instanceof UnitEnum) {
        return '\\' . ltrim($value::class, '\\') . '::' . $value->name;
    }
    if (is_null($value) || is_bool($value) || is_int($value) || is_string($value)) return var_export($value, true);
    if (is_float($value)) {
        if (is_nan($value)) return 'NAN';
        if ($value === INF) return 'INF';
        if ($value === -INF) return '-INF';
        return var_export($value, true);
    }
    if (is_array($value)) {
        $items = [];
        foreach ($value as $key => $item) {
            $rendered = literal($item);
            if ($rendered === null) return null;
            $items[] = literal($key) . ' => ' . $rendered;
        }
        return 'array (' . ($items ? "\n    " . implode(",\n    ", $items) . "\n" : '') . ')';
    }
    return null;
}

function firstVisibility(array $modifiers): string
{
    foreach (['public', 'protected', 'private'] as $visibility) {
        if (in_array($visibility, $modifiers, true)) return $visibility . ' ';
    }
    return 'public ';
}

function propertyWriteVisibility(ReflectionProperty $property): ?string
{
    foreach (['isPrivateSet' => 'private', 'isProtectedSet' => 'protected', 'isPublicSet' => 'public'] as $method => $visibility) {
        if (method_exists($property, $method) && $property->$method()) return $visibility;
    }
    return null;
}

function visibilityRank(string $visibility): int
{
    return match (trim($visibility)) { 'public' => 0, 'protected' => 1, 'private' => 2, default => -1 };
}

function indentBlock(string $text, int $spaces): string
{
    $prefix = str_repeat(' ', $spaces);
    return $prefix . str_replace("\n", "\n{$prefix}", rtrim($text));
}

function writeFileAtomically(string $path, string $contents): void
{
    $directory = dirname($path);
    if (!is_dir($directory) && !mkdir($directory, 0777, true) && !is_dir($directory)) {
        throw new RuntimeException("Cannot create directory: {$directory}");
    }
    $temporary = $path . '.tmp-' . getmypid();
    if (file_put_contents($temporary, $contents, LOCK_EX) === false) {
        @unlink($temporary);
        throw new RuntimeException("Cannot write generated file: {$path}");
    }
    // PHP's rename() may not replace an existing destination on Windows.
    if (file_exists($path) && !unlink($path)) {
        @unlink($temporary);
        throw new RuntimeException("Cannot replace generated file: {$path}");
    }
    if (!rename($temporary, $path)) {
        @unlink($temporary);
        throw new RuntimeException("Cannot publish generated file: {$path}");
    }
}