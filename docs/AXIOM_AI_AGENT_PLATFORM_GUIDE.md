# Axiom AI & Agent Platform — Architecture, Guardrails, Roadmap & Agent Handoff

> **Status:** Documento unificado de orientação técnica  
> **Baseline arquitetural:** ADR 0007 / M.0.1 — auditado em 2026-09-14  
> **Uso pretendido:** entregar ao agente de desenvolvimento quando o frontend visual de IA estiver concluído  
> **Projeto:** Axiom IDE — Rust / GPUI / PHP

---

## 0. Como usar este documento

Este arquivo consolida em um único lugar:

1. a auditoria arquitetural real do Axiom feita antes da implementação de IA;
2. as decisões de boundary da ADR 0007;
3. os guardrails obrigatórios de estabilidade e performance;
4. o ponto de partida após a conclusão do frontend visual de IA;
5. o roadmap completo da **Phase M — Axiom AI & Agent Platform**;
6. as regras para providers, chat, contexto, tools, Agent Runtime, permissões, diff/review, traces, memória, skills, Hermes, MCP, sub-agents e evolução futura.

### Regra de precedência

Quando houver diferença entre uma intenção de roadmap e o estado real do source:

```text
source atual auditado
    >
ADR / decisões arquiteturais aceitas
    >
roadmap futuro
    >
suposições do agente
```

O agente deve sempre auditar o source antes de materializar uma abstração.

Este documento não autoriza a criação antecipada de crates, traits, serviços ou dependências apenas porque aparecem no roadmap.

---

# 1. Princípio central

> **A IA é consumidora da inteligência do Axiom, nunca sua fonte de verdade.**

Index, Type Engine, PHPDoc, diagnostics, Composer, filesystem, Git, testes e demais sistemas determinísticos continuam autoritativos.

A IA recebe acesso a essas capacidades por APIs, snapshots e Tools controladas.

Consequência:

```text
Current deterministic project state
        >
Accepted explicit instructions
        >
Historical memory
        >
Model inference
```

O Axiom deve continuar plenamente utilizável com IA desativada, sem credenciais ou com provider indisponível.

---

# 2. ADR 0007 — Baseline arquitetural auditada

## 2.1 Status

**Decisão de boundary para M.0.1.**

Contratos e integrações de IA ainda não eram implementados quando a auditoria foi feita.

A ADR foi criada para registrar o estado real do projeto antes de materializar a arquitetura de IA.

---

## 2.2 Workspace auditado

O `Cargo.toml` do workspace declarava oito crates:

| Crate | Responsabilidade e dependências locais |
| --- | --- |
| `axiom-app` | Composição GPUI; depende dos outros sete crates |
| `axiom-editor` | Document, buffer, seleção e histórico; `floem-editor-core` headless |
| `axiom-project` | Project, Composer e PSR-4; sem dependências locais |
| `axiom-syntax` | Parsing incremental Tree-sitter; sem UI |
| `axiom-php` | Símbolos de runtime/stubs; depende de `axiom-syntax` |
| `axiom-index` | Índices, identidade, referências e `SemanticSnapshot`; depende de `axiom-php` e `axiom-syntax` |
| `axiom-lsp` | Cliente genérico stdio, protocolo e codecs; sem dependências locais |
| `axiom-terminal` | PTY e emulação; sem dependências locais |

Não havia crate genérico de shared types.

Tipos pertenciam aos respectivos domínios, por exemplo:

```text
Document                         -> axiom-editor
SemanticSnapshot / IDs           -> axiom-index
protocol / LSP types             -> axiom-lsp
```

Não criar um `axiom-common` ou equivalente apenas por conveniência sem uma necessidade arquitetural comprovada.

---

## 2.3 Owners e snapshots existentes

`WorkspaceView` em `axiom-app` mantém composição de Project, abas/EditorView e engine semântica compartilhada.

`EditorView` possui, entre outros estados:

```text
Document
syntax
DiagnosticStore
document/session state
edit generations
```

`SemanticEngine` publica `Arc<SemanticSnapshot>` via `RwLock`.

A publicação protege monotonicidade de revisão e snapshots podem ser obtidos por caminho não bloqueante quando disponível.

### Regra para IA

AI não deve receber owners mutáveis como:

```text
&mut WorkspaceView
&mut EditorView
&mut Document
```

Contexto deve entrar como valores/snapshots explicitamente capturados pela aplicação.

---

# 3. Async, cancelamento e eventos existentes

## 3.1 Estado auditado

O projeto usa padrões diferentes conforme a área:

```text
cx.spawn
GPUI tasks
background executors
std::thread::spawn
std::sync::mpsc
processos stdio no LSP
```

Não havia runtime Tokio explicitamente criado pelos crates locais.

Dependências transitivas não constituem contrato de runtime da aplicação.

### Não assumir

O agente não deve concluir:

```text
Tokio existe transitivamente
→ então AI pode depender globalmente de Tokio
```

Uma decisão dessa natureza exige fase própria e justificativa arquitetural.

---

## 3.2 Cancelamento existente

Existiam padrões reutilizáveis, mas não uma abstração universal:

```text
document_session
edit_generation
native_inspection_generation
AtomicU64 latest-generation
project generation
semantic generation
LSP request identities
```

Inspections usam cancelamento cooperativo por geração entre regras.

Isso não implica interrupção automática de uma operação de I/O já em execução.

### Direção para IA

Reutilizar conceitualmente:

```text
request identity
session/base revision
latest-wins
cooperative cancellation
stale-result rejection
```

Não reutilizar um contador de edição como identidade de toda tarefa de Agent.

Cada domínio deve possuir identidade coerente com seu lifecycle.

---

## 3.3 Streaming e eventos

Existem canais tipados e enums específicos, mas a auditoria não encontrou um event bus genérico adequado para streaming de modelos.

Não transformar canais LSP ou `mpsc` existentes em contrato de AI apenas porque já existem.

Streaming futuro deve considerar:

```text
bounded channels
backpressure
disconnectable consumers
coalescing/batching
cancellation
stale-result guards
```

Evitar:

```text
polling por token
render por token sem batching
filas ilimitadas
```

Eventos conceituais futuros podem incluir:

```text
ModelStarted
ModelDelta
ModelFinished
ModelFailed

AgentStarted
ToolRequested
ToolStarted
ToolFinished
AgentFinished
AgentCancelled
AgentFailed
```

Os nomes reais devem nascer do contrato mínimo necessário, não antecipadamente.

---

# 4. Settings e secrets

## 4.1 Estado auditado

O projeto já persiste configurações por domínio, por exemplo:

```text
recent-projects.json
keymap.json
ui-settings.json
```

Não havia serviço central de settings consolidado.

Também existiam diferenças históricas de `ProjectDirs`, que não devem ser “corrigidas” incidentalmente durante a implementação de IA.

---

## 4.2 SecretStore

Na auditoria não foi encontrada implementação local de:

```text
SecretStore
keyring
Windows Credential Manager adapter
```

Credenciais reais **não podem** ser armazenadas em plaintext em:

```text
project config
ui-settings.json
trace
prompt
roadmap
logs
memory
```

Direção futura:

```text
common config
    -> opaque secret reference

OS SecretStore adapter
    -> real secret
```

Se o cofre estiver indisponível:

```text
provider unavailable
```

Nunca:

```text
fallback plaintext
```

A seleção de biblioteca, disponibilidade, redaction, portabilidade e testes do SecretStore devem ser decididos antes do primeiro provider real.

---

# 5. Boundary arquitetural aprovada

Boundary conceitual:

```text
axiom-app
    ↓
axiom-ai-provider
    ↓
axiom-ai-core
```

Com a interpretação correta:

### `axiom-app`

Responsável por:

```text
GPUI
UI composition
captura explícita de contexto
adapters entre IDE e AI
estado visual
commands
integration wiring
```

### `axiom-ai-core`

Deve ser headless.

Não depende de:

```text
GPUI
WorkspaceView
EditorView
HTTP SDK específico
OpenAI
Anthropic
Hermes
```

Contém apenas contratos/tipos que tenham consumidor real.

### `axiom-ai-provider`

Integra modelos/providers.

Não depende de GPUI.

Não acessa diretamente owners da IDE.

Converte contratos neutros do core para protocolos concretos.

---

## 5.1 Regra contra ciclos

Nunca criar:

```text
axiom-index -> axiom-ai-*
axiom-editor -> axiom-ai-*
axiom-syntax -> axiom-ai-*
```

apenas para facilitar integração.

AI consome inteligência da IDE.

A inteligência determinística da IDE não depende de AI.

---

# 6. ModelProvider ≠ AgentRuntime

Essa separação é obrigatória.

## ModelProvider

Responsável por coisas como:

```text
models
request
stream
usage
capabilities
provider errors
```

## AgentRuntime

Responsável por:

