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
- **Implemented:** Axiom AI chat and Agent runtime with HTTP model providers,
  streaming responses, registered project tools, approval-gated mutations, and
  optional persistent memory.

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

## Axiom AI Agent

The Axiom AI panel and Agent runtime are implemented. The runtime supports
streaming HTTP model providers and twelve tool capability groups: `read_file`,
`list_directory`, `find_files`, `search_text`, `find_symbol`,
`find_references`, `fetch_url`, `write_file`, `update_file`, `delete_file`,
`execute_command`, and persistent memory. The memory group provides
`memory_search` and `memory_get`, with `memory_remember` exposed only when
memory writing is available. Memory operations are otherwise conditional on
backend availability.

`StandardToolPolicy` allows safe reads and requires approval for file mutations,
terminal commands, and persistent-memory writes. `ApprovalBridge` pauses a run
for a user decision and resumes or denies the requested operation. Local
persistent memory uses SQLite with an FTS5 index in WAL mode.

Further provider integrations and Agent capabilities remain under development;
that ongoing work does not mean the existing Agent runtime and tools are only
planned.

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
inspections, refactorings, Windows-native validation, continued AI Agent
improvements, and a future plugin architecture. See the [project roadmap](docs/ROADMAP.md).

# Compilar em debug
```
cargo build -p axiom-app --locked
```
# Compilar em release
```
cargo build --release -p axiom-app --locked
```

# Compilar release com saída mais limpa
```
cargo build --release -p axiom-app --locked --quiet
```

# Só verificar se compila, sem gerar binário final
```
cargo check -p axiom-app --locked
```

# Rodar direto em debug
```
cargo run -p axiom-app --locked -- .
```

# Rodar direto em release

```
cargo run --release -p axiom-app --locked -- .
```

 Sobre --quiet — acredito que era isso que você quis dizer, não --quit.

Ele só reduz a quantidade de mensagens no terminal:
```
cargo build --release -p axiom-app --locked --quiet
```
Depois, para abrir o executável já compilado:
```
.\target\release\axiom.exe .
```
Se quiser o fluxo que eu mais recomendo para testar performance do Axiom:
``` 
cd E:\dev\Axiom
cargo build --release -p axiom-app --locked --quiet
.\target\release\axiom.exe .
```

E para desenvolvimento rápido, sem esperar build release:
```
cargo run -p axiom-app --locked -- .
```

## Repository and contribution

The official repository is [github.com/omegaalfa/Axiom](https://github.com/omegaalfa/Axiom).
The project is in an early development phase; focused issues and pull requests
are welcome, with APIs and UX expected to evolve.

Project license decision is currently **pending**. Third-party licenses are
documented in [docs/THIRD_PARTY.md](docs/THIRD_PARTY.md).

