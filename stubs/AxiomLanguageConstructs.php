<?php
/**
 * Axiom PHP language constructs — synthetic completion stubs.
 *
 * IMPORTANT: This file intentionally uses reserved PHP keywords as pseudo-function
 * names. It is NOT valid executable PHP and must NOT be included, required,
 * executed, or validated with `php -l`.
 *
 * It is intended ONLY for a stub parser capable of recognizing synthetic
 * keyword declarations. Axiom's current updater may skip/reject these entries.
 * Map integration alone does not guarantee that completion will work.
 */
// Runtime declarations exported from PHP language syntax (synthetic; not an extension).

namespace {
    // Arrays and destructuring
    function array(mixed ...$values): array {}
    function list(mixed ...$variables): array {}

    // Value and variable checks
    function isset(mixed $variable, mixed ...$variables): bool {}
    function empty(mixed $value): bool {}
    function unset(mixed $variable, mixed ...$variables): void {}
    function eval(string $code): mixed {}

    // Process termination
    function exit(string|int $status = 0): never {}
    function die(string|int $status = 0): never {}

    // Output
    function print(string $expression): int {}
    function echo(string ...$expressions): void {}

    // File inclusion
    function include(string $filename): mixed {}
    function include_once(string $filename): mixed {}
    function require(string $filename): mixed {}
    function require_once(string $filename): mixed {}

    // Expressions and object operations
    function new(string $className, mixed ...$arguments): object {}
    function clone(object $object, array $withProperties = array ()): object {}
    function match(mixed $subject): mixed {}
    function instanceof(mixed $expression, string|object $class): bool {}
    function fn(mixed ...$parameters): mixed {}

    // Generators and flow
    function yield(mixed $value = null): mixed {}
    function yield_from(iterable $values): mixed {}
    function throw(\Throwable $exception): never {}
    function return(mixed $value = null): mixed {}
    function break(int $levels = 1): void {}
    function continue(int $levels = 1): void {}

    // Scope and declarations (synthetic completion signatures)
    function global(mixed ...$variables): void {}
    function static(mixed $variable, mixed $initialValue = null): void {}
    function function(string $name, mixed ...$parameters): void {}
    function use(string $qualifiedName): void {}
    function namespace(string $name): void {}
    function class(string $name): void {}
    function interface(string $name): void {}
    function trait(string $name): void {}
    function enum(string $name): void {}

    // Conditions, iteration and branching
    function if(bool $condition): void {}
    function elseif(bool $condition): void {}
    function else(): void {}
    function foreach(iterable $iterable, mixed $value): void {}
    function for(mixed $initialization, mixed $condition, mixed $iteration): void {}
    function while(bool $condition): void {}
    function do(): void {}
    function switch(mixed $expression): void {}

    // Exception handling
    function try(): void {}
    function catch(string $exceptionType, mixed $exception): void {}
    function finally(): void {}

    // Compiler directives
    function declare(string $directive, mixed $value): void {}
}