```text
context
tools
permissions
model invocation
task lifecycle
cancellation
trace
evaluation
memory integration
```

Uma chamada a um modelo não é uma execução agentic.

Arquitetura conceitual:

```text
Axiom AI
├── Context
├── Tools
├── Trace
└── ModelProvider

AgentRuntime
├── NativeAxiomRuntime
└── future external runtime adapters
```

Hermes, Codex ou outro runtime externo nunca deve ditar o núcleo do Axiom.

---

# 7. `panic = "abort"` e isolamento

O profile release auditado utiliza:

```text
panic = "abort"
```

Consequência:

**thread != strong isolation**

Não prometer isolamento de falhas de runtime externo apenas com:

```text
thread
catch_unwind
```

Se integração futura exigir isolamento forte:

```text
process boundary
```

ou outra estratégia explicitamente validada deve ser considerada.

Essa decisão pertence a uma fase própria.

---

# 8. Guardrails permanentes de performance

AI roda **ao lado do editor**, nunca dentro do typing/render hot path.

Proibido introduzir:

```text
new document.content() per keystroke
full parsing per keystroke
filesystem canonicalization per keystroke
metadata calls per keystroke
directory walks
project-wide scans during typing
vendor-wide scans during typing
index rebuild during completion
blocking HTTP on UI thread
blocking model calls on UI thread
memory writes during typing
embedding generation during typing
UI-thread waits
unbounded queues
unbounded stream payloads
polling loops per token
```

Também preservar os guardrails históricos do editor:

```text
no heavy UI-thread work
no blocking locks/waits
prefer resident indexes/snapshots
reuse SymbolStore/by_fqn-style lookups
generation/session/revision guards
bounded background work
cooperative cancellation
```

Toda fase que tocar editor/completion/diagnostics/semantic/index/LSP deve reportar explicitamente impacto no hot path.

---

# 9. Situação do frontend visual de IA

O frontend visual foi iniciado antes da materialização do backend arquitetural.

Isso é aceitável porque essa etapa é uma **prototipação local de UX dentro de `axiom-app`**, sem contratos de provider ou runtime.

## 9.1 Estrutura visual já desenhada durante o protótipo

Direção atual:

```text
[Activity][Project][Editor][Axiom AI]
```

AI fica no extremo direito da janela.

O painel:

```text
- abre/fecha por toggle
- possui resize horizontal
- persiste visibilidade/largura
- não deve sobrepor o editor
- editor recebe viewport real restante
```

A área de AI possui:

```text
Chat
Agent
composer
Context
Model picker
Manage Providers
```

---

## 9.2 Model Picker local

O protótipo inclui picker visual/local com grupos como:

```text
OpenAI
Anthropic
```

Nenhum `ProviderId`, `ModelId` ou contrato real deve ser inferido a partir desse estado temporário.

O label escolhido no protótipo é apenas UI state.

---

## 9.3 Manage Providers local

Existe fluxo visual:

```text
Model Picker
    ↓
Manage Providers...
    ↓
AI Providers modal
```

A versão visual contém providers e controles fake/local.

Antes de considerar a UI finalizada, revisar:

```text
- status por provider deve ser único e não ambíguo
- evitar "Connected / Not configured" simultaneamente
- avaliar "Configure" como botão discreto em vez de link
- seletor que parece funcional deve ter comportamento coerente
- melhorar hierarquia entre provider, status e controles
- revisar seção Local / Future providers quando houver conteúdo real
```

Esses itens são polish visual, não contratos de domínio.

---

# 10. Handoff: o que fazer quando o frontend visual estiver concluído

**Não conectar API real diretamente do `WorkspaceView`.**

Quando o usuário declarar o frontend visual suficientemente aprovado, o agente deve parar a expansão visual e retomar a arquitetura.

Sequência recomendada:

```text
Visual frontend approved
        ↓
M.0.2 — minimal contracts
        ↓
materialize axiom-ai-core only when justified
        ↓
materialize axiom-ai-provider when first real provider exists
        ↓
SecretStore decision
        ↓
first provider
        ↓
streaming
        ↓
connect UI to real provider
```

---

# 11. M.0.2 — Próxima microfase recomendada após a UI visual

M.0.2 deve ser pequena e arquitetural.

## Objetivo

Especificar contratos mínimos antes de integrar SDK/HTTP/provider.

Cobrir:

```text
request/result identity
request lifecycle
cancellation identity
context/base revision
stale result policy
stream event semantics
bounded backpressure
provider failure semantics
SecretStore criteria
panic=abort isolation implications
```

## Não fazer ainda

```text
no provider SDK
no HTTP
no AgentRuntime
no tools
no memory
no Hermes
no MCP
no sub-agents
```

## Materialização de crates

O ADR M.0.1 deliberadamente **não criou crates vazios**.

Em M.0.2:

- definir primeiro o contrato;
- somente criar `axiom-ai-core` se já houver tipos/comportamento efetivamente necessários;
- provar que compila headless;
- provar ausência de GPUI na árvore de dependências;
- não criar `axiom-ai-provider` até existir uma implementação útil ou contrato que o justifique.

---

# 12. Protocolo obrigatório para agentes de código

Antes de cada microfase:

1. auditar o source relevante;
2. identificar o mecanismo existente que deve ser reutilizado;
3. declarar arquivos esperados;
4. implementar apenas um objetivo;
5. adicionar regressão permanente quando houver comportamento testável;
6. executar validações aplicáveis;
7. reportar impacto de performance/hot path;
8. não fazer commit/push salvo pedido explícito.

Preferir microfases.

Evitar prompts gigantes e mudanças horizontais desnecessárias.

### Validação mínima típica

Quando aplicável:

```powershell
cargo check -p axiom-app --locked
cargo test -p axiom-app --bin axiom --locked
cargo fmt --all -- --check
cargo build --release -p axiom-app --locked
```

Para crates novos, validar também o crate isoladamente.

Manual reproduction pelo usuário tem prioridade sobre alegações do agente.

---

# 13. Definition of architectural safety before first real provider

Antes de ligar a primeira API real, confirmar:

- [ ] `axiom-ai-core` é headless.
- [ ] GPUI não aparece em dependências do core.
- [ ] provider não acessa `WorkspaceView`/`EditorView` diretamente.
- [ ] existe identidade própria de request.
- [ ] existe política explícita para stale results.
- [ ] streaming possui trabalho limitado/bounded.
- [ ] UI não recebe atualização por token sem batching quando houver churn.
- [ ] cancelamento não depende de contador de edição.
- [ ] credenciais não entram em JSON/plaintext.
- [ ] SecretStore failure é graceful.
- [ ] provider failure não quebra o editor.
- [ ] AI desativada mantém Axiom funcional.
- [ ] nenhum caminho novo entra no typing/render hot path.
- [ ] `panic = "abort"` foi considerado para qualquer isolamento prometido.

---

# 14. Roadmap canônico completo — Phase M

A partir daqui está o roadmap funcional completo consolidado.  
Ele descreve o destino do produto. As seções anteriores descrevem **como esse destino deve respeitar o Axiom real**.

---

Sim. Como o ****Axiom parte de zero em IA****, eu evitaria começar por Hermes, Codex, ai-memory ou MCP. Essas peças entram depois que o Axiom tiver um ****núcleo próprio de AI bem definido****.

O roadmap abaixo é o que eu adotaria como uma nova grande etapa do projeto, mantendo o princípio que definimos:

> ****A IA é consumidora da inteligência do Axiom, nunca sua fonte de verdade.****

Ou seja: Index, Type Engine, PHPDoc, diagnostics, Composer, filesystem, Git, testes etc. continuam determinísticos. A IA recebe acesso a essas capacidades por ferramentas controladas.

**---**

**# Roadmap — Axiom AI & Agent Platform**

```text

M.0   Architecture Foundations

M.1   Model Provider Layer

M.2   AI Chat

M.3   Context Engine

M.4   Axiom Agent Tools — Read-only

M.5   Agent Runtime

M.6   Permissions & Safety

M.7   Mutating Tools + Diff / Review / Apply

M.8   Task Execution & Agent UX

M.9   Execution Traces & Evaluation

M.10  Persistent Memory

M.11  Skills Runtime

M.12  Closed Learning Loop

M.13  Hermes Integration

M.14  MCP

M.15  Sub-agents

M.16  Skill Evolution & Evaluations

M.17  Teams / Shared Knowledge

```

Eu não tentaria implementar tudo de uma vez.

Há três grandes marcos comerciais/técnicos:

```text

M.0–M.3

AI Assistant

M.4–M.9

Axiom Agent

M.10–M.16

Learning Agent Platform

```

**---**

**# M.0 — AI Architecture Foundations**

Essa é provavelmente a fase mais importante.

Ainda ****não implemente Chat****.

Primeiro fixe os contratos arquiteturais.

