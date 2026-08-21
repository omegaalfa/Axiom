# ADR 0003 — Persistência do Índice de Símbolos

**Status:** Aceito
**Data:** 2026-08-19

## Contexto

O RustStorm precisa persistir um índice de símbolos (classes, métodos,
propriedades, funções, namespaces, etc.) entre sessões, permitindo startup
rápido sem reindexar projetos grandes a cada abertura.

## Decisão

Adotamos **SQLite** via crate **rusqlite** (com feature `bundled`).

```toml
rusqlite = { version = "0.32", features = ["bundled"] }
```

## Motivos

1. **Maturidade:** SQLite é um dos bancos de dados mais usados do mundo, testado
   em bilhões de dispositivos.

2. **Transacional:** ACID completo, garante consistência em caso de crash.

3. **Portabilidade:** Arquivo único, fácil de copiar, backup, migrar.

4. **Bundled:** `rusqlite` compila o SQLite estaticamente, sem dependências
   externas no Windows.

5. **Performance:** Suporta índices, queries complexas, transactions em lote
   para indexação.

## Alternativas consideradas

| Banco        | Motivo de descarte                                     |
| ------------ | ------------------------------------------------------ |
| **redb**     | Puro Rust, rápido, mas novo (1.5.0 em 2024), menos testado em produção. |
| **sled**     | Descontinuado (autor recomendou não usar).             |
| **heed**     | Wrapper do LMDB, rápido, mas LMDB tem limitações de concorrência. |
| **RocksDB**  | Overkill para índice de símbolos; complexo de manter.  |

## Trade-offs

- **Não é puro Rust:** rusqlite binda a SQLite C. Aceitável dado que SQLite
  é extremamente estável.
- **Concorrência de escrita:** SQLite bloqueia writes concorrentes. Mitigação:
  ter um único writer (indexer thread) e leitores concorrentes (queries).
- **Tamanho do arquivo:** Para projetos muito grandes, pode chegar a dezenas
  de MBs. Aceitável.

## Consequências

- **Positivas:**
  - Startup rápido (ler índice existente)
  - Robustez (transactions protegem contra corrupção)
  - Ferramentas externas (DB Browser for SQLite) para debug

- **Negativas:**
  - Binário final inclui SQLite C (~1MB)
  - Requer schema migration management

## Revisão futura

Se redb amadurecer e oferecer performance significativamente melhor em
benchmarks, considerar migração. Manter schema abstrato em `ruststorm-index`
para facilitar troca.
