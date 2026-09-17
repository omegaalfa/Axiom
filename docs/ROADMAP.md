# Roadmap do Axiom

Estado real: **pre-alpha experimental**. O workspace compila no Windows com Rust 1.88 e já possui editor PHP, índice semântico residente e navegação funcional. Ainda não há contrato de estabilidade nem cobertura de IDE completa.

## Concluído

- ✅ Workspace Cargo, janela GPUI, abas, árvore de projeto, abrir/salvar e terminal.
- ✅ Editor com buffer, seleção, clipboard, undo/redo, syntax highlighting PHP, números de linha e Find local.
- ✅ Índice incremental/persistente com Project, Vendor e Runtime, identidade de caminho Windows e prefix lookups.
- ✅ LSP bridge com sessões/revisões e descarte de respostas obsoletas.
- ✅ Diagnostics nativos de aridade e tipos para constructors, métodos e funções globais; ranges exatos e tipos union/nullable/mixed/variadic.
- ✅ Formatting PHP conservador, guard de linguagem, preservação de literais multilinha e validação de revisão.
- ✅ Find/Replace Current/All, References/Find Usages e Go to Implementation básicos com snapshots e dirty buffers.
- ✅ Modal Create PHP Type com Class/Interface/Trait/Enum, inputs independentes, PSR-4 residente e auto-link Name → File.

## Parcial

- 🟡 **Axiom AI / Chat:** provider Ollama local, seleção e cache de modelos,
  Thinking por modelo persistido, respostas Markdown, Thinking retornado pelo
  provider, streaming NDJSON com fila/stale guard, cópia de respostas/blocos de
  código e scroll nativo do chat já estão implementados. Permanecem cancelamento,
  retry, histórico persistente, metadados de uso e validação manual contínua do
  streaming.

- 🟡 Completion: lookup e ranking existem, mas faltam inferência de intenção, expected type e edição semântica.
- 🟡 References/Find Usages: classes, métodos, propriedades, funções e constantes indexadas; variáveis locais/parâmetros e casos ambíguos permanecem limitados.
- 🟡 Go to Implementation: interfaces e métodos indexados funcionam; subclasses genéricas, overrides abstratos e consumidores de traits ainda não são completos.
- 🟡 Project model: Composer PSR-4/PSR-0, watcher e normalização existem; VFS e resolução abrangente de módulos ainda faltam.
- 🟡 Create PHP Type: namespace PSR-4 básico funciona; autocomplete de bases/interfaces, imports/FQN, validação avançada e File Structure estão pendentes.
- 🟡 Formatting: provider nativo seguro existe; providers externos e format-on-save continuam fora do escopo.

## Pendente / futuro

File Structure, identidade de variáveis e parâmetros, safe rename local, hierarchy completa, quick documentation, imports seguros, refactorings, Composer/PHPUnit/Xdebug, Git, debugger, plugins e distribuição.

## Próximas fases recomendadas

1. Consolidar e testar a independência/UX do Create PHP Type.
2. Tornar PSR-4 e imports/FQN seguros, sem filesystem no typing.
3. Implementar File Structure e identidade de variáveis/parâmetros.
4. Safe rename local e navegação adicional.
5. Completion de Extends/Implements baseada em índices residentes.
6. Composer/PHP ecosystem, Git e debugger.
7. Plugins e distribuição.

## Guardrails de performance do editor

- Não adicionar trabalho pesado à UI thread, `document.content()` por tecla, parse completo por tecla/render, filesystem/canonicalize/metadata no hot path ou scans O(project/vendor/scopes) durante typing/completion.
- Não reconstruir índices durante completion; evitar locks bloqueantes e espera síncrona na UI.
- Preferir snapshots/índices residentes, workers para trabalho pesado e checks de generation/revision/session.
- Toda feature que toque editor, completion, diagnostics, semantic, indexação ou LSP deve demonstrar ausência de regressão no typing.

## Princípios

Cada fase deve compilar e executar no Windows, ser modular, incremental e validada por testes focados antes de ampliar o escopo.

## Evidências principais

As capacidades acima são implementadas em `crates/axiom-app/src/workspace_view.rs`, `editor_view.rs`, `ui/input_line.rs`, `crates/axiom-index/src/semantic.rs` e `crates/axiom-project/src/lib.rs`, com testes unitários nos próprios módulos. O estado permanece experimental.
## Atualização de fase

Safe Rename local foi removido. LocalBindingId, Find Usages local e completion de variáveis permanecem preservados. Próxima etapa recomendada: File Structure baseada em snapshots e índices residentes; reavaliar Safe Rename somente após estabilizar identidade e escopo.
## AtualizaÃ§Ã£o 2026-09-13

File Structure / Outline estÃ¡ concluÃ­do com snapshot semÃ¢ntico, navegaÃ§Ã£o por teclado/mouse, caret colapsado, reveal centralizado e destaque transitÃ³rio. PrÃ³xima etapa: ampliar References/Find Usages e Go to Implementation em casos ambÃ­guos.
