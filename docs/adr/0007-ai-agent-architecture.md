# ADR 0007 — Axiom AI & Agent Platform

**Status:** Decisão de boundary para M.0.1; contratos e integrações futuros não implementados.
**Data:** 2026-09-14

## Contexto e auditoria do source

A IDE continua em desenvolvimento. AI será uma trilha paralela ao motor semântico.
Esta ADR segue a numeração existente em `docs/adr`; substitui apenas o nome
proposto `ADR_AI_AGENT_ARCHITECTURE.md`, não decisões de outros ADRs.

O `Cargo.toml` atual declara oito crates (o resumo antigo de seis em
`docs/ARCHITECTURE.md` está desatualizado):

| Crate | Responsabilidade e dependências locais |
| --- | --- |
| axiom-app | Composição GPUI; depende dos outros sete crates |
| axiom-editor | Document, buffer, seleção e histórico; floem-editor-core headless |
| axiom-project | Project, Composer e PSR-4; sem dependências locais |
| axiom-syntax | Parsing incremental Tree-sitter; sem UI |
| axiom-php | Símbolos de runtime/stubs; depende de axiom-syntax |
| axiom-index | Índices, identidade, referências e SemanticSnapshot; depende de axiom-php e axiom-syntax |
| axiom-lsp | Cliente genérico stdio, protocolo e codecs; sem dependências locais |
| axiom-terminal | PTY e emulação; sem dependências locais |

Os manifests de cada crate comprovam essas direções. Não existe um crate genérico
de shared types: os tipos pertencem ao domínio (`Document` em editor,
`SemanticSnapshot`/IDs em index, protocolo em lsp/lsp-types).
`WorkspaceView` em `crates/axiom-app/src/workspace_view.rs` mantém Project,
abas com entidades EditorView e engine semântica compartilhada. `EditorView`
possui Document, sintaxe, DiagnosticStore e estado de sessão/edição.
`SemanticEngine` em `crates/axiom-index/src/semantic.rs` publica
`Arc<SemanticSnapshot>` através de `RwLock`; `publish` rejeita revisões não
monotônicas, `publish_from` também valida a base. `try_snapshot` é não bloqueante.
AI não deverá receber acesso mutável a esses owners.

## Async, cancelamento e eventos existentes

`EditorView` e `WorkspaceView` usam `cx.spawn`, tarefas GPUI e executores de
background; indexação/inspections também usam `std::thread::spawn` e
`std::sync::mpsc`. `axiom-lsp/src/lib.rs` usa processo stdio, threads, requests
pendentes e canais; `axiom-app/src/lsp_bridge.rs` adapta resultados em
`IdeLspEvent`, inclusive usando `background_executor().spawn(...).detach()`.
Não há runtime Tokio explicitamente criado pelos crates locais. Dependências
transitivas não constituem um contrato de runtime da aplicação.

Existem padrões reutilizáveis, não uma abstração universal de cancelamento:
`document_session`, `edit_generation`, `native_inspection_generation`,
`AtomicU64` com latest-generation, gerações de projeto/semântica e de requests
LSP. O cancelamento cooperativo de inspections verifica geração entre regras;
isso não prova interrupção de uma operação de I/O já em execução.
Para AI, reutilizar conceitualmente request identity + session/base revision +
latest-wins e cancelamento cooperativo. Não reutilizar um contador de edição
como identidade de toda tarefa de agente.

`WorkspaceView::poll_semantic_updates` recebe resultados, publica a snapshot e
atualiza as abas; o refresh de inspections ocorre após publicação bem-sucedida.
Existem canais tipados e enums de eventos, mas não foi encontrado event bus
genérico nem stream AI. Os canais mpsc atuais e a fila de eventos LSP não são
um contrato de backpressure para streams de modelo. Futuramente definir canais
limitados, consumidores desconectáveis e descarte/coalescimento; não acrescentar
polling por token. ModelStarted/Delta/Finished/Failed e eventos Agent/Tool são
apenas exemplos conceituais nesta fase.

## Settings e secrets

`shell_state.rs` persiste `RecentProjects` via serde_json em
`ProjectDirs(...).config_dir()/recent-projects.json`; também define caminhos de
cache e stubs. `commands.rs::Keymap` persiste `keymap.json`. Há compatibilidade
com diretórios legados RustStorm. Não há serviço central de settings identificado.
Os identificadores de ProjectDirs diferem (`dev` em shell_state, `org` no keymap);
nesta fase não unificamos essas políticas.

Não foi encontrada implementação local de SecretStore, keyring ou Windows
Credential Manager nos sources/manifests auditados. Futuro armazenamento de
credenciais deve ser um adapter do sistema operacional, com referência opaca na
configuração comum, nunca API key em JSON, trace, prompt ou roadmap. Falha do
cofre deve deixar a integração indisponível, sem fallback plaintext. Seleção da
biblioteca, portabilidade, redaction e teste de disponibilidade ficam pendentes.

## Decisão de boundary

AI consumes Axiom intelligence. AI is never the source of truth for deterministic IDE state.

