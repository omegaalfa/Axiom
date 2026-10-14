<?php
// Runtime declarations exported from extension: Reflection

namespace {
    class ReflectionException extends \Exception implements \Throwable, \Stringable {
    }

    class Reflection {
        public static function getModifierNames(int $modifiers) {}
    }

    interface Reflector extends \Stringable {
    }

    abstract class ReflectionFunctionAbstract implements \Reflector, \Stringable {
        public  string $name;
        private function __clone(): void {}
        public function inNamespace() {}
        public function isClosure() {}
        public function isDeprecated() {}
        public function isInternal() {}
        public function isUserDefined() {}
        public function isGenerator() {}
        public function isVariadic() {}
        public function isStatic() {}
        public function getClosureThis() {}
        public function getClosureScopeClass() {}
        public function getClosureCalledClass() {}
        public function getClosureUsedVariables(): array {}
        public function getDocComment() {}
        public function getEndLine() {}
        public function getExtension() {}
        public function getExtensionName() {}
        public function getFileName() {}
        public function getName() {}
        public function getNamespaceName() {}
        public function getNumberOfParameters() {}
        public function getNumberOfRequiredParameters() {}
        public function getParameters() {}
        public function getShortName() {}
        public function getStartLine() {}
        public function getStaticVariables() {}
        public function returnsReference() {}
        public function hasReturnType() {}
        public function getReturnType() {}
        public function hasTentativeReturnType(): bool {}
        public function getTentativeReturnType(): ?\ReflectionType {}
        public function getAttributes(?string $name = NULL, int $flags = 0): array {}
    }

    class ReflectionFunction extends \ReflectionFunctionAbstract implements \Stringable, \Reflector {
        public const IS_DEPRECATED = 2048;
        public function __construct(\Closure|string $function) {}
        public function __toString(): string {}
        public function isAnonymous(): bool {}
        public function isDisabled() {}
        public function invoke(mixed ...$args) {}
        public function invokeArgs(array $args) {}
        public function getClosure() {}
    }

    final class ReflectionGenerator {
        public function __construct(\Generator $generator) {}
        public function getExecutingLine(): int {}
        public function getExecutingFile(): string {}
        public function getTrace(int $options = \DEBUG_BACKTRACE_PROVIDE_OBJECT): array {}
        public function getFunction(): \ReflectionFunctionAbstract {}
        public function getThis(): ?object {}
        public function getExecutingGenerator(): \Generator {}
        public function isClosed(): bool {}
    }

    class ReflectionParameter implements \Stringable, \Reflector {
        public  string $name;
        private function __clone(): void {}
        public function __construct($function, string|int $param) {}
        public function __toString(): string {}
        public function getName() {}
        public function isPassedByReference() {}
        public function canBePassedByValue() {}
        public function getDeclaringFunction() {}
        public function getDeclaringClass() {}
        public function getClass() {}
        public function hasType() {}
        public function getType() {}
        public function isArray() {}
        public function isCallable() {}
        public function allowsNull() {}
        public function getPosition() {}
        public function isOptional() {}
        public function isDefaultValueAvailable() {}
        public function getDefaultValue() {}
        public function isDefaultValueConstant() {}
        public function getDefaultValueConstantName() {}
        public function isVariadic() {}
        public function isPromoted(): bool {}
        public function getAttributes(?string $name = NULL, int $flags = 0): array {}
    }

    abstract class ReflectionType implements \Stringable {
        private function __clone(): void {}
        public function allowsNull() {}
        public function __toString(): string {}
    }

    class ReflectionNamedType extends \ReflectionType implements \Stringable {
        public function getName() {}
        public function isBuiltin() {}
    }

    class ReflectionUnionType extends \ReflectionType implements \Stringable {
        public function getTypes(): array {}
    }

    class ReflectionIntersectionType extends \ReflectionType implements \Stringable {
        public function getTypes(): array {}
    }

    class ReflectionMethod extends \ReflectionFunctionAbstract implements \Stringable, \Reflector {
        public const IS_STATIC = 16;
        public const IS_PUBLIC = 1;
        public const IS_PROTECTED = 2;
        public const IS_PRIVATE = 4;
        public const IS_ABSTRACT = 64;
        public const IS_FINAL = 32;
        public  string $class;
        public function __construct(object|string $objectOrMethod, ?string $method = NULL) {}
        public static function createFromMethodName(string $method): static {}
        public function __toString(): string {}
        public function isPublic() {}
        public function isPrivate() {}
        public function isProtected() {}
        public function isAbstract() {}
        public function isFinal() {}
        public function isConstructor() {}
        public function isDestructor() {}
        public function getClosure(?object $object = NULL) {}
        public function getModifiers() {}
        public function invoke(?object $object, mixed ...$args) {}
        public function invokeArgs(?object $object, array $args) {}
        public function getDeclaringClass() {}
        public function getPrototype() {}
        public function hasPrototype(): bool {}
        public function setAccessible(bool $accessible) {}
    }