**## Objetivo**

Criar as abstrações que impedirão o Axiom de ficar acoplado a OpenAI, Claude, Hermes, Codex ou qualquer outro fornecedor.

A arquitetura base:

```text

                    Axiom AI

                       │

                Agent Runtime

                       │

        ┌──────────────┼──────────────┐

        │              │              │

      Context         Tools          Trace

        │              │              │

        │              │              │

        └──────────────┼──────────────┘

                       │

                    Model

                       │

             Model Provider API

                       │

        ┌──────────────┼──────────────┐

        ▼              ▼              ▼

      OpenAI        Anthropic        Local

```

E separadamente:

```text

Agent Runtime

      │

      ├── Axiom Native Runtime

      └── Hermes Adapter

```

****Model Provider e Agent Runtime são conceitos diferentes.****

Isso precisa estar decidido desde o começo.

**---**

**## Criar ADR**

Eu criaria algo como:

```text

docs/adr/ADR_AI_AGENT_ARCHITECTURE.md

```

Decisões fundamentais:

1\. Semantic systems do Axiom são autoritativos.

2\. Model Providers são intercambiáveis.

3\. Agent Runtime é independente do modelo.

4\. Agentes só interagem com a IDE através de Tools registradas.

5\. Tools mutáveis obedecem ao sistema de permissões.

6\. Toda execução agentic deve ser auditável.

7\. Memory, Context, Trace, Instructions e Skills são conceitos diferentes.

8\. Skills são conhecimento procedural.

9\. Memory não pode alterar Skills automaticamente.

10\. Learned Skills exigem validação/review.

11\. Nenhuma integração de IA pode entrar no typing hot path.

12\. Axiom continua plenamente funcional sem AI configurada.

**---**

**# Organização de crates**

Eu não colocaria tudo dentro de `axiom-app`.

Algo próximo de:

```text

crates/

axiom-ai-core/

axiom-ai-provider/

axiom-ai-context/

axiom-ai-tools/

axiom-agent/

axiom-ai-memory/

axiom-ai-skills/

```

E adapters:

```text

axiom-provider-openai/

axiom-provider-anthropic/

axiom-memory-ai-memory/

axiom-runtime-hermes/

```

Não precisam nascer todos agora.

Começaria com:

```text

axiom-ai-core

axiom-ai-provider

```

**---**

**# M.1 — Model Provider Layer**

Agora começamos com modelos.

**## Objetivo**

Permitir:

```text

Axiom

  ↓

ModelProvider

  ↓

qualquer modelo

```

Sem o restante da IDE saber se a resposta veio de OpenAI, Anthropic ou outro serviço.

**---**

**## Interface conceitual**

Algo próximo de:

```rust

trait ModelProvider {

    async fn models(&self) -> Result\<Vec\<ModelInfo>>;

    async fn complete(&self, request: CompletionRequest)

        -> Result\<CompletionStream>;

}

```

Mas não exponha estruturas específicas de uma API.

Evite:

```rust

OpenAIChatCompletionRequest

```

espalhado pelo Axiom.

Tenha tipos próprios:

```text

ModelRequest

ModelMessage

ModelResponse

ModelUsage

ModelError

ModelCapabilities

```

**---**

**## Capabilities**

Desde o começo, modele capacidades:

```text

ModelCapabilities

streaming

tool_use

vision

reasoning

structured_output

context_window

max_output

```

Porque futuramente o Agent Runtime poderá perguntar:

```text

Esse modelo suporta tool calling?

```

em vez de:

```text

if provider == OpenAI

```

**---**

**# Primeiro provider**

Eu implementaria ****apenas um****.

Provavelmente OpenAI ou o provider que você pretende usar no desenvolvimento do Axiom.

Não implemente:

```text

OpenAI

Anthropic

Gemini

OpenRouter

Ollama

LM Studio

...

```

simultaneamente.

Primeiro prove a abstração.

Depois adicione um segundo provider para testar se ela realmente é independente.

**---**

**# Configuração de credenciais**

Nunca:

```text

.axiom/config.toml

api_key = "..."

```

em texto simples dentro do projeto.

Crie uma abstração:

```text

SecretStore

```

e use armazenamento seguro do SO quando possível.

**---**

**# M.2 — AI Chat**

Somente agora aparece a primeira UI.

**## Objetivo**

Entregar:

```text

AI

└── Chat

```

Ainda sem agente alterando arquivos.

**---**

**## Primeira interface**

Algo simples:

```text

┌───────────────────────────────────────┐

│ Axiom AI                         GPT  │

├───────────────────────────────────────┤

│                                       │

│ User                                  │

│ Explique essa classe.                 │

│                                       │

│ Assistant                             │

│ Essa classe implementa...             │

│                                       │

├───────────────────────────────────────┤

│ Ask anything...                       │

└───────────────────────────────────────┘

```

Primeiro objetivo:

```text

prompt

→ provider

→ streaming

→ UI

```

Nada além disso.

**---**

**## Recursos necessários**

* streaming;

* cancelar generation;

* retry;

* provider/model selector;

* Markdown;

* syntax highlighting;

* copy;

* conversation history;

* token/usage metadata;

* erros claros.

**---**

**## Importante**

Streaming nunca deve bloquear GPUI/UI thread.

Fluxo:

```text

UI

 ↓

spawn async request

 ↓

provider

 ↓

stream channel

 ↓

batched UI updates

```

Evite renderizar uma vez por token se isso provocar churn excessivo.

**---**

**# M.3 — Context Engine**

Esse é o ponto onde Chat começa a ficar realmente integrado ao Axiom.

**## Problema**

Não faça isso:

```text

LLM:

"Leia todos os arquivos do projeto."

```

O Axiom já entende o projeto.

Use essa inteligência.

**---**

**## Criar `ContextEngine`**

Responsável por montar contexto controladamente.

Fontes:

```text

Context

├── active file

├── selected code

├── open files

├── diagnostics

├── symbols

├── Composer

├── PHP version

├── workspace metadata

└── user-added files

```

**---**

**## Progressive context**

Já aplicaria aqui o mesmo princípio que futuramente usaremos para Skills.

```text

Level 0

metadata

Level 1

relevant snippets

Level 2

full content only when necessary

```

Não mande o workspace inteiro para o modelo.

**---**

**# M.3.1 — Chat context commands**

Algo como:

```text

@file

@selection

@diagnostics

@symbol

@composer

@project

```

Exemplo:

```text

Explique @selection

```

ou:

```text

Por que @diagnostics está acusando isso?

```

**---**

**# Marco 1**

Aqui já temos:

```text

Axiom AI Assistant

```

com:

```text

✓ Providers

✓ Chat

✓ Streaming

✓ Context

✓ Semantic information

```

E ainda:

```text

✗ sem terminal

✗ sem edit

✗ sem agent loop

✗ sem memória

```

Eu lançaria internamente exatamente assim antes de avançar.

**---**

**# M.4 — Axiom Agent Tools**

Agora começa a parte mais importante.

O modelo não deve receber acesso arbitrário ao sistema.

Ele recebe ****Tools do Axiom****.

**---**

**# M.4.1 — Tool Registry**

Criar:

```text

ToolRegistry

```

Cada tool tem:

```text

name

description

schema

permission level

execution handler

result type

```

**---**

**# M.4.2 — Primeiras Tools: somente leitura**

Comece estritamente read-only.

**### Filesystem**

```text

read_file

list_directory

search_text

```

**### Editor**

```text

get_active_file

get_selection

get_open_files

```

**### PHP intelligence**

```text

find_symbol

find_definition

find_references

find_implementations

get_symbol_info

```

**### Diagnostics**

```text

get_diagnostics

```

**### Composer**

```text

get_composer_info

get_installed_packages

```

**### Project**

```text

get_project_info

```

**---**

**# E há uma regra muito importante**

Quando existe uma API semântica:

```text

find_references

```

o agente deve preferi-la sobre:

```text

grep pelo projeto inteiro

```

Essa é uma das vantagens competitivas do Axiom Agent.

**---**

**# M.4.3 — Resultados estruturados**

Não devolva somente texto.

Exemplo:

```text

find_symbol

```

deve produzir algo conceitualmente como:

```json

{

  "symbol": "App\\\User",

  "kind": "class",

  "file": "src/User.php",

  "range": "...",

  "source": "project-index"

}

```

Isso torna Agent Runtime, tracing e avaliação muito melhores.

**---**

**# M.5 — Agent Runtime**

Finalmente chegamos ao agente.

**## Agent loop**

Inicialmente:

```text

USER

 ↓

MODEL

 ↓

TOOL CALL

 ↓

TOOL RESULT

 ↓

MODEL

 ↓

TOOL CALL

 ↓

...

 ↓

FINAL

```

**---**

**## Componentes**

