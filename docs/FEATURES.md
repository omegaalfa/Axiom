# Axiom Features

This page describes features exposed by the current application build. Axiom
is pre-alpha, so APIs and behavior may change.

## Implemented

### Editor

Use the Project Explorer to open PHP files. Editing supports selection,
clipboard, undo/redo, auto-pairs, auto-indent, diagnostics, and scrolling.
The current shortcut is shown in the Edit menu and in Settings → Keymap.

### Completion

Press `Ctrl+Space` at a PHP expression. Native project/runtime indexes are used
when available; a configured LSP can provide additional results. Member
completion also opens automatically after `->`, `::`, and `new` when the
relevant index is ready.

### Go to Definition

Place the caret on a class or member and press `Ctrl+B`, or use Ctrl+Click.
Native project/runtime indexes are used before an LSP fallback when possible.

### Go to Class and Symbol

Press `Ctrl+N` for classes or `Ctrl+Shift+Alt+O` for project symbols, then type
to filter and confirm with Enter.

### Reformat Code

Invoke `Ctrl+Alt+L`. If no PHP formatter provider is configured, Axiom reports
that state instead of silently doing nothing.

### Terminal

Use the Activity Bar, View → Terminal, or `Ctrl+`` to show the integrated PTY
terminal in the project working directory. Recognized file and line links can
be opened with Ctrl+Click.

### Command Palette and Settings

Press `Ctrl+Shift+P` to search commands. File → Settings → Keymap lets you
search actions, inspect descriptions, edit or remove shortcuts, and reset them.
Menus and this help view use the current keymap.

### Windows diagnostics

For shortcut troubleshooting, start a debug build with `AXIOM_DEBUG_KEYS=1`.
Axiom logs key names, matched command IDs, context, and dispatch results;
typed text is never logged. The status bar reports LSP and PHP runtime-stub
availability so completion claims can be checked against the running project.

## Partial

LSP completion, hover, diagnostics, references, formatting, and signature help
depend on a configured compatible PHP server such as Intelephense. Rich PHP
type inference and framework-specific intelligence are still being expanded.

## Planned

Debugger, complete Git UI, plugin marketplace, deep Laravel/Symfony support,
database tools, the full PHP type engine, and Axiom AI are roadmap items.