    class ReflectionClass implements \Stringable, \Reflector {
        public const IS_IMPLICIT_ABSTRACT = 16;
        public const IS_EXPLICIT_ABSTRACT = 64;
        public const IS_FINAL = 32;
        public const IS_READONLY = 65536;
        public const SKIP_INITIALIZATION_ON_SERIALIZE = 8;
        public const SKIP_DESTRUCTOR = 16;
        public  string $name;
        private function __clone(): void {}
        public function __construct(object|string $objectOrClass) {}
        public function __toString(): string {}
        public function getName() {}
        public function isInternal() {}
        public function isUserDefined() {}
        public function isAnonymous() {}
        public function isInstantiable() {}
        public function isCloneable() {}
        public function getFileName() {}
        public function getStartLine() {}
        public function getEndLine() {}
        public function getDocComment() {}
        public function getConstructor() {}
        public function hasMethod(string $name) {}
        public function getMethod(string $name) {}
        public function getMethods(?int $filter = NULL) {}
        public function hasProperty(string $name) {}
        public function getProperty(string $name) {}
        public function getProperties(?int $filter = NULL) {}
        public function hasConstant(string $name) {}
        public function getConstants(?int $filter = NULL) {}
        public function getReflectionConstants(?int $filter = NULL) {}
        public function getConstant(string $name) {}
        public function getReflectionConstant(string $name) {}
        public function getInterfaces() {}
        public function getInterfaceNames() {}
        public function isInterface() {}
        public function getTraits() {}
        public function getTraitNames() {}
        public function getTraitAliases() {}
        public function isTrait() {}
        public function isEnum(): bool {}
        public function isAbstract() {}
        public function isFinal() {}
        public function isReadOnly(): bool {}
        public function getModifiers() {}
        public function isInstance(object $object) {}
        public function newInstance(mixed ...$args) {}
        public function newInstanceWithoutConstructor() {}
        public function newInstanceArgs(array $args = array ()) {}
        public function newLazyGhost(callable $initializer, int $options = 0): object {}
        public function newLazyProxy(callable $factory, int $options = 0): object {}
        public function resetAsLazyGhost(object $object, callable $initializer, int $options = 0): void {}
        public function resetAsLazyProxy(object $object, callable $factory, int $options = 0): void {}
        public function initializeLazyObject(object $object): object {}
        public function isUninitializedLazyObject(object $object): bool {}
        public function markLazyObjectAsInitialized(object $object): object {}
        public function getLazyInitializer(object $object): ?callable {}
        public function getParentClass() {}
        public function isSubclassOf(\ReflectionClass|string $class) {}
        public function getStaticProperties() {}
        public function getStaticPropertyValue(string $name, mixed $default = null) {}
        public function setStaticPropertyValue(string $name, mixed $value) {}
        public function getDefaultProperties() {}
        public function isIterable() {}
        public function isIterateable() {}
        public function implementsInterface(\ReflectionClass|string $interface) {}
        public function getExtension() {}
        public function getExtensionName() {}
        public function inNamespace() {}
        public function getNamespaceName() {}
        public function getShortName() {}
        public function getAttributes(?string $name = NULL, int $flags = 0): array {}
    }

    class ReflectionObject extends \ReflectionClass implements \Reflector, \Stringable {
        public function __construct(object $object) {}
    }

    class ReflectionProperty implements \Stringable, \Reflector {
        public const IS_STATIC = 16;
        public const IS_READONLY = 128;
        public const IS_PUBLIC = 1;
        public const IS_PROTECTED = 2;
        public const IS_PRIVATE = 4;
        public const IS_ABSTRACT = 64;
        public const IS_PROTECTED_SET = 2048;
        public const IS_PRIVATE_SET = 4096;
        public const IS_VIRTUAL = 512;
        public const IS_FINAL = 32;
        public  string $name;
        public  string $class;
        private function __clone(): void {}
        public function __construct(object|string $class, string $property) {}
        public function __toString(): string {}
        public function getName() {}
        public function getMangledName(): string {}
        public function getValue(?object $object = NULL) {}
        public function setValue(mixed $objectOrValue, mixed $value = null) {}
        public function getRawValue(object $object): mixed {}
        public function setRawValue(object $object, mixed $value): void {}
        public function setRawValueWithoutLazyInitialization(object $object, mixed $value): void {}
        public function skipLazyInitialization(object $object): void {}
        public function isLazy(object $object): bool {}
        public function isInitialized(?object $object = NULL) {}
        public function isPublic() {}
        public function isPrivate() {}
        public function isProtected() {}
        public function isPrivateSet(): bool {}
        public function isProtectedSet(): bool {}
        public function isStatic() {}
        public function isReadOnly(): bool {}
        public function isDefault() {}
        public function isDynamic(): bool {}
        public function isAbstract(): bool {}
        public function isVirtual(): bool {}
        public function isPromoted(): bool {}
        public function getModifiers() {}
        public function getDeclaringClass() {}
        public function getDocComment() {}
        public function setAccessible(bool $accessible) {}
        public function getType() {}
        public function getSettableType(): ?\ReflectionType {}
        public function hasType() {}
        public function hasDefaultValue(): bool {}
        public function getDefaultValue() {}
        public function getAttributes(?string $name = NULL, int $flags = 0): array {}
        public function hasHooks(): bool {}
        public function getHooks(): array {}
        public function hasHook(\PropertyHookType $type): bool {}
        public function getHook(\PropertyHookType $type): ?\ReflectionMethod {}
        public function isFinal(): bool {}
    }

