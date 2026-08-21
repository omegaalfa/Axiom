# ADR 0001 — Framework de Interface Gráfica

**Status:** Aceito
**Data:** 2026-08-19

## Contexto

Precisamos escolher um framework de UI em Rust para construir o RustStorm,
uma IDE desktop moderna para PHP. A escolha deve atender:

- Windows 10 64-bit como plataforma primária
- Performance alta (GPU-accelerated)
- Renderização de texto avançada (code editor)
- Suporte a múltiplas janelas, HiDPI, IME, clipboard, menus
- Acessibilidade
- Maturidade e manutenção
- Licença permissiva

## Decisão

Adotamos **GPUI** (versão 0.2), o framework de UI criado pela equipe do Zed.

```toml
gpui = "0.2"
```

## Motivos

1. **Propósito adequado:** GPUI foi projetado especificamente para editores
   de código (Zed é um code editor em produção). Possui text layout avançado
   (cosmic-text/font-kit), editor primitives (`InputHandler`, `EntityInputHandler`),
   e `UniformList` para listas virtualizadas.

2. **Performance:** Renderização acelerada por GPU via Blade/WGPU, com
   arquitetura imediata-retida híbrida que se adapta bem a UIs complexas.

3. **Windows suportado:** Zed roda oficialmente no Windows desde 2025,
   provando que GPUI funciona em produção no Windows 10/11.

4. **Ecosistema maduro:** 208k+ downloads no crates.io, mantido por
   desenvolvedores experientes (criadores do Atom e Tree-sitter).

5. **Licença:** Apache-2.0, permissiva e compatível com IDEs comerciais.

## Alternativas consideradas

| Framework | Motivo de descarte                                  |
| --------- | --------------------------------------------------- |
| **Floem** | Menor adoção (33k downloads), pre-1.0, documentação menor. Boa alternativa se GPUI falhar. |
| **Iced**  | Excelente, mas menos orientado a editores; requer mais trabalho para editor customizado. |
| **Slint** | Licença GPLv3/comercial; restrições de uso.         |
| **egui**  | Immediate mode; text editing limitado para editor principal; performance em UIs grandes. |
| **Tauri** | Base web; não atende objetivo "majoritariamente Rust". |
| **winit + wgpu custom** | Muito trabalho; GPUI já resolve a maior parte. |

## Trade-offs

- **Pre-1.0:** API muda com frequência. Mitigação: isolar código de UI em
  `ruststorm-ui` e criar wrappers quando necessário.
- **Documentação:** Ainda escassa, principalmente fora do código-fonte do
  Zed. Mitigação: estudar o código do Zed como referência.
- **Dependências pesadas:** GPUI traz várias deps (Blade, font-kit, etc.).
  Aceitável dado que IDE já é pesado por natureza.
- **Curva de aprendizado:** Alta. Mitigação: equipe focada e exemplos do Zed.

## Consequências

- **Positivas:**
  - UI profissional e rápida desde o primeiro dia
  - Base sólida para editor complexo
  - Menos código custom a escrever

- **Negativas:**
  - Dependemos de mudanças do GPUI upstream
  - Se GPUI for descontinuado, reescrever UI terá custo alto

## Revisão futura

Revisitar após a Fase 2 (Tree-sitter) para avaliar se GPUI atende às
necessidades do editor custom. Se necessário, migrar para Floem ou solução
própria em `ruststorm-ui`.
