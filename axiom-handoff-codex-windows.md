# HANDOFF CANÔNICO — PROJETO AXIOM

## 1. Objetivo e arquitetura geral

Axiom é uma IDE em Rust para desenvolvimento PHP, composta por uma aplicação GPUI e crates especializados para edição, projeto, sintaxe, índices, LSP, terminal e integração de modelos.

A arquitetura atual separa:

- `axiom-app`: composição da UI GPUI, `WorkspaceView`, chat, Agent e adapters;
- `axiom-editor`: documentos, buffer, seleção e histórico de edição;
- `axiom-project`: identidade de projeto, Composer e capabilities seguras de leitura;
- `axiom-syntax`: parsing incremental;
- `axiom-php`: símbolos e informações de runtime PHP;
- `axiom-index`: índices, referências e snapshots semânticos;
- `axiom-lsp`: cliente LSP genérico;
- `axiom-terminal`: PTY e emulação;
- `axiom-ai-provider`: contratos e adapters de providers, incluindo Ollama;
- `axiom-web`: capability `fetch_url`;
- `axiom-agent`: runtime headless de Agent introduzido em M.5.

O Chat já possui integração real com Ollama, streaming, Thinking, Markdown, ferramentas read-only, histórico persistente e múltiplas proteções de lifecycle. O Agent agora possui um runtime headless próprio e uma ponte mínima para a UI.

A sequência de milestones considerada autoridade para a evolução foi a do roadmap `AXIOM_M10_PERSISTENT_MEMORY_ROADMAP.md`:

```text
M.2 Chat
→ M.3 Context
→ M.4 Read-only Tools
→ M.5 Agent Runtime
→ M.6 Permissions
→ M.7 Mutating Tools
→ M.9 Trace/Evals
→ M.10 Memory
```

Documentos arquiteturais antigos, incluindo ADR 0007 e roadmaps anteriores, não devem ser usados isoladamente para alterar essa sequência.

---

## 2. Invariantes arquiteturais

Estas regras foram repetidamente mantidas e devem continuar sendo respeitadas:

- IA não pode entrar no hot path de typing, renderização, completion, cursor movement ou selection change.
- Não adicionar `document.content()` por tecla, render, completion, cursor ou seleção.
- Provider, HTTP, filesystem e execução de tools devem ocorrer fora da thread da UI.
- A UI não deve executar `recv()`, `wait`, `join` ou qualquer operação bloqueante.
- Não introduzir runtime Tokio local.
- Não criar threads não gerenciadas para fazer a ponte de AI.
- Tarefas assíncronas devem usar os executores e mecanismos já existentes do GPUI.
- Resultados atrasados precisam ser descartados por identidade de request/run.
- Chat e Agent possuem identidades e sessões independentes.
- Chat continua usando sua própria orchestration em `axiom-app`; não deve ser migrado automaticamente para `AgentExecutor`.
- `axiom-agent` deve permanecer headless e não conhecer GPUI, `WorkspaceView`, Ollama, `ToolRegistry` ou capabilities específicas de projeto.
- Ferramentas atuais são somente read-only.
- Não introduzir mutações, shell, terminal, processos, permissões ou aprovações antes de M.5-E/M.6/M.7.
- Não criar Memory, embeddings, MCP ou integração externa de memória antecipadamente.
- Tool protocol interno não deve aparecer como mensagens visíveis do usuário.
- Thinking antigo não deve ser reenviado automaticamente como contexto textual do Agent.
- Falhas de tool antigas não podem alterar o estado de disponibilidade global do provider nem contaminar uma nova request.

---

## 3. Milestones concluídos

### M.3 — Context

Concluído.

Entregas principais:

- `ContextSnapshot` mínimo.
- Seleção explícita de fontes de contexto.
- Context chip na UI.
- Fontes explícitas como arquivo ativo, seleção e contexto selecionado.
- Contexto vinculado ao turno atual do usuário.
- Serialização limitada e segura.
- Testes de:
  - labels das fontes;
  - seleção versus arquivo ativo;
  - limites UTF-8;
  - payload enviado ao provider;
  - isolamento do contexto entre turns.