```text

AgentRuntime

├── AgentSession

├── Context

├── ToolRegistry

├── PermissionManager

├── ModelProvider

├── CancellationToken

└── EventStream

```

**---**

**## Eventos**

Eu usaria eventos internos:

```text

AgentStarted

ModelStarted

ToolRequested

ToolStarted

ToolFinished

ModelFinished

AgentFinished

AgentCancelled

AgentFailed

```

Isso será extremamente útil depois para traces e UI.

**---**

**# M.5.1 — Cancellation**

Isso precisa existir desde o primeiro agent loop.

```text

generation token

```

ou abstração semelhante.

Cancelamento deve chegar a:

```text

model request

tool execution

terminal command

search

```

Sempre que tecnicamente possível.

**---**

**# M.6 — Permission System**

Antes de permitir edição ou terminal, faça isso.

**## Categorias**

```text

READ

WRITE

EXECUTE

NETWORK

DESTRUCTIVE

```

Exemplo:

```text

read_file

READ

edit_file

WRITE

run_test

EXECUTE

composer_install

NETWORK + WRITE

git_reset_hard

DESTRUCTIVE

```

**---**

**# Modos**

Poderíamos ter:

**### Ask**

```text

Perguntar antes de qualquer ação mutável.

```

**### Auto-edit**

```text

Leitura + edição permitidas.

Terminal exige confirmação.

```

**### Agent**

```text

Permissões conforme policy configurada.

```

**---**

**# Nunca faça**

```text

LLM pede shell

→ execute diretamente

```

Sempre:

```text

Model

 ↓

Tool request

 ↓

PermissionManager

 ↓

Tool

```

**---**

**# M.7 — Mutating Tools**

Agora o agente pode modificar o projeto.

Primeiras ferramentas:

```text

edit_file

create_file

delete_file

rename_file

```

Mas edição deve gerar ****patch/diff****, não substituir silenciosamente arquivos.

**---**

**# M.7.1 — Proposed Change**

Fluxo:

```text

Agent

 ↓

edit_file

 ↓

ProposedChange

 ↓

Diff

 ↓

Review

 ↓

Apply

```

UI:

```diff

src/User.php

- public function save()

+ public function save(): void

```

Botões:

```text

[Accept]

[Reject]

[Accept All]

```

**---**

**# M.7.2 — Revision guard**

Aqui eu reutilizaria a filosofia das proteções que já colocamos em formatting.

A alteração deve carregar:

```text

document_session

edit_generation

original_hash

```

Se o usuário editar o documento depois da proposta:

```text

patch stale

```

Não aplique cegamente.

**---**

**# M.7.3 — Undo**

Qualquer modificação do agente precisa se integrar ao undo/redo.

Ideal:

```text

Agent Task

   ↓

one logical transaction

```

quando apropriado.

**---**

**# M.7.4 — Terminal Tools**

Somente depois das permissões.

Não comece com:

```text

shell(command)

```

absolutamente genérico.

Forneça primeiro tools semânticas:

```text

run_tests

run_phpunit

run_phpstan

run_composer_script

```

Depois, opcionalmente:

```text

run_terminal_command

```

**---**

**# M.8 — Task Execution / Agent UX**

Chat e Agent precisam ser experiências diferentes.

Eu teria:

```text

AI

├── Chat

└── Agent

```

Chat:

```text

responde

explica

analisa

```

Agent:

```text

planeja

usa tools

modifica

executa

valida

```

**---**

**## UI de execução**

```text

Agent Task

Fix the failing UserService tests

✓ Inspected UserService.php

✓ Found 3 diagnostics

✓ Read test

✓ Updated UserService.php

● Running PHPUnit

○ Review changes

```

**---**

**# M.8.1 — Tasks**

Começa a fazer sentido:

```text

AI

├── Chat

├── Agent

└── Tasks

```

Cada execução agentic vira uma task.

**---**

**# M.9 — Execution Trace**

Esse componente é essencial para tudo que vem depois.

Não armazene chain-of-thought.

Armazene ****eventos observáveis****.

```text

AgentRun

Task

Model

Provider

Runtime

Started

Finished

Outcome

Steps

├── tool calls

├── tool results

├── file mutations

├── diagnostics changes

├── tests

└── validation

```

**---**

**# Exemplo**

```text

Run #1832

Task

Fix failing PHPUnit tests

1\. get_diagnostics

   4 errors

2\. read_file

   UserService.php

3\. find_references

   17 references

4\. edit_file

   UserService.php

5\. run_tests

   184/184 PASS

Diagnostics

4 → 0

Outcome

SUCCESS

```

Esse é o material que futuramente alimentará aprendizado.

**---**

**# M.9.1 — Evaluator**

Não pergunte apenas ao modelo:

> funcionou?

O Axiom pode avaliar deterministicamente:

```text

Tests

Diagnostics

PHPStan

Composer

Git diff

exit codes

```

Exemplo:

```text

Evaluation

Tests             PASS

Diagnostics       4 → 0

PHPStan           PASS

Files changed     2

Unexpected files  0

Outcome

SUCCESS

```

Isso é muito mais poderoso que autoavaliação do LLM.

**---**

**# Marco 2 — Axiom Agent MVP**

Aqui temos um produto extremamente significativo:

```text

✓ Chat

✓ Providers

✓ Context

✓ Semantic tools

✓ Agent loop

✓ Permissions

✓ Diff review

✓ Terminal/tests

✓ Traces

✓ Deterministic evaluation

```

E ainda:

```text

✗ sem memória permanente

✗ sem skills

✗ sem Hermes

✗ sem sub-agents

```

Eu estabilizaria bastante esse ponto antes de avançar.

**---**

**# M.10 — Persistent Memory**

Agora entra o ****ai-memory****.

Não antes.

Até aqui o Axiom já deve possuir:

```text

Model Providers

Chat

Context Engine

Agent Tools

Agent Runtime

Permissions

Diff / Review / Apply

Tasks

Execution Traces

Evaluation

```

Somente depois disso faz sentido adicionar memória persistente.

A memória não será responsável por executar tarefas, decidir alterações ou substituir o Agent Runtime. Sua responsabilidade será outra:

> ****preservar conhecimento útil entre sessões, modelos, agentes e máquinas.****

O Axiom deve continuar funcionando normalmente mesmo quando nenhum backend de memória estiver disponível.

A arquitetura será:

```text

Axiom Agent Runtime

        │

        ▼

MemoryService

        │

        ├── NoMemory

        │

        └── AiMemoryBackend

                 │

                 ▼

              ai-memory

```

O `ai-memory` será a primeira implementação de memória persistente do Axiom, mas não deverá se tornar uma dependência arquitetural irreversível.

**---**

**# M.10.0 — Memory Architecture Contract**

Antes da integração com `ai-memory`, definir o contrato conceitual de memória do Axiom.

A primeira regra é separar explicitamente:

```text

Context

Memory

Instructions

Trace

Skills

```

Eles não são equivalentes.

**## Context**

Informação necessária para a execução atual.

Exemplos:

```text

active file

selection

diagnostics

symbols

related files

Composer metadata

```

Context pode ser temporário e desaparecer ao final da tarefa.

**---**

**## Memory**

Conhecimento persistente derivado de experiências anteriores.

Exemplos:

```text

este projeto usa Pest

PHPStan deve rodar em level 8

tentativas anteriores mostraram que X causa Y

a arquitetura deste módulo possui determinada restrição

essa abordagem falhou anteriormente

```

Memory ajuda futuras execuções, mas não representa automaticamente uma regra obrigatória.

**---**

**## Instructions**

Regras declaradas explicitamente pelo usuário, projeto ou organização.

Exemplo:

```text

Controllers não podem acessar Repository diretamente.

```

Instructions possuem autoridade maior do que inferências provenientes da memória.

**---**

**## Trace**

Registro observável de uma execução específica.

Exemplo:

```text

read_file

find_references

edit_file

run_tests

184/184 PASS

```

Trace é evidência histórica.

Não deve ser confundido com conhecimento consolidado.

**---**

**## Skills**

Conhecimento procedural reutilizável.

Exemplo:

```text

Como investigar e corrigir falhas de PHPUnit/Pest neste tipo de projeto.

```

Skills descrevem ****como fazer**** alguma coisa.

Memory pode ajudar a descobrir candidatos a Skills, mas Memory não altera Skills automaticamente.

**---**

**# M.10.1 — Portability and Source of Truth**

A memória do usuário não deve ficar presa ao banco interno do Axiom nem a um provider específico.

Princípios obrigatórios:

```text

human-readable where possible

open/exportable representation

no model-provider lock-in

no Agent Runtime lock-in

database indexes must be rebuildable

memory must survive provider changes

memory must survive runtime changes

```

A arquitetura deve seguir a filosofia utilizada pelo `ai-memory`:

```text

Human-readable knowledge

          =

source of truth

SQLite / FTS / embeddings

          =

derived indexes

```

