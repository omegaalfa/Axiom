# ADR 0006 — Editor core headless

**Status:** Aprovado com baseline estabilizado
**Data de estabilização:** 2026-08-19

## Decisão

Adotar:

```text
ruststorm-editor::Document
└── floem-editor-core 0.2.0
    └── lapce-xi-rope 0.3.2
```

O core é headless: não depende de Floem UI, GPUI, wgpu ou winit.

## Contrato validado

- offsets internos em bytes UTF-8;
- edição limitada a fronteiras de Unicode scalar;
- offsets inválidos são clampados e normalizados para trás;
- seleção é armazenada apenas em `CursorMode`;
- inserções consecutivas do mesmo tipo são coalescidas;
- cursor e seleção são restaurados no undo/redo;
- pristine acompanha a revisão salva;
- EOL predominante LF/CRLF é preservado em edição e save;
- I/O real e documentos de 10 mil/100 mil linhas têm smoke tests.

## Licenças

- floem-editor-core 0.2.0: MIT;
- lapce-xi-rope 0.3.2: Apache-2.0.

## Limites

Navegação por grapheme, mapeamento UTF-16 para LSP e integração GPUI não fazem
parte deste baseline. A GUI Windows permanece não validada.
