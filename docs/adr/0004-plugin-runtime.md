# ADR 0004 — Runtime de Plugins

**Status:** Proposto (não implementado até Fase 9)
**Data:** 2026-08-19

## Contexto

O RustStorm terá sistema de plugins para estender suporte a outras linguagens,
frameworks, ferramentas. Precisamos de isolamento por segurança: plugins não
devem ter acesso irrestrito ao sistema de arquivos, rede, ou estado da IDE.

## Decisão (proposta)

Adotar **WebAssembly (WASM)** via **Wasmtime** como runtime de plugins.

```toml
wasmtime = "25"
```

## Motivos

1. **Isolamento:** WASM é sandboxed por design; plugins só acessam o que a
   IDE expõe via capabilities.
2. **Portabilidade:** Plugins compilados para WASM rodam em qualquer OS.
3. **Performance:** WASM é quase nativo; plugins podem fazer parsing, análise.
4. **Ecossistema:** Wasmtime é mantido pela Bytecode Alliance, maduro.

## Alternativas consideradas

| Runtime      | Motivo de descarte                              |
| ------------ | ----------------------------------------------- |
| **Lua**      | Simples, mas sem sandboxing nativo.             |
| **JS (Deno)**| Pesado, complexo, não nativo Rust.             |
| **Processos separados** | Overhead de IPC; difícil compartilhar estado. |
| **Extism**   | Wrapper sobre WASM, menos flexível que Wasmtime. |

## Trade-offs

- **Complexidade:** Wasmtime é grande; aumenta tamanho do binário.
- **Learning curve:** Desenvolvedores de plugins precisam conhecer WASM ou
  usar linguagens que compila para WASM (Rust, Go, C, AssemblyScript).

## Consequências

- **Positivas:** Segurança, portabilidade.
- **Negativas:** Ecossistema menor que plugins JS/Lua tradicionais.

## Revisão futura

Revisitar antes da Fase 9. Se WASM provar muito complexo, considerar Lua
com sandboxing manual ou Extism.
