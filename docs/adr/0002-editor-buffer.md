# ADR 0002 — Buffer do editor

**Status:** Substituído pelo ADR 0006

## Contexto

A decisão inicial propunha `ropey`, antes da prova de conceito do editor.
O código real adotou o backend headless do Floem/Lapce.

## Decisão vigente

```text
Document
└── floem-editor-core 0.2.0
    └── lapce-xi-rope 0.3.2
```

O backend permanece encapsulado: `Buffer` não é exposto pela API pública.

## Contrato de posições

Offsets internos são bytes UTF-8. Cursor e seleção públicos são normalizados
para fronteiras de Unicode scalar antes de editar. Grapheme clusters e
conversão UTF-16 para LSP ficam fora deste ADR.

## Consequências

- rope eficiente para arquivos grandes;
- undo/redo transacional reutilizado do backend;
- dependência externa isolada por `Document`;
- necessidade futura de mapear bytes para graphemes e posições LSP.