Correções posteriores:

- M.3-B1: interação do chip e comportamento de contexto.
- M.3-B2: lifecycle explícito e fonte ativa.
- M.3-B3: grounding explícito da solicitação.
- M.3-B4: contexto vinculado ao turno atual do usuário.

Limitação atual:

- O Agent não herda automaticamente o `ContextSnapshot` visual do Chat.
- O chip pode continuar visível na UI, mas o Agent usa a sessão visível própria.
- A integração explícita do contexto com Agent ainda não foi implementada.

---

### M.4 — Read-only Tools e Chat Orchestration

Concluído.

Ferramentas atuais:

- `read_file`
- `list_directory`
- `fetch_url`

Componentes principais:

- `crates/axiom-app/src/ai/tools/mod.rs`
- `crates/axiom-app/src/ai/tool_orchestration.rs`
- `crates/axiom-project/src/project_read.rs`
- capability de diretórios em `axiom-project`
- `crates/axiom-web`
- integração Ollama em `axiom-ai-provider`

Proteções implementadas:

- somente paths relativos;
- rejeição de caminhos absolutos;
- rejeição de drive paths e UNC;
- prevenção de traversal;
- canonicalização;
- containment dentro do workspace;
- rejeição de symlink/junction escape;
- validação UTF-8;
- limite de leitura de 1 MiB;
- suporte a ranges;
- distinção tipada para diretório em `ReadFileError::Directory`;
- listagem não recursiva;
- preservação do path solicitado em `list_directory`;
- limites e proteção de `fetch_url`;
- limite de conteúdo enviado ao modelo para resultados de tools.

Orchestration:

- `MAX_TOOL_ROUNDS = 4`;
- `MAX_TOOL_CALLS = 16`;
- chamadas sequenciais;
- mensagens internas de `Assistant(tool_calls)` e `Tool`;
- erros controlados enviados ao provider como resultado de tool;
- erros de infraestrutura separados de erros controlados;
- stale guard e cancelamento entre etapas;
- síntese final sem tools após o orçamento de rounds;
- ToolCall inesperado durante síntese resulta em comportamento controlado de limite/protocolo.

Diagnósticos:

- probes temporários `CHAT_STREAM` foram removidos;
- diagnóstico de resposta HTTP não-2xx do Ollama permanece limitado a 16 KiB;
- truncamento UTF-8 seguro;
- diagnostics não registram API keys, headers nem payloads completos;
- request-shape diagnostics estruturais podem registrar papéis, número de tool calls, presença de IDs e tamanho de conteúdo, sem registrar o conteúdo completo.

Achados importantes:

- `list_directory("src")` preserva `src` e retorna `App`.
- `list_directory("src/App")` retorna `FileStone.php`.
- A distinção entre `App/` na raiz e `src/App/` está preservada.
- O caso real em que o modelo não encontrou `src/App/FileStone.php` foi diagnosticado como problema de planejamento/navegação do modelo, não como perda do path no pipeline da ferramenta.
- Execução de tools ocorre no background executor.
- Nenhuma leitura de filesystem foi adicionada ao typing, render, completion, cursor ou seleção.

Histórico persistente do Chat:

- conversas são armazenadas separadamente das configurações de UI/provider;
- criação, abertura, continuação, rename, delete e recuperação após restart foram implementados;
- ordenação por recência foi implementada;
- conversas são isoladas no contexto enviado ao provider;
- chat vazio não deve reaparecer após restart;
- o teste manual HISTORY-4 foi solicitado e posteriormente desfeito; não deve ser tratado como estado persistente válido do usuário.

---

### M.5-A — Agent Runtime Architecture Audit

Concluído como auditoria arquitetural.

Conclusão:

- Agent Runtime deveria ser headless.
- Chat e Agent deveriam permanecer independentes.
- Tool execution deveria usar adapters.
- Lifecycle, cancellation, budget e stale guard deveriam existir fora da UI.
- Não criar runtime de mutação ou permissões antes das fases correspondentes.

---

### M.5-B — Headless Agent Runtime Foundation

Concluído.

Crate:

- `crates/axiom-agent`

Tipos principais:

- `AgentRunId`
- `AgentState`
- `InvalidTransition`
- `AgentBudget`
- `AgentUsage`
- `BudgetResource`
- `BudgetExceeded`
- `Cancellation`
- `AgentEvent`
- `AgentRun`
- `is_stale_event`

Características:

- IDs monotônicos;
- cancelamento compartilhado por `Arc<AtomicBool>`;
- transições explícitas;
- budget de rounds e tool calls;
- eventos lifecycle;
- stale event detection;
- sem dependência de GPUI;
- sem acesso a filesystem, provider concreto ou UI.

Testes:

- 17 testes do crate `axiom-agent` passaram.

---

### M.5-C — Provider/Tool Execution Adapter

Concluído.

Contratos principais em `axiom-agent`:

- `ProviderExecutor`
- `ToolExecutor`
- `ToolOutcome`
- `AgentExecutionError`
- `AgentExecutionResult`

Comportamento:

- provider envia eventos incrementais;
- ThinkingDelta e ContentDelta são repassados durante a execução;
- tool calls são executadas sequencialmente;
- mensagens de protocolo permanecem internas;
- cancelamento é verificado antes de provider, tool e novo round;
- não existe síntese final escondida no runtime headless;
- erros controlados e erros de infraestrutura permanecem distintos.

---

### M.5-D — Minimal Agent UI Bridge e Adapters

Concluído em implementação, com a última polida D3 ainda pendente de confirmação manual.

Arquivos principais:

- `crates/axiom-app/src/ai/agent_bridge.rs`
- `crates/axiom-app/src/ai/mod.rs`
- `crates/axiom-app/src/workspace_view.rs`
- `crates/axiom-app/Cargo.toml`
- `Cargo.lock`

Adapters:

- `AgentProviderAdapter` encapsula o Ollama existente.
- `AgentToolAdapter` encapsula o `ToolRegistry`.
- Definições de tools são reutilizadas das helpers do Chat.
- Apenas `read_file`, `list_directory` e `fetch_url` ficam disponíveis.
- Erros controlados viram `ToolOutcome::ControlledError`.
- Calls malformadas são tratadas como erros controlados.
- Tool results crus não são exibidos diretamente na UI.

Ponte UI:

- execução ocorre no background executor;
- eventos são colocados em fila thread-safe;
- `poll_agent_events()` drena a fila no ciclo GPUI já existente;
- apenas eventos com o `agent_run_id` ativo são aceitos;
- eventos atrasados são descartados;
- Stop invalida a identidade da execução ativa;
- eventos tardios não conseguem alterar outra execução.

Estados de UI:

- `Idle`
- `Running`
- `Completed`
- `Cancelled`
- `Failed`

Sessão Agent:

- mensagens são representadas por `AgentUiMessage`;
- cada execução cria uma mensagem User e uma Assistant;
- execuções posteriores são anexadas abaixo das anteriores;
- mensagens anteriores não são substituídas;
- cada Assistant possui `run_id`;
- eventos atualizam somente a Assistant correspondente;
- provider context é reconstruído apenas com User/Assistant visíveis;
- tool protocol, tool results e Thinking antigo são excluídos do contexto visual enviado;
- Stop preserva conteúdo parcial;
- nova execução recebe novo `AgentRunId`.

UI:

- tab Chat e tab Agent são separadas;
- Agent possui scroll handle próprio;
- auto-follow só ocorre quando o viewport está próximo do fim;
- Markdown e code blocks reutilizam o renderer do Chat;
- copy de resposta possui notice próprio;
- status lifecycle é resumido para o usuário;
- não há exposição de ToolResult bruto.

