# Updating PHP Runtime Stubs

The canonical updater command is:

```powershell
cargo run --locked -p axiom-php --bin axiom-php-stub-updater
```

The command is manual and may access the network. No Python installation is required.
Normal Cargo builds and tests remain offline.
The updater reads `phpstorm-stubs.lock`, downloads the pinned archive, validates SHA-256, extracts
to a temporary directory outside the repository, and atomically replaces `../crates/axiom-php/embedded/runtime-stubs.bin`.
Cargo embeds this binary artifact into the application; `embedded/runtime-stubs.json` is not used
by the normal build or runtime loading path.

## Removing runtime stubs

To stop using an external stub checkout, first inspect the configured path, then remove that
directory only if you intend to delete the checkout itself:

```powershell
$stubPath = $env:AXIOM_PHP_STUBS
if (-not $stubPath) { $stubPath = $env:RUSTSTORM_PHP_STUBS }
$stubPath # Verify this is the directory you intend to remove
if ($stubPath) { Remove-Item -LiteralPath $stubPath -Recurse -Force }
```

Clear the override from the current PowerShell session and your user environment to return to the
bundled stubs:

```powershell
Remove-Item Env:AXIOM_PHP_STUBS -ErrorAction SilentlyContinue
Remove-Item Env:RUSTSTORM_PHP_STUBS -ErrorAction SilentlyContinue
[Environment]::SetEnvironmentVariable('AXIOM_PHP_STUBS', $null, 'User')
[Environment]::SetEnvironmentVariable('RUSTSTORM_PHP_STUBS', $null, 'User')
```

To remove only Axiom's incremental external-stub cache (it will be recreated when needed):

```powershell
$stubCache = Join-Path $env:LOCALAPPDATA 'Axiom\Axiom\cache\php-stubs-index-v1.json'
Remove-Item -LiteralPath $stubCache -Force -ErrorAction SilentlyContinue
```

## Replacing the bundled set with your own stubs

For a one-command workflow from Bash (Linux, WSL, or Git Bash), use
`../scripts/update-php-stubs.sh` from the repository:

```bash
# Remove all bundled runtime completion symbols (the binary artifact remains valid but empty).
bash scripts/update-php-stubs.sh clear

# Build the default ./stubs folder and its AxiomStubsMap.php.
bash scripts/update-php-stubs.sh build

# Optionally use another folder/map and compile-check the app after replacing the artifact.
bash scripts/update-php-stubs.sh build /path/to/my-stubs /path/to/my-stubs/AxiomStubsMap.php --build-app
```

To export the extensions and symbols available in a local PHP CLI first, run the reflection
generator, then package its output with the updater. The PHP CLI version and loaded extensions
determine what is exported:

```bash
php scripts/generate-runtime-php-stubs.php
bash scripts/update-php-stubs.sh build
```

Both scripts default to the repository's `../stubs` directory. You do not need to pass
`/mnt/e/dev/Axiom/stubs` or the map path unless using a different location.

The reflection generator emits namespaced declarations, enum cases, class members, functions, and
representable global constants. Reflection cannot recover original source/docblocks or arbitrary
implementation details, so this is a runtime API stub export, not a replacement for curated
upstream stubs. Review its output before embedding it.

On Windows, if Git Bash cannot find Cargo in its `PATH`, set `CARGO_BIN` to the Cargo executable.
The script also searches for `cargo.exe` with `where.exe`. To use a specific Rust toolchain, set
`CARGO_TOOLCHAIN` (for example, `CARGO_TOOLCHAIN=1.88.0 bash scripts/update-php-stubs.sh clear`).

The script keeps a copy at `../crates/axiom-php/embedded/runtime-stubs.previous.bin`. `clear` creates
an empty generated artifact rather than deleting `runtime-stubs.bin`, because `build.rs` requires
that file to exist to compile the updater itself. Clearing or changing the source directory later
does not automatically change the embedded bundle; run the script again after changing the inputs.

The generated artifact contains only symbols from PHP files referenced by the map. Create a source
directory containing your `.php` stub files and an `AxiomStubsMap.php` like this (paths are relative
to the source directory):

```php
<?php
const CLASSES = array (
    'MyLibrary\\Client' => 'src/Client.php',
);
const FUNCTIONS = array (
    'my_library_function' => 'src/functions.php',
);
const CONSTANTS = array (
);
```

Generate a new binary to a separate file first, then replace the bundled artifact only after
generation succeeds:

```powershell
$repo = 'E:\dev\Axiom'
$source = 'E:\dev\my-php-stubs'
$embedded = Join-Path $repo 'crates\axiom-php\embedded'
$generated = Join-Path $embedded 'runtime-stubs.custom.bin'
$bundled = Join-Path $embedded 'runtime-stubs.bin'
$backup = Join-Path $embedded 'runtime-stubs.upstream-backup.bin'

Copy-Item -LiteralPath $bundled -Destination $backup -Force
cargo run --locked -p axiom-php --bin axiom-php-stub-updater -- generate `
  --root $source `
  --map (Join-Path $source 'AxiomStubsMap.php') `
  --output $generated
if ($LASTEXITCODE -ne 0) { throw 'Stub generation failed; bundled artifact was not replaced.' }

Move-Item -LiteralPath $generated -Destination $bundled -Force
cargo run --locked -p axiom-app -- .
```

This replaces the complete bundled symbol set; it does not merge with the old defaults. The
existing `runtime-stubs.bin` must remain in place until the updater has finished compiling and
generating the replacement. To restore upstream stubs, run the canonical updater command at the
top of this document, or copy the backup back to `runtime-stubs.bin`.
