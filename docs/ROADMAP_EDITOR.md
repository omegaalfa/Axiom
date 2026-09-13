# Axiom IDE — Roadmap real de desenvolvimento

Este documento é um guia de execução para uma IDE PHP experimental. A prioridade é preservar a fluidez do editor e ampliar capacidades comprovadas pelo índice semântico existente.

## Concluído

- ✅ Edição diária: seleção, clipboard, undo/redo, abas, abrir/salvar, árvore, syntax highlighting e Find local.
- ✅ Find/Replace: próximo/anterior, wrap, Replace Current e Replace All em operação única.
- ✅ Diagnostics: aridade e compatibilidade de tipos em constructors, métodos e funções globais, com ranges dos argumentos.
- ✅ Navegação: References/Find Usages e Go to Implementation básicos, com dirty-buffer e stale-result checks.
- ✅ Formatting: Ctrl+Alt+L restrito a PHP, fallback nativo conservador e proteção de revisão/session.
- ✅ Create PHP Type: selector de tipo, Name, Namespace, File, Extends, Implements e Directory informativo; inputs independentes e PSR-4 básico.

## Parcial

- 🟡 Completion ainda é principalmente lookup/ranking.
- 🟡 References e Go to Implementation não cobrem todos os bindings locais, ambiguidades, traits, genéricos e hierarquias complexas.
- 🟡 Project model resolve Composer PSR-4 e metadata residente, mas não é um VFS completo.
- 🟡 Formatting não inclui PHP CS Fixer, PHPCBF, format-on-save, range formatting ou optimize imports.
- 🟡 UX dos popups e do modal continua sujeita a refinamentos visuais e validação manual.

## Próximo

1. Finalizar testes e UX do Create PHP Type sem introduzir estado compartilhado.
2. Robustecer PSR-4, imports/FQN e autocomplete seguro de Extends/Implements.
3. Implementar File Structure usando o snapshot atual.
4. Modelar identidade de variáveis/parâmetros e safe rename local.
5. Completar referências e implementação com casos ambíguos explicitamente conservadores.
6. Melhorar Quick Documentation e navegação adicional.

## Futuro

Search Everywhere, hierarchy completa, refactorings de classe/método, geração de código, completion por expected type, optimize imports, providers de formatter externos, Composer UI, PHPUnit, Git, Xdebug/DAP, plugins e distribuição.

## Guardrails de performance do editor

- Não adicionar trabalho pesado à UI thread, `document.content()` por tecla, parse completo por tecla/render, filesystem/canonicalize/metadata no hot path ou scans O(project/vendor/scopes) durante typing/completion.
- Não reconstruir índices durante completion; evitar locks bloqueantes e espera síncrona na UI.
- Reutilizar snapshots/índices residentes, executar trabalho pesado em workers e validar generation/revision/session antes de publicar resultados.
- Qualquer mudança em editor, completion, diagnostics, semantic, indexação ou LSP deve incluir uma auditoria explícita de impacto no typing.

## Critérios permanentes de aceite

Cada fase precisa de reprodução focada, teste determinístico quando possível, `cargo check`, `cargo fmt -- --check`, `git diff --check` e validação manual quando a mudança for visual. Respostas stale devem ser descartadas; conteúdo e seleção nunca podem ser perdidos por troca de foco.

## Evidências

O estado descrito é baseado nos fluxos em `workspace_view.rs`, `editor_view.rs`, `ui/input_line.rs`, `axiom-index/src/semantic.rs` e `axiom-project/src/lib.rs`, além dos testes de diagnostics, formatting, Find/Replace, referências, implementação e modal presentes no binário `axiom`.
## Atualização de fase

Safe Rename local foi removido. LocalBindingId e Find Usages local permanecem preservados. Próxima etapa recomendada: File Structure baseada no snapshot atual; reavaliar Safe Rename após estabilizar identidade e escopo.
## AtualizaÃ§Ã£o 2026-09-13

File Structure / Outline estÃ¡ concluÃ­do com snapshot semÃ¢ntico, caret colapsado, reveal centralizado e destaque transitÃ³rio. PrÃ³xima etapa: ampliar References/Find Usages e Go to Implementation em casos ambÃ­guos.