Loading indicator:

- `Generating…` aparece imediatamente quando `AgentUiState::Running`;
- permanece durante Thinking, Content e tool execution;
- desaparece em Completed, Cancelled ou Failed;
- eventos stale não conseguem reativá-lo.

Correção D3 de troca Chat/Agent:

- a ordem dos dados já estava correta;
- a inserção era User seguida de Assistant;
- IDs de Assistant eram únicos;
- eventos carregavam `request_id` e `assistant_id`;
- a causa visual provável era o estado/âncora do `ScrollHandle` ao trocar de modo;
- foi adicionado `switch_ai_panel_mode()`;
- ao entrar no Chat:
  - scroll para o fim;
  - `chat_auto_follow = true`;
  - observação de scroll resetada;
  - scroll programático marcado como pendente;
- ao entrar no Agent, o handle do Agent vai para o fim;
- handlers dos tabs usam o método centralizado;
- inserção do Chat foi centralizada em `append_chat_turn`.

A causa do posicionamento visual foi inferida a partir do código e da ausência de reset de viewport. Ainda precisa de confirmação manual após a última alteração.

---

## 4. Arquivos e módulos importantes

### `crates/axiom-app/src/workspace_view.rs`

Responsável por:

- `WorkspaceView`;
- Chat e Agent tabs;
- seleção de provider/modelo;
- Chat history;
- Composer;
- Thinking;
- scroll;
- Chat orchestration;
- Agent session;
- Agent event polling;
- stale guards;
- copy actions;
- renderização de mensagens;
- transições Chat/Agent.

É o arquivo mais sensível para qualquer mudança futura.

### `crates/axiom-app/src/ai/tools/mod.rs`

Responsável por:

- tipos de tool;
- registry;
- metadata;
- tool request/result;
- classificação read-only.

Não adicionar mutating tools neste ponto.

### `crates/axiom-app/src/ai/tool_orchestration.rs`

Responsável por:

- orchestration multi-round do Chat;
- tool budget;
- provider-local messages;
- final synthesis;
- cancelamento;
- stale validation.

Não migrar automaticamente essa lógica para `AgentExecutor`.

### `crates/axiom-app/src/ai/agent_bridge.rs`

Responsável por:

- adaptar Ollama ao `axiom-agent`;
- adaptar `ToolRegistry` ao `ToolExecutor`;
- converter erros controlados;
- executar Agent no background;
- gerar IDs de run;
- reaproveitar definições de tool do Chat.

### `crates/axiom-agent/src/lib.rs`

Responsável por:

- lifecycle headless;
- budget;
- cancellation;
- provider/tool executor contracts;
- Agent events;
- execução sequencial.

Não deve importar GPUI nem conhecer implementação concreta de provider/tool.

### `crates/axiom-project/src/project_read.rs`

Responsável por:

- leitura segura de arquivos;
- paths relativos;
- containment;
- canonicalização;
- limites;
- ranges;
- erro `Directory`.

### Capability de diretório em `axiom-project`

Responsável por:

- listar entradas imediatas;
- preservar o path solicitado;
- distinguir diretórios homônimos em níveis diferentes;
- manter as proteções de workspace.

### `crates/axiom-ai-provider`

Responsável por:

- contratos do provider;
- Ollama;
- requests de chat;
- streaming;
- ThinkingDelta;
- ContentDelta;
- ToolCall;
- Done;
- erros de provider;
- diagnostics HTTP limitados.

### `crates/axiom-web`

Responsável por:

- `FetchUrlCapability`;
- limites de rede e conteúdo;
- adapter de `fetch_url`.

---

## 5. Decisões importantes

### Chat e Agent permanecem separados

O Chat possui histórico persistente e orchestration própria. O Agent possui sessão em memória independente e runtime headless próprio.

Motivo:

- evitar contaminar o Chat existente;
- preservar compatibilidade;
- permitir lifecycle e budget diferentes;
- separar contexto visual de protocolo interno.

### Tool protocol não é mensagem visual

Assistant tool calls e Tool results são necessários para o provider, mas não entram no histórico visível do usuário.

Motivo:

- evitar poluição da conversa;
- evitar reenvio incorreto de protocolo;
- manter contexto visual compreensível.

### Reutilização de tool definitions

O Agent reutiliza as definições existentes do Chat em vez de duplicar schemas.

Motivo:

- evitar divergência entre os caminhos;
- manter compatibilidade com o registry atual.

### Final synthesis após orçamento

Após o quarto round de tools, é feita uma request adicional sem tools para permitir resposta textual final.

Motivo:

- preservar a possibilidade de síntese;
- evitar aumentar `MAX_TOOL_ROUNDS`;
- impedir novas execuções de tool após o orçamento.

### Stale guards por identidade

Cada request Chat e cada run Agent possui identidade própria.

Motivo:

- respostas atrasadas não podem modificar uma nova execução;
- Stop deve invalidar imediatamente a execução anterior;
- Chat e Agent não devem compartilhar IDs.

### Agent não herda automaticamente ContextSnapshot

Decisão atual:

- o Agent usa apenas a sessão visível própria;
- o contexto explícito do chip ainda não está ligado ao Agent.

Motivo:

- evitar acoplamento implícito;
- preservar escopo mínimo de M.5-D.

---

## 6. Bugs e regressões encontrados

### Chat/Agent parecia mostrar mensagens fora de ordem

Auditoria do código confirmou:

- vetor de mensagens estava na ordem correta;
- inserção era User seguida de Assistant;
- IDs eram únicos;
- eventos tinham identidade correta;
- persistência preservava a ordem.

Causa visual provável:

- `ScrollHandle` mantinha viewport/âncora antiga ao trocar entre Chat e Agent.

Correção:

- método `switch_ai_panel_mode()`;
- reset de scroll ao entrar em cada modo;
- auto-follow e flags de observação reinicializados.

Status:

- correção implementada;
- validação manual após D3 ainda pendente.

### Agent substituía a interação anterior

Causa:

- somente `agent_content` e `agent_thinking` eram armazenados;
- cada Send apagava a interação anterior.

Correção:

- `Vec<AgentUiMessage>`;
- User e Assistant persistentes em memória;
- atualização por `run_id`;
- contexto reconstruído em ordem.

Status:

- corrigido;
- testes headless adicionados.

### Loading indicator ausente

Causa:

- UI só mostrava conteúdo depois do primeiro evento de Thinking/Content.

Correção:

- `Generating…` baseado no estado `Running`.

Status:

- corrigido em D3;
- validação manual ainda pendente.

### `read_file` em diretório

Causa anterior:

- diretório chegava ao `fs::read` como erro de I/O genérico.

Correção:

- `ReadFileError::Directory`.

Status:

- corrigido e testado.

### Navegação incorreta de `src/App`

Causa observada:

- o modelo não continuou a navegação até `src/App`.

Não foi encontrada perda de caminho no pipeline da ferramenta.

Status:

- ferramenta e serialização preservam `src` e `src/App`;
- problema atribuído ao planejamento do modelo;
- não implementar busca recursiva ou busca global como correção prematura.

### Erro antigo de tool aparecendo após nova mensagem

Foi investigado como possível vazamento de lifecycle/stale state.

Proteções relevantes:

- identidade de request;
- validação de evento stale;
- isolamento de estado de orchestration;
- separação de estado de provider;
- nenhuma mutação global de disponibilidade por erro antigo.

O código atual deve continuar descartando eventos de requests anteriores.

### Overflow, Markdown, seleção e scroll

Durante fases anteriores foram corrigidos ou polidos:

- overflow horizontal do composer;
- paste multiline que causava panic de newline no GPUI;
- wrapping Markdown;
- strong e inline code;
- copy de resposta e code block;
- botão Scroll to latest;
- mouse wheel;
- indicador de geração;
- cor e animação do indicador;
- ações de copy de mensagens User/Assistant;
- popup/notice de cópia.

A seleção de texto multiline foi tentada, apresentou regressões visuais e foi explicitamente adiada pelo usuário. Não reabrir essa frente sem solicitação específica.

---

## 7. Testes automatizados

Resultados conhecidos:

- `cargo fmt --all -- --check`: passou nas últimas validações.
- `cargo check --workspace --locked`: passou.
- `cargo check -p axiom-app --locked`: passou.
- `cargo test -p axiom-agent --locked`: 17 testes passaram.
- `cargo test -p axiom-project --locked`: 24 testes passaram.
- `cargo test -p axiom-web --locked`: 8 testes passaram.
- `cargo test -p axiom-ai-provider --locked`: 17 testes passaram.
- `cargo test -p axiom-app --bin axiom --locked`: 227 testes passaram após M.5-D3.

Testes adicionados ou existentes relevantes:

- lifecycle e transições do Agent;
- cancellation;
- stale events;
- budget;
- provider/tool adapters;
- erros controlados de tools;
- sessão Agent com múltiplas mensagens;
- preservação de ordem User/Assistant;
- Assistant ownership por `run_id`;
- contexto do Agent sem tool protocol;
- inserção Chat com IDs únicos;
- loading indicator por estado;
- final synthesis determinística do Chat;
- tool round limit;
- list_directory em `src` e `src/App`;
- erro tipado para diretório;
- segurança de paths;
- limites de conteúdo;
- histórico persistente do Chat.

Falha conhecida:

```text
crates/axiom-php/src/lib.rs
tests::indexes_runtime_symbols_and_signatures
left: 10
right: 9
```

Essa falha é conhecida e não foi causada pelas mudanças de Chat/Agent. Nenhum código PHP foi alterado para corrigi-la.

---

## 8. Testes manuais já realizados

Confirmados pelo usuário ou por validações anteriores:

- Chat real com Ollama;
- streaming incremental;
- Thinking ON/OFF;
- Markdown;
- fenced code blocks;
- copy de resposta;
- copy de código;
- copy de mensagens User/Assistant;
- composer;
- Enter para envio;
- acentos e IME/dead keys;
- mouse wheel;
- botão Scroll to latest;
- histórico de Chat;
- rename/delete/restart de Chat;
- isolamento entre conversas;
- Chat com `read_file`;
- Chat com `list_directory`;
- Chat com `fetch_url`;
- Agent simples;
- Agent com tools read-only;
- Stop Generation;
- nova execução após Stop;
- preservação parcial após Stop;
- lifecycle básico do Agent.

M.5-D2 foi manualmente validado pelo usuário quanto ao runtime/lifecycle, depois foi identificado e corrigido o problema de substituição de mensagens.

---

## 9. Testes manuais pendentes

Após M.5-D3, ainda devem ser executados manualmente:

### Chat → Agent → Chat

1. Abrir uma conversa Chat já existente.
2. Alternar para Agent.
3. Voltar para Chat.
4. Enviar `bom dia 1`.
5. Enviar `bom dia 2`.
6. Confirmar que cada turn aparece como:

```text
User
Assistant
User
Assistant
```

sem deslocamento visual incorreto.

### Loading do Agent

1. Abrir Agent.
2. Desativar Thinking.
3. Enviar:

```text
Responda somente: LOADING OK
```

4. Confirmar que `Generating…` aparece imediatamente.
5. Confirmar que desaparece ao terminar.

### Tool run

1. Ativar Thinking.
2. Enviar uma pergunta que use uma tool read-only.
3. Confirmar que o loading permanece durante a execução.
4. Confirmar que o status não expõe ToolResult bruto.

### Stop