Sempre que possível, conhecimento consolidado deve poder existir em formato aberto e legível, compatível com a ideia de OKF/Markdown.

Isso permite:

```text

backup

versioning

inspection

manual editing

migration

import/export

future backend replacement

```

O usuário não deve perder anos de memória acumulada simplesmente porque o Axiom trocou seu backend interno.

**---**

**# M.10.2 — MemoryService Abstraction**

Criar:

```text

MemoryService

```

O `AgentRuntime` não deve conhecer diretamente `ai-memory`.

Conceitualmente:

```rust

trait MemoryService {

    async fn briefing(...);

    async fn query(...);

    async fn session_start(...);

    async fn observe(...);

    async fn session_end(...);

    async fn handoff(...);

    async fn recent(...);

    async fn history(...);

}

```

A interface real deve ser desenhada de acordo com as necessidades do Agent Runtime e não copiar diretamente a API externa do `ai-memory`.

Implementações iniciais:

```text

NoMemory

AiMemoryBackend

```

Possíveis implementações futuras:

```text

AxiomMemoryBackend

RemoteTeamMemoryBackend

EnterpriseMemoryBackend

```

Isso garante:

```text

AgentRuntime

      │

      ▼

MemoryService

e não:

AgentRuntime

      │

      ▼

ai-memory-specific API

```

**---**

**# M.10.3 — ai-memory Adapter**

A primeira integração deve manter baixo acoplamento.

Inicialmente:

```text

Axiom

 ↓

MemoryService

 ↓

AiMemoryBackend

 ↓

local HTTP / MCP

 ↓

ai-memory

```

O Axiom pode gerenciar o processo local posteriormente, mas não deve incorporar os crates do `ai-memory` diretamente nessa primeira implementação.

Motivos:

```text

reduzir acoplamento

permitir upgrades independentes

facilitar experimentação

preservar substituição futura

evitar trazer detalhes internos do backend para o Agent Runtime

```

Somente depois de experiência real com a integração deve ser avaliado se faz sentido incorporar crates específicos diretamente no processo do Axiom.

**---**

**# M.10.4 — Memory Process Lifecycle**

Se o Axiom gerenciar uma instância local do `ai-memory`, o lifecycle deve ocorrer fora da UI thread.

Fluxo:

```text

Axiom starts

    │

    ▼

MemoryManager

    │

    ├── detect backend

    ├── start backend asynchronously

    ├── health check

    └── establish connection

```

A interface pode mostrar:

```text

Memory: Starting

Memory: Ready

Memory: Unavailable

Memory: Disabled

```

Nunca:

```text

IDE startup

   ↓

wait synchronously for memory server

```

Falha da memória não pode impedir o editor de abrir.

**---**

**# M.10.5 — What Gets Stored**

Somente eventos semanticamente relevantes provenientes do ****Agent Runtime**** devem alimentar a memória.

Exemplos:

```text

user task

important tool results

important discoveries

explicit decisions

important diagnostics

mutations

test results

validation results

task outcome

handoff information

repeated failures

repeated successful procedures

```

Exemplo:

```text

Task:

Fix failing UserService tests

Discovery:

Project uses Pest through composer test.

Changes:

UserService.php

Validation:

184/184 tests passed.

Important decision:

Use composer test instead of invoking vendor/bin/phpunit directly.

```

Isso é informação potencialmente útil em futuras sessões.

**---**

**# M.10.6 — What Must Never Enter the Memory Hot Path**

A memória não participa diretamente do ciclo de digitação.

Não enviar automaticamente:

```text

keypress

cursor movement

completion invocation

hover

every diagnostics refresh

every syntax parse

every render

every document revision

every semantic refresh

```

Também não realizar no typing hot path:

```text

HTTP calls

embedding generation

SQLite writes

memory lookup

memory consolidation

filesystem scans

```

A regra arquitetural é:

```text

Editor hot path

      │

      X

      │

Persistent Memory

```

A memória observa o ****Agent Runtime****, não o teclado.

**---**

**# M.10.7 — Session Capture**

Cada execução agentic importante pode criar uma sessão de memória.

Fluxo:

```text

Agent Task starts

      │

      ▼

memory.session_start()

      │

      ▼

Agent execution

      │

      ├── important observations

      ├── decisions

      ├── results

      └── outcome

      │

      ▼

memory.session_end()

```

Uma sessão deve preservar informação suficiente para responder posteriormente:

```text

O que estávamos tentando fazer?

O que descobrimos?

O que alteramos?

O que funcionou?

O que falhou?

O que ainda falta?

Quais decisões foram tomadas?

```

**---**

**# M.10.8 — Memory Briefing**

Antes de uma nova task, o Agent Runtime pode solicitar um briefing relevante.

Fluxo:

```text

User Task

   │

   ▼

AgentRuntime

   │

   ▼

MemoryService.briefing()

   │

   ▼

relevant project knowledge

   │

   ▼

ContextEngine

   │

   ▼

Model

```

Exemplo:

```text

User:

Corrija os testes deste módulo.

```

Memory briefing:

```text

Relevant project knowledge

- Tests use Pest.

- composer test is the canonical command.

- PHPStan level 8 must pass before completion.

- Previous failures in this module were caused by stale DTO assumptions.

```

Esse conteúdo deve ser tratado como contexto auxiliar, e não como verdade absoluta.

Quando houver conflito:

```text

current deterministic state

        >

memory inference

```

Por exemplo, se a memória diz:

```text

PHP version is 8.4

```

mas `composer.json` atual indica:

```text

PHP 8.5

```

o estado atual do projeto vence.

**---**

**# M.10.9 — Memory Query**

O Agent Runtime deve poder pesquisar memória sob demanda.

Exemplo:

```text

memory.query(

    "Previous decisions involving UserService"

)

```

A recuperação pode utilizar:

```text

full-text search

entity matching

semantic embeddings

graph relations

temporal relevance

authority / confidence

project scope

```

Mas o Agent Runtime não deve depender do mecanismo interno de ranking.

Ele recebe:

```text

MemoryResult[]

```

com metadados suficientes para avaliar a origem.

Exemplo:

```text

source

timestamp

scope

confidence

session

relations

```

**---**

**# M.10.10 — Temporal Memory**

A memória deve preservar histórico.

Não apenas:

```text

What do we know now?

```

Mas também, quando suportado:

```text

What did we know at time X?

```

Isso permite investigar:

```text

quando uma decisão surgiu

quando uma regra mudou

qual contexto existia antes de uma regressão

qual procedimento era usado anteriormente

```

Relações úteis podem incluir:

```text

causes

fixes

contradicts

supersedes

related_to

```

Exemplo:

```text

Completion latency regression

        │

        └── fixed_by

            resident prefix indexes

```

Isso transforma a memória em histórico técnico navegável, não apenas em texto recuperado por similaridade.

**---**

**# M.10.11 — Handoff**

Memória também deve permitir continuidade entre:

```text

sessions

models

providers

Agent Runtimes

eventualmente machines

```

Exemplo:

```text

Session #183

GPT

   │

   ├── investigated Type Engine

   ├── changed inference

   ├── 3 tests still failing

   │

   ▼

HANDOFF

   │

   ▼

Session #184

Claude

```

O novo agente recebe um resumo estruturado:

```text

Goal

Work completed

Files changed

Important discoveries

Current failures

Open questions

Recommended next actions

```

Isso não significa copiar todo o contexto anterior.

O objetivo é transferir apenas informação útil.

**---**

**# M.10.12 — Memory Authority**

Nem toda memória possui o mesmo peso.

O Axiom deve considerar futuramente níveis conceituais de autoridade:

```text

Explicit User Instruction

Project Instruction

Verified Project State

Accepted Decision

Consolidated Experience

Session Observation

Model Inference

```

Uma observação criada por um agente não pode sobrescrever silenciosamente uma regra explícita do usuário.

Exemplo:

```text

Project instruction:

Never modify generated migrations.

Memory observation:

Previous agent modified generated migration successfully.

```

Resultado:

```text

Project instruction wins.

```

**---**

**# M.10.13 — Consolidation**

Não transforme todos os eventos de uma sessão em páginas permanentes.

Fluxo conceitual:

```text

raw observations

      ↓

session

      ↓

consolidation

      ↓

useful knowledge

```

A consolidação deve tentar identificar:

```text

important decisions

stable project facts

repeated gotchas

successful procedures

failed approaches

unresolved questions

```

O restante pode permanecer apenas no trace ou expirar de acordo com política de retenção.

**---**

**# M.10.14 — Experience**

Depois de existir histórico suficiente, várias sessões podem ser analisadas em conjunto.

Exemplo:

```text

Session 12

Forgot composer script.

Session 31

Forgot composer script.

Session 57

Forgot composer script.

```

Uma única execução pode parecer irrelevante.