    class ReflectionClassConstant implements \Stringable, \Reflector {
        public const IS_PUBLIC = 1;
        public const IS_PROTECTED = 2;
        public const IS_PRIVATE = 4;
        public const IS_FINAL = 32;
        public  string $name;
        public  string $class;
        private function __clone(): void {}
        public function __construct(object|string $class, string $constant) {}
        public function __toString(): string {}
        public function getName() {}
        public function getValue() {}
        public function isPublic() {}
        public function isPrivate() {}
        public function isProtected() {}
        public function isFinal(): bool {}
        public function getModifiers() {}
        public function getDeclaringClass() {}
        public function getDocComment() {}
        public function getAttributes(?string $name = NULL, int $flags = 0): array {}
        public function isEnumCase(): bool {}
        public function isDeprecated(): bool {}
        public function hasType(): bool {}
        public function getType(): ?\ReflectionType {}
    }

    class ReflectionExtension implements \Stringable, \Reflector {
        public  string $name;
        private function __clone(): void {}
        public function __construct(string $name) {}
        public function __toString(): string {}
        public function getName() {}
        public function getVersion() {}
        public function getFunctions() {}
        public function getConstants() {}
        public function getINIEntries() {}
        public function getClasses() {}
        public function getClassNames() {}
        public function getDependencies() {}
        public function info() {}
        public function isPersistent() {}
        public function isTemporary() {}
    }

    class ReflectionZendExtension implements \Stringable, \Reflector {
        public  string $name;
        private function __clone(): void {}
        public function __construct(string $name) {}
        public function __toString(): string {}
        public function getName() {}
        public function getVersion() {}
        public function getAuthor() {}
        public function getURL() {}
        public function getCopyright() {}
    }

    final class ReflectionReference {
        public static function fromArrayElement(array $array, string|int $key): ?\ReflectionReference {}
        public function getId(): string {}
        private function __clone(): void {}
        private function __construct() {}
    }

    class ReflectionAttribute implements \Stringable, \Reflector {
        public const IS_INSTANCEOF = 2;
        public  string $name;
        public function getName(): string {}
        public function getTarget(): int {}
        public function isRepeated(): bool {}
        public function getArguments(): array {}
        public function newInstance(): object {}
        public function __toString(): string {}
        private function __clone(): void {}
        private function __construct() {}
    }

    class ReflectionEnum extends \ReflectionClass implements \Reflector, \Stringable {
        public function __construct(object|string $objectOrClass) {}
        public function hasCase(string $name): bool {}
        public function getCase(string $name): \ReflectionEnumUnitCase {}
        public function getCases(): array {}
        public function isBacked(): bool {}
        public function getBackingType(): ?\ReflectionNamedType {}
    }

    class ReflectionEnumUnitCase extends \ReflectionClassConstant implements \Reflector, \Stringable {
        public function __construct(object|string $class, string $constant) {}
        public function getEnum(): \ReflectionEnum {}
        public function getValue(): \UnitEnum {}
    }

    class ReflectionEnumBackedCase extends \ReflectionEnumUnitCase implements \Stringable, \Reflector {
        public function __construct(object|string $class, string $constant) {}
        public function getBackingValue(): string|int {}
    }

    final class ReflectionFiber {
        public function __construct(\Fiber $fiber) {}
        public function getFiber(): \Fiber {}
        public function getExecutingFile(): ?string {}
        public function getExecutingLine(): ?int {}
        public function getCallable(): callable {}
        public function getTrace(int $options = \DEBUG_BACKTRACE_PROVIDE_OBJECT): array {}
    }

    class ReflectionConstant implements \Stringable, \Reflector {
        public  string $name;
        public function __construct(string $name) {}
        public function getName(): string {}
        public function getNamespaceName(): string {}
        public function getShortName(): string {}
        public function getValue(): mixed {}
        public function isDeprecated(): bool {}
        public function getFileName(): string|false {}
        public function getExtension(): ?\ReflectionExtension {}
        public function getExtensionName(): string|false {}
        public function __toString(): string {}
        public function getAttributes(?string $name = NULL, int $flags = 0): array {}
    }

    enum PropertyHookType: string implements \BackedEnum, \UnitEnum {
        case Get = 'get';
        case Set = 'set';
        public static function cases(): array {}
        public static function from(string|int $value): static {}
        public static function tryFrom(string|int $value): ?static {}
    }

}
