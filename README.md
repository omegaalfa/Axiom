# Axiom

Modern PHP IDE written in Rust.

Axiom is a desktop IDE in active pre-alpha development. It is designed for a
fast, native-feeling PHP workflow with a strong local foundation: project
navigation, Composer awareness, PHP intelligence, an integrated terminal, and
future extensibility for AI-assisted development. APIs, UX, and architecture
may change while the project evolves.

## Project status

**Pre-Alpha / Experimental** — useful for development and experimentation, but
not production-ready and not yet finally validated on Windows GUI.

## Implemented capabilities

- **Implemented:** GPUI desktop shell, project lifecycle, Welcome Screen,
  Project Explorer, multi-file tabs, and dirty-document handling.
- **Implemented:** PHP Tree-sitter parsing, syntax highlighting, diagnostics,
  auto-pairs, auto-indent, mouse/keyboard editing, clipboard, undo/redo, and
  virtualized scrolling.
- **Implemented:** configurable keymap, Settings, Command Palette, and command
  descriptions.
- **Implemented:** native Project Symbol Index, PHP Runtime Stubs, Composer
  PSR-4/classmap metadata, Go to Class, Go to Symbol, navigation history,
  Ctrl+Click/Go to Definition, and native auto-import support.
- **Partial:** completion, hover, diagnostics, definitions, references,
  formatting, and signature help through configurable PHP LSP providers such as
  Intelephense. LSP complements, rather than replaces, local parsing and
  indexing.
- **Implemented:** integrated terminal over a PTY, project working directory,
  terminal file/line links, Ctrl+Click link navigation, and terminal context
  integration.

Full PHP type inference, debugger, complete Git UI, deep framework intelligence,
database tools, plugin marketplace, and final installer remain roadmap items.

## Architecture

The workspace is split into focused crates:

- `axiom-app` — GPUI application shell and user interface.
- `axiom-editor` — document buffer and editor interaction primitives.
- `axiom-syntax` — Tree-sitter parsing, tokens, highlighting, and diagnostics.
- `axiom-project` — project lifecycle, filesystem model, Composer metadata, and PSR-4 resolution.
- `axiom-index` — project and runtime symbol indexing.
- `axiom-php` — PHP runtime stubs and native PHP symbol model.
- `axiom-lsp` — JSON-RPC/LSP client and provider integration.
- `axiom-terminal` — PTY-backed terminal sessions and link extraction.

See the detailed [architecture document](docs/ARCHITECTURE.md).

## PHP intelligence

Axiom combines Tree-sitter syntax data, the Project Symbol Index, Composer
metadata, PHP Runtime Stubs, and optional LSP providers. This layered design
keeps core navigation useful without requiring an LSP, while allowing
Intelephense or another compatible server to add richer assistance.

## Axiom AI — Planned

AI-assisted development is a future direction, not an implemented feature.
Planned work includes an agent panel, Hermes Agent integration, semantic IDE
tools, diff review/apply, explicit permissions, Skills, session traces, and a
closed learning loop: `Execute → Evaluate → Extract → Retrieve`.

## Requirements and build

The Rust toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml).
The currently validated workflow is Linux/WSL with X11; the final target is
Windows 10, but Windows GUI validation is still pending. Linux/WSL GPUI builds
may require `libxkbcommon-dev` and `libxkbcommon-x11-dev`.

```bash
cargo check --workspace
cargo test --workspace
cargo build -p axiom-app
```

To run in the validated WSL/X11 workflow:

```bash
env -u WAYLAND_DISPLAY cargo run -p axiom-app -- .
```

## Configuration

- `AXIOM_PROJECT` — optional project path override.
- `AXIOM_PHP_LSP` — optional path to a PHP LSP executable.
- `AXIOM_PHP_STUBS` — optional path to an external PHP runtime-stubs checkout.

Older RustStorm-prefixed variables may be recognized only as migration
fallbacks; they are not the primary configuration interface.

## Roadmap

Near-term direction includes a PHP type engine, PHPDoc/type inference, deeper
inspections, refactorings, Windows-native validation, an AI agent foundation,
and a future plugin architecture. See the [project roadmap](docs/ROADMAP.md).

## Repository and contribution

The official repository is [github.com/omegaalfa/Axiom](https://github.com/omegaalfa/Axiom).
The project is in an early development phase; focused issues and pull requests
are welcome, with APIs and UX expected to evolve.

Project license decision is currently **pending**. Third-party licenses are
documented in [docs/THIRD_PARTY.md](docs/THIRD_PARTY.md).

