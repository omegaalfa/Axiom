#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"

cargo fmt --all --check
cargo check -p axiom-editor
cargo test -p axiom-editor
cargo clippy -p axiom-editor --all-targets -- -D warnings
cargo check --workspace
cargo test --workspace