Três execuções mostram um padrão.

Fluxo:

```text

Sessions

   ↓

Experience analysis

   ↓

Repeated pattern

```

Exemplo:

```text

Before running PHPUnit directly,

inspect composer scripts because this project

wraps Pest through composer test.

```

Esse conhecimento pode inicialmente permanecer como uma:

```text

Procedure Memory

```

Mas isso ainda ****não é automaticamente um Axiom Skill****.

**---**

**# M.10.15 — Memory → Skill Boundary**

Manter uma fronteira explícita:

```text

Memory

   │

   ▼

Experience

   │

   ▼

Procedure Candidate

   │

   ▼

Skill Candidate

   │

   ▼

Validation

   │

   ▼

Human Review

   │

   ▼

SKILL.md

```

Nunca:

```text

Memory detects pattern

      ↓

rewrites active SKILL.md

```

O sistema de memória pode sugerir:

```text

A reusable procedure appears to exist.

```

Mas o `Skills Runtime` é responsável por transformar isso em conhecimento procedural executável.

**---**

**# M.10.16 — Human Review for Memory Evolution**

Alterações importantes em memória de alta autoridade devem poder exigir revisão humana.

Especialmente:

```text

project rules

procedures

architecture decisions

shared team knowledge

knowledge that contradicts previous accepted knowledge

```

Exemplo:

```text

Memory Improvement Proposal

Current:

Run vendor/bin/phpunit.

Proposed:

Use composer test because it invokes Pest with

project-specific configuration.

Evidence:

8 sessions

7 successful

1 failed

[Review]

[Accept]

[Reject]

```

Para conhecimento procedural que possa influenciar futuras execuções, o default deve favorecer revisão, não autoaprovação silenciosa.

**---**

**# M.10.17 — Memory Scopes**

A memória deve reconhecer escopos diferentes.

Inicialmente:

```text

User

Workspace

Project

```

Futuramente:

```text

Organization

Team

```

Exemplo:

```text

User Memory

Personal preferences.

Workspace Memory

Knowledge shared across related projects.

Project Memory

Knowledge specific to this repository.

Organization Memory

Institutional knowledge.

```

Escopos devem ser explícitos para evitar contaminação.

Uma decisão específica de um projeto Laravel não deve aparecer como verdade em outro projeto não relacionado.

**---**

**# M.10.18 — Project Identity**

A identificação de projeto precisa ser estável.

A memória não deve depender exclusivamente de:

```text

C:\dev\Axiom

```

porque o mesmo projeto pode existir em:

```text

E:\dev\Axiom

/home/user/Axiom

worktree A

worktree B

another machine

```

O backend deve possuir conceito estável de:

```text

workspace

project identity

checkout/path

```

A identidade lógica deve ser separada do caminho físico.

**---**

**# M.10.19 — Memory UI**

Adicionar:

```text

AI

├── Chat

├── Agent

├── Tasks

└── Memory

```

A primeira versão pode conter:

```text

Overview

Sessions

Decisions

Gotchas

Procedures

Timeline

```

Posteriormente:

```text

Entities

Relationships

History

Proposals

```

**---**

**## Overview**

Exemplo:

```text

Project Memory

Project            Axiom

Status             Ready

Sessions           47

Decisions          18

Gotchas             9

Procedures          6

Last activity      Today

```

**---**

**## Sessions**

```text

#184 Fix type inference

Success

Today

#183 Formatting regression

Success

Yesterday

#182 Completion investigation

Partial

```

**---**

**## Decisions**

```text

Avoid project-wide FQN scans

Status         Active

Evidence       9 sessions

First learned  Aug 28

Updated        Sep 11

Reason

Use resident SymbolStore::by_fqn lookups instead

of scans across project/vendor scopes.

```

**---**

**## Gotchas**

Exemplo:

```text

Formatting responses can become stale

after document revision.

Related:

document_session

edit_generation

```

**---**

**## Procedures**

Procedures provenientes de memória aparecem aqui, mas precisam ser claramente diferenciadas de Skills.

```text

Procedure Memory

Run composer test before direct PHPUnit invocation.

Evidence:

8 sessions

```

Possível ação:

```text

[Create Skill Candidate]

```

**---**

**## Timeline**

Mostrar evolução temporal:

```text

Aug 28

Decision created

Sep 02

Contradicting observation recorded

Sep 04

Decision updated

Sep 11

Procedure extracted

```

**---**

**# M.10.20 — Memory Editing**

Memória não deve ser uma caixa-preta.

O usuário deve conseguir:

```text

Open

Inspect

Edit

Pin

Archive

Forget

View history

View evidence

```

Quando a fonte for Markdown/OKF, o conteúdo pode inclusive ser aberto no editor normal do Axiom.

Isso torna memória:

```text

visible

inspectable

correctable

portable

```

**---**

**# M.10.21 — Forget / Retention**

O sistema precisa suportar remoção e retenção.

Exemplos:

```text

Forget this memory.

Forget memories from this session.

Forget memories related to this project.

Do not retain terminal output matching secrets.

```

Também deve existir política para evitar que informação irrelevante cresça indefinidamente.

Categorias podem ter políticas diferentes:

```text

raw observations      short-lived

sessions              medium/long

accepted decisions    long-lived

procedures            long-lived

temporary failures    decayable

```

**---**

**# M.10.22 — Privacy and Sanitization**

Antes de persistir informação, aplicar sanitização.

Nunca memorizar automaticamente:

```text

API keys

passwords

access tokens

private keys

authentication headers

known secret patterns

```

Ferramentas que retornem valores sensíveis devem marcar resultados apropriadamente.

Idealmente:

```text

ToolResult

├── content

└── sensitivity

```

e o `MemoryService` recebe apenas conteúdo permitido.

**---**

**# M.10.23 — Local First**

A política padrão do Axiom deve ser:

```text

Memory storage       Local

Embeddings           Local

Cloud sync           Off

Team sync            Off

Remote memory        Off

```

A utilização de memória não deve exigir uma conta de cloud.

Recursos externos devem ser opt-in.

**---**

**# M.10.24 — Local Embeddings**

Quando semantic retrieval estiver habilitado:

```text

Embeddings

    ↓

local by default

```

Não enviar automaticamente conhecimento do projeto para um serviço externo apenas para produzir embeddings.

O backend pode utilizar embeddings locais quando disponível.

Se embeddings estiverem indisponíveis:

```text

FTS / symbolic retrieval

```

continua funcionando.

**---**

**# M.10.25 — Failure Isolation**

Memory é um subsistema opcional.

Portanto:

```text

memory unavailable

       ↓

Agent continues

       ↓

without persistent memory

```

Não:

```text

memory unavailable

       ↓

Agent fails entirely

```

Cada chamada de memória deve possuir:

```text

timeout

cancellation

bounded work

graceful fallback

```

**---**

**# M.10.26 — Performance Guardrails**

O backend de memória nunca pode causar regressões na responsividade do editor.

Proibido:

```text

blocking UI thread

SQLite writes on UI thread

embedding generation on UI thread

memory retrieval during typing

full-project scans for memory

unbounded queues

unbounded session payloads

```

Operações devem ocorrer:

```text

async

background

bounded

cancelable

```

Quando possível, eventos devem ser enviados de maneira fire-and-forget ou através de filas limitadas.

**---**

**# M.10.27 — Trace vs Memory**

O Axiom continua sendo responsável pelo trace primário.

Arquitetura:

```text

Agent Runtime

      │

      ▼

Axiom Trace Store

      │

      ├───────────────┐

      ▼               ▼

Evaluator          MemoryService

                      │

                      ▼

                  ai-memory

```

Exemplo:

Trace:

```text

Tool call #31

find_references

17 ms

```

Memory:

```text

Resident semantic reference lookup is preferred

over project-wide textual search.

```

Não persistir telemetria bruta como memória consolidada sem necessidade.

**---**

**# M.10.28 — Deterministic State Beats Memory**

A memória nunca substitui o estado atual do Axiom.

Ordem conceitual:

```text

Current deterministic project state

            >

Accepted explicit instructions

            >

Historical memory

            >

Model inference

```

Exemplo:

```text

Memory:

Tests use PHPUnit.

Current composer.json:

Pest installed and composer test invokes Pest.

```

O agente deve trabalhar com o estado atual e pode atualizar ou contradizer a memória antiga.

**---**

**# M.10.29 — Initial Integration Scope**

A primeira versão não precisa implementar tudo.

O primeiro milestone deve entregar somente:

```text

MemoryService abstraction

AiMemoryBackend

session start/end

important observations

memory briefing

memory query

basic handoff

basic Memory UI

local-first configuration

```

Deixar para iterações posteriores:

```text

temporal graph

advanced authority

experience analysis

skill candidates

team memory

cloud synchronization

organization scopes

```

**---**

**# M.10.30 — Definition of Done**