1. Iniciar uma execução longa/multi-tool.
2. Clicar Stop.
3. Confirmar:
   - loading desaparece;
   - conteúdo parcial permanece;
   - nenhum evento tardio altera a mensagem;
   - nova execução funciona normalmente.

### Isolamento

1. Alternar várias vezes entre Chat e Agent.
2. Confirmar que mensagens Chat não aparecem no Agent.
3. Confirmar que mensagens Agent não aparecem no Chat.
4. Confirmar que cada viewport permanece coerente.

Não afirmar que esses testes passaram até serem executados em desktop.

---

## 10. Limitações e dívida técnica

- Agent não possui histórico persistente.
- Agent não possui restart restore.
- Agent não possui New Agent Session persistente.
- Agent não possui rename/delete/reopen.
- Agent não usa o schema de Chat History.
- Ainda não existe compaction de contexto.
- Sessões longas podem crescer até limites normais do provider.
- Thinking antigo é exibido na UI, mas não entra automaticamente no próximo contexto do Agent.
- `AgentEvent::ToolRequested` e `ToolStarted` ainda possuem identificação limitada; a UI mostra status genérico.
- O adapter de produção do Agent é atualmente Ollama-specific.
- Não existe ToolPolicy.
- Não existe permission gate.
- Não existem approval flows.
- Não existem ferramentas mutantes.
- Não existe shell/terminal/process execution para Agent.
- Não existe TraceStore.
- Não existem evals estruturadas.
- Não existe MemoryService.
- Não existem embeddings, MCP de memória ou integração com `ai-memory.exe`.
- Não existe busca global de arquivos.
- Não existe navegação recursiva automática.
- A falha conhecida de `axiom-php` permanece.
- O diagnóstico visual de scroll após troca Chat/Agent ainda precisa de confirmação manual.

---

## 11. Trabalho explicitamente adiado

Não implementar antecipadamente:

- `ToolPolicy`;
- permissões;
- approvals;
- write/edit/delete tools;
- shell;
- terminal;
- processos;
- mutation tools;
- Agent persistence;
- Agent history UI;
- context compaction;
- planner;
- browser automation;
- web search;
- MCP;
- MemoryService;
- embeddings;
- vector store;
- secret store;
- novo `axiom-ai-core` apenas por pureza arquitetural;
- busca recursiva ou busca global para corrigir comportamento de planejamento do modelo;
- streaming alternativo;
- runtime Tokio;
- polling/timers novos para resolver lifecycle;
- renderer paralelo para seleção de texto;
- alteração do pipeline do editor.

---

## 12. Estado atual exato

Estado funcional atual:

- Chat com Ollama real está implementado.
- Chat possui streaming e Thinking separados.
- Chat possui Markdown e code blocks.
- Chat possui tools read-only.
- Chat possui orchestration multi-round limitada.
- Chat possui histórico persistente.
- Agent possui runtime headless.
- Agent possui bridge GPUI.
- Agent possui adapters de provider e tools.
- Agent possui lifecycle, cancellation, budget e stale guards.
- Agent possui sessão em memória com múltiplos turns.
- Agent possui loading indicator.
- Agent reutiliza renderer Markdown do Chat.
- Agent ainda não possui persistência.
- A última alteração relevante foi M.5-D3 em `workspace_view.rs`.
- Não houve commit ou push nesta sessão.
- Nenhum arquivo foi alterado durante a solicitação deste handoff.

A implementação M.5-D está concluída em código, mas a aprovação completa depende da validação manual final do comportamento visual após troca Chat/Agent.

---

## 13. Próximo milestone planejado

O próximo milestone é **M.5-E — ToolPolicy e boundary de permissões para Agent**.

Objetivo provável:

- definir política explícita para tools;
- separar tool disponível, tool permitida e tool executável;
- preparar futuras aprovações;
- manter as tools atuais read-only;
- evitar que o Agent ganhe mutações implicitamente;
- fornecer contratos headless que possam ser usados pela UI depois.

