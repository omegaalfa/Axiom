# Roadmap do Axiom

## Axiom AI — FUTURO

Direção futura: painel de IA e abstração de provedores para integração com o
Hermes Agent, com contexto do projeto, índice semântico, execução controlada,
revisão de diffs, permissões, traces e skills com aprovação humana.

Ciclo conceitual: Execute → Evaluate → Extract → Retrieve.
Nenhuma integração, dependência, chamada de modelo ou telemetry é implementada
nesta etapa.


Este roadmap define as fases de desenvolvimento do Axiom, sempre
garantindo que a aplicação permaneça compilável e executável ao final de cada
fase.

## Fase 0 — Bootstrap (atual)

**Objetivo:** Janela abre no Windows 10, logging, settings básicas,
estrutura de workspace.

**Entregas:**
- [x] Cargo workspace
- [x] Janela com GPUI
- [x] Logging estruturado (tracing)
- [x] Estrutura de diretórios (docs, crates, assets, etc.)
- [x] CI básico (GitHub Actions)
- [ ] Documentação inicial (ARCHITECTURE.md, ROADMAP.md, ADRs)

**Milestone:** Axiom 0.1.0 — Bootstrap ✓

## Fase 1 — Editor Básico

**Objetivo:** Editor de texto funcional, abas, syntax highlighting, find.

**Entregas:**
- [ ] Explorador de projeto (file tree)
- [ ] Editor com ropey (buffer)
- [ ] Abas (múltiplos arquivos abertos)
- [ ] Abrir/Salvar arquivos
- [ ] Syntax highlighting (inicial, via regex ou Tree-sitter)
- [ ] Números de linha
- [ ] Find (Ctrl+F)
- [ ] Undo/Redo
- [ ] Seleção de texto
- [ ] Copiar/Colar
- [ ] Atalhos básicos

**Milestone:** Axiom 0.1.0 — Editor PHP

## Fase 2 — Tree-sitter

**Objetivo:** Parsing incremental e syntax highlighting baseado em Tree-sitter.

**Entregas:**
- [ ] Gramáticas Tree-sitter (PHP, HTML, JS, TS, CSS, JSON, YAML, Markdown, SQL)
- [ ] Parser incremental
- [ ] Syntax highlighting baseado em árvore
- [ ] Outline (estrutura do arquivo)
- [ ] Code folding
- [ ] Bracket matching
- [ ] Seleção estrutural

## Fase 3 — Project Model

**Objetivo:** Compreender projetos PHP.

**Entregas:**
- [ ] Detecção de `composer.json`
- [ ] Parsing de autoload PSR-4/PSR-0
- [ ] File watcher (notify)
- [ ] VFS (Virtual File System)
- [ ] Normalização de caminhos Windows

## Fase 4 — LSP

**Objetivo:** Integrar Intelephense e Phpactor via LSP.

**Entregas:**
- [ ] Cliente LSP genérico
- [ ] Autocomplete
- [ ] Hover
- [ ] Go to Definition
- [ ] Find References
- [ ] Rename
- [ ] Diagnostics
- [ ] Quick fixes básicos

## Fase 5 — Index Próprio

**Objetivo:** Índice de símbolos persistente.

**Entregas:**
- [ ] SQLite para persistência
- [ ] Indexação em background
- [ ] Go to Class / Go to Symbol
- [ ] Busca rápida de símbolos
- [ ] Incremental indexing

## Fase 6 — PHP Semantic Engine

**Objetivo:** Motor próprio de inteligência PHP.

**Entregas:**
- [ ] Sistema de tipos PHP completo
- [ ] Parser de PHPDoc
- [ ] Resolução de símbolos
- [ ] Inferência de tipos básica
- [ ] Herança, traits, interfaces

## Fase 7 — IDE Completa

**Objetivo:** Funcionalidades completas de IDE.

**Entregas:**
- [ ] Terminal integrado (portable-pty)
- [ ] Git (git2)
- [ ] Composer UI
- [ ] PHPUnit runner
- [ ] Xdebug via DAP
- [ ] Command Palette
- [ ] Breadcrumbs

## Fase 8 — Refactoring

**Objetivo:** Refatorações semânticas.

**Entregas:**
- [ ] Safe Rename
- [ ] Extract Method / Variable
- [ ] Inline Variable
- [ ] Move Class
- [ ] Change Namespace
- [ ] Safe Delete
- [ ] Generate Constructor / Getter / Setter

## Fase 9 — Plugins

**Objetivo:** Sistema de extensões.

**Entregas:**
- [ ] Wasmtime runtime
- [ ] Extension API
- [ ] Extension manifest
- [ ] Capabilities restritas
- [ ] Plugins oficiais (Docker, SQL, frameworks)

## Fase 10 — Distribuição

**Objetivo:** Instalação profissional.

**Entregas:**
- [ ] Instalador Windows (WiX / NSIS)
- [ ] Auto-updater assinado
- [ ] Release pipeline
- [ ] Code signing
- [ ] Associação de arquivos (.php, .phtml)
- [ ] Menu de contexto

## Milestones

| Versão  | Nome                      | Status     |
| ------- | ------------------------- | ---------- |
| 0.1.0   | Bootstrap                 | 🚧 em curso |
| 0.1.1   | Editor PHP                | 📋 planned |
| 0.2.0   | PHP Intelligence (LSP)    | 📋 planned |
| 0.3.0   | Native PHP Intelligence   | 📋 planned |
| 1.0.0   | Axiom IDE             | 📋 future  |

## Princípios

1. **Incremental:** cada entrega deve compilar e executar.
2. **Modular:** componentes desacoplados, substituindo LSP por native quando
   for vantajoso.
3. **Performance:** nunca bloquear a UI thread.
4. **Windows-first:** priorizar compatibilidade com Windows 10/11.
5. **Original:** não copiar visual, nomes internos ou propriedades
   intelectuais de outras IDEs.
## Axiom AI — FUTURO

Futuramente, a arquitetura poderá incorporar um painel de IA e uma abstração
de provedores para integração com o Hermes Agent. Essa direção poderá expor
contexto do projeto, índice semântico, execução controlada de terminal/testes,
revisão/aplicação de diffs, permissões, traces de sessão e skills com aprovação
humana.

O ciclo conceitual previsto é: Execute → Evaluate → Extract → Retrieve.
Nenhuma integração, dependência, chamada de modelo ou telemetry é implementada
nesta etapa.