A fase inicial de Persistent Memory pode ser considerada funcional quando:

```text

1\. Agent Runtime works normally with NoMemory.

2\. ai-memory can be enabled without changing Agent Runtime logic.

3\. New agent sessions can retrieve useful knowledge

   from previous sessions.

4\. Memory is scoped correctly per project/workspace.

5\. Model/provider can change without losing memory.

6\. Memory backend can fail without freezing or breaking the IDE.

7\. No memory operation occurs in the typing hot path.

8\. Sensitive information is filtered before persistence.

9\. User can inspect the main memories stored for a project.

10\. Memory cannot modify active Skills automatically.

11\. Historical memory never overrides current deterministic

    project state.

12\. Stored knowledge can be exported or inspected in an

    open/human-readable representation.

```

**---**

**# Resulting Architecture**

Ao final desta fase:

```text

                       Axiom Agent Runtime

                                │

             ┌──────────────────┼──────────────────┐

             │                  │                  │

             ▼                  ▼                  ▼

          Context              Trace             Memory

             │                  │                  │

     Axiom semantic         AgentRun        MemoryService

      intelligence          Steps                │

             │              Results              │

             │                                   ▼

             │                              AiMemoryBackend

             │                                   │

             │                                   ▼

             │                               ai-memory

             │                                   │

             │                       ┌───────────┼───────────┐

             │                       ▼           ▼           ▼

             │                    Sessions   Knowledge   Experience

             │

             └──────────────────────────────────────────────┐

                                                            │

                                                            ▼

                                                     Future Skills

                                                        Pipeline

```

A principal regra dessa fase permanece:

> ****Memory helps the Axiom Agent remember. It does not decide what is true, and it does not autonomously decide what becomes a Skill.****

A responsabilidade continua dividida:

```text

Axiom

    understands the current codebase

Agent Runtime

    executes work

Trace

    records what happened

Evaluator

    measures what happened

Memory

    preserves useful experience

Skills

    encode reusable procedures

Human

    governs permanent procedural evolution

```



**# M.10.5 — Local first**

Minha política padrão seria:

```text

Memory storage       Local

Embeddings           Local

Cloud sync           Off

Team sync            Off

```

**---**

**# M.11 — Skills Runtime**

Agora finalmente implementamos ****procedural knowledge****.

Não confundir:

```text

Memory:

"Projeto usa Pest."

Instruction:

"Sempre execute PHPStan."

Skill:

"Procedimento para investigar testes falhando."

```

**---**

**# Estrutura**

```text

\~/.axiom/ai/skills/

project/.axiom/ai/skills/

```

Skill:

```text

phpunit-debugging/

├── SKILL.md

└── references/

```

**---**

**# Scopes**

Eu manteria:

```text

Built-in

User

Workspace

Project

```

Com projeto tendo maior prioridade.

**---**

**# M.11.1 — Progressive Disclosure**

Muito importante.

**### Level 0**

O modelo recebe apenas:

```text

name

description

tags

scope

tool requirements

```

Exemplo:

```text

phpunit-debugging

Diagnose and repair failing PHPUnit/Pest tests.

```

**### Level 1**

Somente quando selecionada:

```text

SKILL.md

```

**### Level 2**

Somente se necessário:

```text

references/

examples/

templates/

```

Assim 500 skills não viram 500 skills no prompt.

**---**

**# M.11.2 — Skill retrieval**

Inicialmente não precisa de embeddings complexos.

Use combinação:

```text

description

tags

task classification

project scope

recent success

```

Depois podemos evoluir.

**---**

**# M.12 — Closed Learning Loop**

Agora juntamos tudo.

Aqui nasce a arquitetura inspirada no Hermes:

```text

EXECUTE

   ↓

EVALUATE

   ↓

EXTRACT

   ↓

RETRIEVE

   ↓

EXECUTE

```

Mas eu faria o Axiom mais conservador:

```text

EXECUTE

   ↓

TRACE

   ↓

EVALUATE

   ↓

MEMORY

   ↓

EXPERIENCE

   ↓

CANDIDATE

   ↓

VALIDATE

   ↓

REVIEW

   ↓

SKILL

   ↓

RETRIEVE

```

**---**

**# M.12.1 — Experience Detection**

Várias sessões mostram o mesmo padrão.

Exemplo:

```text

Run 14

phpunit falhou porque composer script não foi usado

Run 31

mesmo problema

Run 52

mesmo problema

```

O sistema identifica:

```text

Potential procedure

```

**---**

**# M.12.2 — Skill Candidate**

Nunca:

```text

AI aprende

 ↓

reescreve SKILL.md

```

Sempre:

```text

AI learns

 ↓

SkillCandidate

```

**---**

**# M.12.3 — Evidence**

UI:

```text

Improve phpunit-debugging?

Evidence

────────────────────────

Executions          8

Successful          7

Failed              1

Proposed change

+ Check composer scripts before

+ invoking vendor/bin/phpunit.

[Review]

[Accept]

[Reject]

```

**---**

**# M.12.4 — Validation**

Antes do usuário aceitar, podemos futuramente avaliar o novo procedimento contra traces anteriores.

```text

Skill v1

vs

Skill candidate v2

```

**---**

**# M.13 — Hermes Integration**

****Só agora eu integraria Hermes.****

Não como núcleo do Axiom.

Como runtime alternativo.

```text

AgentRuntime

├── NativeAxiomRuntime

└── HermesRuntime

```

Isso significa que Hermes poderá utilizar:

```text

Axiom tools

Axiom memory

Axiom skills

Axiom permissions

```

sem controlar a arquitetura da IDE.

**---**

**# Benefício**

Se Hermes desaparecer amanhã:

```text

Axiom Agent continua funcionando.

```

Se surgir runtime melhor:

```text

AgentRuntime

└── NewRuntime

```

**---**

**# M.14 — MCP**

Eu colocaria MCP surpreendentemente tarde.

Porque MCP deve ampliar um Agent Runtime que já funciona, não substituí-lo.

**---**

**## MCP Client**

Permitir que o Axiom use servidores externos:

```text

GitHub

PostgreSQL

browser

documentation

custom tools

```

Fluxo:

```text

MCP Server

 ↓

MCP Tool

 ↓

Axiom Tool Registry

 ↓

Permission Manager

 ↓

Agent

```

MCP nunca ignora as permissões do Axiom.

**---**

**# M.14.1 — MCP Server do próprio Axiom**

Depois podemos fazer o inverso:

```text

Axiom

 ↓

MCP Server

```

Expondo:

```text

find_symbol

find_references

diagnostics

type_information

composer

project information

```

Assim agentes externos poderiam utilizar a inteligência semântica do Axiom.

Isso pode virar uma feature muito interessante por si só.

**---**

**# M.15 — Sub-agents**

Somente aqui.

Não antes.

Porque sub-agents multiplicam complexidade:

```text

tokens

permissions

tracing

concurrency

context

memory

failures

```

**---**

**# Arquitetura**

```text

                Lead Agent

                    │

        ┌───────────┼───────────┐

        ▼           ▼           ▼

     Composer      Code        Tests

      Agent        Agent       Agent

        │           │           │

        └───────────┼───────────┘

                    ▼

                 Lead

                    ▼

               Final Diff

```

Cada filho recebe contexto isolado.

Retorna apenas resultado estruturado.

**---**

**# M.15.1 — Context isolation**

Não:

```text

Lead context

+

all sub-agent conversations

```

Faça:

```text

Lead

 ↓

delegates task

Child

 ↓

isolated context

Child

 ↓

summary/result

Lead

```

Isso é a ideia que gostamos no Hermes.

**---**

**# M.16 — Skill Evolution & Evaluation**

Agora começamos algo realmente sofisticado.

Cada Skill possui histórico:

```text

v1.0

v1.1

v1.2

```

E métricas fora do `SKILL.md`.

```text

executions

success rate

failure rate

tool calls

cost

latency

project distribution

model distribution

```

**---**

**# Não coloque métricas no SKILL.md**

Separe:

```text

SKILL.md

= procedure

SQLite

= measurements

```

**---**

**# Skill UI**

```text

phpunit-debugging

Version          1.4

Executions        47

Success          93.6%

Last used        Today

Tools

✓ tests

✓ composer

✓ diagnostics

[Open Skill]

[History]

[Improve]

[Disable]

```

**---**

**# M.17 — Teams / Shared Knowledge**

Isso só seria necessário bastante depois.

Aqui entra o conceito de ****conhecimento institucional****.

Imagine uma empresa usando Axiom.

```text

Organization

│

├── global instructions

├── architecture decisions

├── coding procedures

├── project memory

└── team skills

```

Novos desenvolvedores e agentes herdam:

```text

"Como fazemos deploy."

"Como debugamos pagamentos."

"Como adicionamos endpoints."

"Quais arquiteturas são proibidas."

```