A próxima fase não deve implementar ainda:

- write/edit/delete;
- shell;
- terminal;
- processos;
- permissões mutantes completas;
- Memory;
- persistência do Agent;
- novo runtime externo.

Antes de começar M.5-E, é recomendável finalizar a validação manual do M.5-D3.

---

## 14. Restrições para o próximo agente

O próximo agente deve:

- preservar `switch_ai_panel_mode()`;
- preservar `append_chat_turn`;
- preservar `poll_agent_events()`;
- preservar a validação por `agent_run_id`;
- preservar o comportamento de Stop;
- preservar a separação Chat/Agent;
- preservar a execução em background;
- preservar o reuse das tool definitions;
- preservar o limite read-only atual;
- executar testes existentes antes de refatorar;
- tratar a falha de `axiom-php` como conhecida até investigação separada;
- distinguir erro de dados de erro visual de scroll;
- não afirmar aprovação manual sem teste real.

O próximo agente não deve:

- migrar Chat para `AgentExecutor`;
- criar ToolPolicy dentro de `axiom-project`;
- adicionar mutating tools;
- introduzir `axiom-ai-core` sem necessidade concreta;
- criar runtime Tokio;
- usar polling adicional;
- capturar `AsyncApp` em task Send;
- bloquear a UI;
- alterar ContextSnapshot apenas para conectar Agent sem uma fase específica;
- persistir Agent dentro do histórico Chat sem contrato próprio;
- implementar Memory;
- corrigir o erro do `axiom-php` incidentalmente;
- remover stale guards;
- remover o loading indicator;
- trocar o mecanismo de scroll validado;
- reabrir a seleção de texto multiline sem escopo explícito.

---

## 15. Detalhes que seriam perdidos sem esta sessão

- O histórico persistente existente é do Chat, não do Agent.
- A sessão do Agent foi intencionalmente mantida em memória em M.5-D2.
- O Agent não deve reenviar tool protocol interno como contexto visível.
- O Agent não herda automaticamente o contexto explícito do chip.
- O problema visual após Chat → Agent → Chat não foi causado por ordem errada dos vetores; a hipótese atual é âncora/viewport do `ScrollHandle`.
- O loading precisa aparecer antes do primeiro evento do provider.
- Stop deve invalidar a identidade do run, não apenas marcar um estado visual.
- A ferramenta `list_directory` já preserva paths como `src/App`.
- O caso de `FileStone.php` foi atribuído ao planejamento do modelo.
- O caso de `read_file("App")` agora produz erro tipado de diretório.
- A síntese final após o limite de tools é uma request sem tools.
- O diagnóstico HTTP do Ollama foi limitado para não vazar payloads ou secrets.
- A suíte do workspace não está totalmente verde por causa de uma falha conhecida em `axiom-php`, não relacionada ao Agent.
- A seleção de texto multiline foi deliberadamente adiada após regressão visual.
- A sequência de roadmap deve ser baseada no source atual e em `AXIOM_M10_PERSISTENT_MEMORY_ROADMAP.md`, não em decisões históricas de ADRs antigos.

## NEXT SESSION START HERE

1. Ler `crates/axiom-app/src/workspace_view.rs`, principalmente:
   - `switch_ai_panel_mode`;
   - `append_chat_turn`;
   - `poll_agent_events`;
   - `send_agent_message`;
   - renderização do Agent.
2. Ler `crates/axiom-app/src/ai/agent_bridge.rs`.
3. Executar:

```text
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test -p axiom-agent --locked
cargo test -p axiom-app --bin axiom --locked
```

4. Realizar os testes manuais de M.5-D3:
   - troca Chat/Agent/Chat;
   - loading imediato;
   - tool run;
   - Stop;
   - isolamento.
5. Só depois registrar M.5-D como validado.
6. Em seguida iniciar o desenho mínimo de M.5-E para ToolPolicy e permissões read-only, sem implementar mutações, Memory ou persistência do Agent.