Índice, semântica, type engine, diagnostics, PHPDoc, Composer, filesystem, Git e
testes permanecem autoritativos. Estado atual determinístico prevalece sobre
memória histórica e inferência do modelo. A listagem inclui capacidades futuras;
não afirma que todas já possuem APIs prontas no Axiom.

Boundary futura aprovada conceitualmente:

```text
axiom-app (composição e adapters de UI/IDE)
  -> axiom-ai-provider (integrações de modelos, futuras)
       -> axiom-ai-core (contratos headless, futuros)
```

App pode consumir os contratos de core diretamente quando necessário. Core não
depende de app, GPUI, HTTP, SDK ou modelos concretos. Provider não depende de GPUI
nem acessa diretamente Document/WorkspaceView. Dados de contexto entram como
valores/snapshots explicitamente capturados pela aplicação. Não criar ciclos
fazendo index/editor dependerem de AI.

Os nomes seguem `axiom-*` e são adequados para separar contratos de adapters.
**Nenhum crate é criado em M.0.1:** não há consumidor ou comportamento de modelo
implementado para justificar dois skeletons vazios. ProviderId/ModelId/message/
capabilities serão definidos somente quando um contrato mínimo exigir esses
tipos. A boundary documental é a entrega desta fase, não um enforcement Cargo.

Alternativas: colocar tudo em app acoplaria contratos à UI; criar muitos crates
agora cristalizaria abstrações sem casos de uso. Adiamos ambos. Quando surgir o
primeiro contrato, core deve compilar isoladamente e a árvore de dependências
deve provar ausência de GPUI. Provider só nasce quando houver implementação útil.

Os manifests atuais não oferecem uma feature AI. Recomenda-se futuramente uma
dependência opcional de app controlada por feature `ai`, desativada por padrão,
mais opt-in em runtime. Não existem features globais de workspace implícitas:
a feature pertencerá a app. Isso permite builds sem adapters pesados e editor
funcional sem credenciais. Nesta fase não adicionamos flags nem dependências.

## Providers e runtime de agentes

ModelProvider envia requests, recebe resultados/streams e declara capabilities;
providers devem ser intercambiáveis. AgentRuntime coordena provider, contexto,
tools, permissões, traces, cancelamento e lifecycle; independe do modelo.
Não tratar chamada a modelo como execução de agente.

```text
Axiom AI: Context | Tools | Trace | ModelProvider
AgentRuntime: NativeAxiomRuntime | future external runtime adapters
```

O desenho é conceitual, sem implementação de trait ModelProvider, HTTP,
streaming, chat, tools, memory, MCP ou sub-agentes. Hermes e outros runtimes
externos serão adapters; não ditarão o núcleo nem o estado da IDE.

Agentes futuramente acessam a IDE por tools registradas. Tools mutantes exigem
permissões e validação da revisão/base no instante de aplicar a mudança.
Execuções terão traces observáveis com identidade, causalidade, resultado e
redaction; não registrar secrets nem capturar conteúdo indiscriminadamente.

Context é evidência capturada para uma operação; Memory é informação histórica;
Instructions são políticas/instruções de execução; Trace é o registro observado;
Skills são procedimentos versionados/revisados. Memory não modifica Skills
automaticamente. Learned Skills exigem validação e review antes da adoção.

## Performance e isolamento

AI fica fora do typing/render hot path. Proibidos novos document.content(), full
parse, canonicalização, directory walk, scans de projeto/vendor, HTTP/model call
bloqueante, escrita de memória ou embeddings por tecla e espera na thread GPUI.
Captura explícita de contexto deve ser limitada e revisionada. Comunicação futura
usa snapshots, eventos, comandos e canais async limitados; resultados atrasados
não podem sobrescrever estado atual. Operações de rede/modelo ficam fora da UI.

AI desativada ou provider indisponível não impede uso da IDE. Erros recuperáveis
devem virar resultados/eventos locais ao subsistema. **Limite atual:** o profile
release define `panic = "abort"`; panic numa thread pode abortar todo o editor.
Não prometer isolamento com catch_unwind/thread. Isolamento forte de runtime
externo exige boundary de processo ou alternativa validada numa fase própria.
Nenhum mecanismo de recovery ou alteração de profile é implementado aqui.

## Consequências e próxima microfase

M.0.1 muda somente esta documentação; não cria testes artificiais para crates
vazios. A validação do workspace deve ser registrada no relatório da execução.
Os documentos antigos são contexto, não prova de implementação.

M.0.2 recomendada: especificar contratos mínimos de request/result e identidade
de cancelamento/contexto, com exemplos de stale result e backpressure; decidir
a política de isolamento compatível com panic=abort e critérios do SecretStore.
Só então materializar axiom-ai-core headless com tipos efetivamente necessários
e testes de comportamento. Ainda sem ModelProvider, SDK, HTTP, UI ou runtime de
agentes; implementação de provider fica para M.1. Revisar esta ADR se esses
contratos indicarem outra divisão.
