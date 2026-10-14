#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  scripts/update-php-stubs.sh clear [--build-app]
  scripts/update-php-stubs.sh build [stub-folder AxiomStubsMap.php] [--build-app]

clear     Replace the embedded stub set with an empty set.
build     Build the embedded stub set from PHP files referenced by the map.
--build-app  Also compile axiom-app after replacing the embedded artifact.
EOF
}

fail() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}

to_bash_path() {
    local path="$1"
    if command -v cygpath >/dev/null 2>&1; then
        cygpath -u "$path"
        return
    fi
    if command -v wslpath >/dev/null 2>&1; then
        wslpath -u "$path"
        return
    fi

    path="${path//\\//}"
    if [[ "$path" =~ ^([A-Za-z]):/(.*)$ ]]; then
        local drive="${BASH_REMATCH[1],,}"
        printf '/%s/%s\n' "$drive" "${BASH_REMATCH[2]}"
    else
        printf '%s\n' "$path"
    fi
}

to_cargo_path() {
    local path="$1"
    if [[ "$cargo_bin" == *.exe ]] && command -v wslpath >/dev/null 2>&1; then
        wslpath -w "$path"
    else
        printf '%s\n' "$path"
    fi
}

find_cargo() {
    if [[ -n "${CARGO_BIN:-}" ]]; then
        to_bash_path "$CARGO_BIN"
    elif command -v cargo >/dev/null 2>&1; then
        command -v cargo
    elif command -v where.exe >/dev/null 2>&1; then
        local windows_cargo
        windows_cargo="$(where.exe cargo.exe 2>/dev/null | tr -d '\r' | head -n 1)"
        [[ -n "$windows_cargo" ]] || return 1
        to_bash_path "$windows_cargo"
    else
        return 1
    fi
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
embedded_dir="$repo_root/crates/axiom-php/embedded"
artifact="$embedded_dir/runtime-stubs.bin"
backup="$embedded_dir/runtime-stubs.previous.bin"
mode="${1:-}"
[[ -n "$mode" ]] || { usage; exit 2; }
shift

cargo_bin="$(find_cargo)" || fail "Cargo was not found. Install Rust or set CARGO_BIN to the full path of cargo.exe."
cargo_toolchain_args=()
if [[ -n "${CARGO_TOOLCHAIN:-}" ]]; then
    cargo_toolchain_args=("+$CARGO_TOOLCHAIN")
fi
cargo_target_args=()
cargo_target_dir="${CARGO_TARGET_DIR:-}"
if [[ -z "$cargo_target_dir" && "$cargo_bin" == *.exe && -d "$repo_root/target-windows-1.88" ]]; then
    cargo_target_dir="$repo_root/target-windows-1.88"
fi
if [[ -n "$cargo_target_dir" ]]; then
    cargo_target_args=(--target-dir "$(to_cargo_path "$cargo_target_dir")")
fi

build_app=0
source_dir=""
map_file=""
case "$mode" in
    clear)
        for arg in "$@"; do
            [[ "$arg" == "--build-app" ]] && build_app=1 || fail "unknown argument: $arg"
        done
        ;;
    build)
        build_args=()
        for arg in "$@"; do
            if [[ "$arg" == "--build-app" ]]; then
                build_app=1
            else
                build_args+=("$arg")
            fi
        done
        if (( ${#build_args[@]} == 0 )); then
            source_input="$repo_root/stubs"
            map_input="$source_input/AxiomStubsMap.php"
        elif (( ${#build_args[@]} == 2 )); then
            source_input="${build_args[0]}"
            map_input="${build_args[1]}"
        else
            usage
            exit 2
        fi
        source_dir="$(cd "$source_input" 2>/dev/null && pwd)" || fail "stub folder not found: $source_input"
        map_file="$(cd "$(dirname "$map_input")" 2>/dev/null && printf '%s/%s' "$PWD" "$(basename "$map_input")")" \
            || fail "map directory not found: $(dirname "$map_input")"
        [[ -f "$map_file" ]] || fail "map file not found: $map_file"
        ;;
    *)
        usage
        exit 2
        ;;
esac

[[ -f "$artifact" ]] || fail "embedded artifact is missing: $artifact"
temp_dir="$(mktemp -d "$embedded_dir/.runtime-stubs-update.XXXXXX")"
cleanup() { rm -rf -- "$temp_dir"; }
trap cleanup EXIT
generated="$temp_dir/runtime-stubs.bin"

if [[ "$mode" == "clear" ]]; then
    empty_map="$temp_dir/AxiomStubsMap.php"
    cat > "$empty_map" <<'EOF'
<?php
const CLASSES = array (
);
const FUNCTIONS = array (
);
const CONSTANTS = array (
);
EOF
    mkdir "$temp_dir/source"
    source_dir="$temp_dir/source"
    map_file="$empty_map"
fi

(
    cd "$repo_root"
    "$cargo_bin" "${cargo_toolchain_args[@]}" run "${cargo_target_args[@]}" --locked -p axiom-php --bin axiom-php-stub-updater -- \
        generate --root "$(to_cargo_path "$source_dir")" \
        --map "$(to_cargo_path "$map_file")" --output "$(to_cargo_path "$generated")"
)
[[ -s "$generated" ]] || fail "stub generator did not produce an artifact"

cp -f -- "$artifact" "$backup"
mv -f -- "$generated" "$artifact"
printf 'Embedded PHP stubs updated: %s\n' "$artifact"
printf 'Previous artifact backup: %s\n' "$backup"

if (( build_app )); then
    cd "$repo_root"
    "$cargo_bin" "${cargo_toolchain_args[@]}" check "${cargo_target_args[@]}" --locked -p axiom-app
fi

printf 'Run Axiom normally; Cargo will rebuild the embedded artifact when needed.\n'