Nesse estágio o diferencial já não é simplesmente:

> Axiom possui GPT.

Passa a ser:

> ****Axiom acumula e operacionaliza o conhecimento técnico da equipe.****

**---**

**# Estrutura final**

Eu imagino a área de AI terminando aproximadamente assim:

```text

AI

│

├── Chat

│

├── Agent

│

├── Tasks

│

├── Memory

│   ├── Sessions

│   ├── Decisions

│   ├── Gotchas

│   ├── Procedures

│   └── Timeline

│

├── Skills

│   ├── Built-in

│   ├── User

│   ├── Project

│   └── Learned

│

├── History

│

└── Settings

    ├── Providers

    ├── Models

    ├── Agents

    ├── Permissions

    ├── Memory

    └── MCP

```

**---**

**# Como ficaria a arquitetura final**

```text

                              USER

                                │

                                ▼

                        ┌──────────────┐

                        │   Axiom AI   │

                        └──────┬───────┘

                               │

                        Agent Runtime

                               │

          ┌────────────────────┼─────────────────────┐

          │                    │                     │

          ▼                    ▼                     ▼

       Context                Tools                 Trace

          │                    │                     │

          │           ┌────────┼─────────┐           │

          │           ▼        ▼         ▼           │

          │         Files    Editor    Terminal       │

          │           │        │         │           │

          │       Symbols   Changes    Tests          │

          │       Types     Git        Composer       │

          │                                           

          ▼

   Deterministic Axiom

   Intelligence

          │

   ┌──────┼────────┐

   ▼      ▼        ▼

 Index   Type   Diagnostics

         Engine

          │

          └────────────────────┐

                               ▼

                        Model Provider

                               │

                  ┌────────────┼────────────┐

                  ▼            ▼            ▼

               OpenAI      Anthropic      Local

                               │

                    ┌──────────┴──────────┐

                    ▼                     ▼

                  Memory                Skills

                    │                     │

               ai-memory              SKILL.md

                    │                     │

               Experience               │

                    │                    │

                    └──── Candidate ─────┘

                              │

                           Validate

                              │

                            Review

```

**---**

**# O que eu NÃO faria**

Há vários atalhos tentadores que eu evitaria.

****Não começaria colocando um WebView de ChatGPT dentro do Axiom.**** Isso não cria infraestrutura reutilizável.

****Não começaria por Codex.**** Codex seria uma integração/provider/runtime posterior, não a arquitetura.

****Não começaria por Hermes.**** Primeiro o Axiom precisa possuir Tools, permissions, trace, diff e contexto próprios.

****Não começaria por ai-memory.**** Não existe memória útil se ainda não existe Agent Runtime produzindo experiências estruturadas.

****Não começaria por MCP.**** MCP amplia Tools; ele não substitui o Tool Runtime.

****Não criaria sub-agents cedo.**** Um agente confiável vale muito mais que quatro agentes imprevisíveis.

****Não daria shell irrestrito ao modelo.****

****Não deixaria memória modificar Skills automaticamente.****

****Não colocaria AI em completion/typing inicialmente.****

**---**

**# Guardrails de performance**

Esta parte eu tornaria requisito oficial da fase M.

Nada de AI deve introduzir no editor:

```text

new document.content() per keystroke

full parsing per keystroke

filesystem canonicalization per keystroke

directory walks

project-wide scans

vendor-wide scans

blocking HTTP

blocking model calls

memory writes on typing

embedding generation on typing

UI-thread waits

```

AI roda ****ao lado**** do editor.

Não dentro do hot path dele.

Arquitetura:

```text

Editor hot path

     │

     X

     │

 AI Runtime

```

Comunicação apenas através de snapshots/eventos/commands bem delimitados.

Isso é especialmente importante no Axiom porque já sabemos que estabilidade durante digitação rápida é uma área que não podemos regredir.

**---**

**# Ordem em relação ao roadmap atual**

Eu ainda manteria:

```text

K

↓

L — Type Engine + PHPDoc

↓

M — AI

```

Porque cada melhoria em L vira imediatamente uma Tool melhor para M.

Por exemplo, depois de L:

```text

get_inferred_type

get_phpdoc_type

get_return_type

get_parameter_types

resolve_generic_type

```

podem ser expostas ao agente.

Sem isso o LLM precisa inferir lendo texto.

Com isso:

```text

Agent

 ↓

Axiom Type Engine

 ↓

fact

```

Muito melhor.

**---**

**# Roadmap resumido por dependência**

```text

                     M.0 Architecture

                            │

                            ▼

                     M.1 Providers

                            │

                            ▼

                       M.2 Chat

                            │

                            ▼

                      M.3 Context

                            │

                            ▼

                  M.4 Read-only Tools

                            │

                            ▼

                    M.5 Agent Runtime

                            │

                            ▼

                    M.6 Permissions

                            │

                            ▼

                 M.7 Mutating Tools

                            │

                            ▼

                  M.8 Agent Tasks/UI

                            │

                            ▼

                    M.9 Trace/Evals

                            │

                  ┌─────────┴─────────┐

                  ▼                   ▼

             M.10 Memory          M.11 Skills

                  │                   │

                  └─────────┬─────────┘

                            ▼

                  M.12 Learning Loop

                            │

                  ┌─────────┼─────────┐

                  ▼         ▼         ▼

             M.13 Hermes  M.14 MCP  M.15 Subagents

                  │         │         │

                  └─────────┼─────────┘

                            ▼

                  M.16 Skill Evolution

                            │

                            ▼

                    M.17 Team Memory

```

**---**

**# E eu dividiria o desenvolvimento em quatro releases internas**

**### Axiom AI Alpha — Assistant**

```text

M.0–M.3

Provider

Chat

Streaming

Context

```

Objetivo:

> conversar com o modelo sobre código do projeto.

**---**

**### Axiom AI Beta — Agent**

```text

M.4–M.9

Tools

Agent Runtime

Permissions

Edit

Diff

Terminal

Tasks

Trace

Evaluation

```

Objetivo:

> entregar uma tarefa completa com segurança.

Esse é, na minha opinião, o ****primeiro grande milestone realmente valioso****.

**---**

**### Axiom AI Learning — Memory & Skills**

```text

M.10–M.12

ai-memory

Skills

Experience

Closed Learning Loop

```

Objetivo:

> parar de começar do zero a cada execução.

**---**

**### Axiom Agent Platform**

```text

M.13–M.17

Hermes

MCP

Sub-agents

Skill evolution

Team knowledge

```

Objetivo:

> transformar o Axiom de uma IDE com agente em uma plataforma agentic.

**---**

**## A visão que eu manteria como norte**

A maior armadilha seria construir:

```text

Axiom

+

ChatGPT sidebar

```

Isso qualquer editor pode ter.

O objetivo que estamos desenhando é bem diferente:

```text

            Axiom

understands code deterministically

              +

lets agents operate on that knowledge

              +

remembers successful experience

              +

turns repeated experience into procedures

              +

allows humans to govern what is learned

```

Então a vantagem competitiva futura não seria simplesmente ****“Axiom tem IA”****.

Seria:

> ****O Axiom entende o projeto, fornece ferramentas semânticas confiáveis aos agentes, aprende procedimentos com o trabalho realizado e acumula conhecimento útil sem transformar o LLM na fonte de verdade.****

Esse é o roadmap que eu congelaria hoje como a futura ****Phase M — Axiom AI & Agent Platform****. E eu colocaria a ADR de M.0 já durante o final da fase L, sem implementar AI ainda, para que todas as decisões futuras do Type Engine, Diagnostics, Index e PHPDoc possam considerar que essas capacidades posteriormente serão expostas como Tools do Agent Runtime.

---

# 15. Regra final para qualquer agente que receber este arquivo

Não transforme este roadmap em uma lista para implementar mecanicamente.

A sequência correta é sempre:

```text
audit current source
        ↓
identify smallest missing contract
        ↓
implement one microphase
        ↓
test
        ↓
manual validation when UX/editor behavior is involved
        ↓
only then advance
```

A vantagem competitiva pretendida não é:

```text
Axiom + generic chatbot
```

É:

```text
Axiom understands the project deterministically
        +
agents operate through controlled semantic tools
        +
execution is observable and governable
        +
useful experience can be retained
        +
repeated validated experience can become procedures
        +
humans remain in control of what becomes durable knowledge
```

## Non-negotiable summary

```text
AI is not source of truth.
ModelProvider is not AgentRuntime.
Context is not Memory.
Memory is not Skills.
Trace is not chain-of-thought.
Tools obey permissions.
Mutations use diff/review/revision guards.
Secrets never fall back to plaintext.
External runtimes are adapters.
Current project state beats historical memory.
AI never enters the typing hot path.
Axiom works without AI.
```

---

**End of unified architecture and roadmap guide.**
