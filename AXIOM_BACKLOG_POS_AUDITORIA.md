# Axiom — Backlog técnico pós-auditoria

Este documento consolida somente os achados que continuam relevantes após as auditorias recentes e as validações adversariais feitas sobre o estado atual do projeto.

Objetivo: servir como backlog seguro para o agente trabalhar depois, em fases pequenas, evitando regressões de performance, concorrência e semântica.

> Regra principal: nenhum item abaixo deve ser tratado como autorização para uma refatoração ampla. Cada trabalho deve começar com uma auditoria/prova focada e terminar com a menor mudança possível.

---

## 1. Regras de segurança para qualquer fase futura

O Axiom já sofreu regressões graves de digitação e responsividade. Portanto:

- Não adicionar trabalho pesado à UI thread.
- Não adicionar `filesystem`, `canonicalize`, `metadata` ou directory walk em hot path.
- Não adicionar `document.content()` novo em typing, completion ou render.
- Não adicionar parse completo por tecla/render.
- Não adicionar scans O(project) ou O(vendor) novos na UI.
- Não trocar `try_read()` por `read()` bloqueante na UI.
- Não reintroduzir arquitetura complexa de background completion/generation sem evidência forte.
- Não misturar otimização, semântica e UI visual na mesma fase.
- Preferir snapshots/índices residentes e lookups in-memory.
- Diagnostics pesados devem continuar fora da UI thread.
- Não fazer commit/push sem autorização explícita.
- Evitar loops repetidos de build/teste; validar de forma focada durante implementação e fazer suíte ampla somente quando a fase estiver estável.

Validação preferida durante trabalho:
- `cargo test -p axiom-index <teste_focado>` quando aplicável.
- `cargo check -p axiom-app`.
- `cargo build -p axiom-app` apenas quando for necessário teste manual.
- `cargo build --release -p axiom-app --quiet` somente quando houver necessidade real de testar performance em release.

---

# 2. Achados confirmados que merecem trabalho futuro

## P1 — Duas materializações completas Rope → String por edição com completion

### Situação confirmada

Em uma edição que dispara autocomplete, o fluxo atual pode materializar o documento inteiro duas vezes na UI thread:

1. `trigger_completion`
   - chama `document.content()`;
   - usa a `String` para completion/prefix extraction.

2. `on_document_changed`
   - chama `document.content()`;
   - cria o `IndexUpdateRequest` enviado ao index worker.

O render não chama `document.content()`, o que é positivo.

### Risco

Em arquivos médios/grandes, duas materializações completas por tecla podem gerar:
- pressão de heap;
- cópia de MBs;
- micro-stutters;
- maior sensibilidade em typing contínuo.

### Direção segura

Não otimizar ainda sem uma fase própria.

A futura fase deve primeiro mapear exatamente:
- quais consumers realmente precisam da String inteira;
- se o completion precisa apenas de uma janela/prefixo;
- se existe conteúdo já materializado no mesmo fluxo que pode ser reutilizado;
- se é possível evitar uma das duas conversões sem mudar arquitetura.

### Restrições

Não:
- mover completion para background como “solução” automática;
- introduzir nova sincronização;
- criar cache global mutável de texto;
- adicionar locks;
- alterar Rope/Tree-sitter juntos na mesma fase.

### Critério de sucesso

Reduzir trabalho de Rope → String no hot path sem alterar comportamento de completion/indexação e sem piorar typing.

---

## P1 — Completion `new` e global fazem scans lineares na UI thread

### Situação confirmada

Os branches `->` e `::` já usam lookup semântico residente + `MemberResolver` e não são o principal problema.

Os branches que continuam potencialmente caros são:

- `new`
- completion global de classes/funções/constantes

Eles percorrem collections de:
- semantic snapshot;
- project symbols;
- vendor symbols;
- runtime symbols;

fazendo prefix filtering/dedup na UI thread.

Complexidade aproximada:
- `new`: O(snapshot + project + vendor + runtime)
- global: O(classes + functions + constants de todas as fontes)

### Risco

Projetos Composer grandes podem ter dezenas de milhares de símbolos em `vendor`.

Isso pode causar latência perceptível justamente durante typing.

### Direção segura

Criar uma fase isolada de indexação de prefixo residente.

Investigar opções simples antes de qualquer arquitetura complexa:
- buckets pelo primeiro caractere;
- mapa ordenado/prefix range;
- índice de prefixos residente;
- estrutura compartilhada pronta no momento da indexação.

### Restrições

Não:
- adicionar parse;
- adicionar filesystem;
- adicionar worker de completion por padrão;
- alterar o branch `->` que já está residente e estável;
- construir novo índice a cada tecla.

### Critério de sucesso

`new` e global devem consultar uma estrutura residente de custo proporcional ao número de matches, e não ao tamanho total de Project/Vendor.

---

## P1 — Native inspections executam trabalho stale até o fim

### Situação confirmada

Pipeline atual:

`edit → debounce → capture → worker → result → generation/revision check → apply/discard`

O worker:
- faz parse do conteúdo;
- percorre AST;
- executa inspections;
- termina normalmente mesmo que o usuário já tenha digitado novamente.

Somente quando o resultado volta à UI ocorre o descarte por revision/generation stale.

### Risco

Em typing contínuo:
- CPU pode ser gasta em análise que nunca será aplicada;
- workers podem atrasar resultados válidos;
- debug pode mostrar latência muito maior que release;
- diagnósticos podem parecer intermitentes mesmo estando semanticamente corretos.

### Observação importante

Não confundir isso com scan O(project/vendor).

As inspeções auditadas usam majoritariamente:
- AST traversal;
- lookups O(1) no snapshot;
- resolução de herança/membros.

### Direção segura

Futura fase deve medir e reduzir trabalho desperdiçado.

Possíveis estratégias a investigar, em ordem de simplicidade:
1. checagem de geração em pontos naturais antes das inspeções caras;
2. coalescing mais agressivo antes de iniciar worker;
3. impedir que jobs stale ainda não iniciados executem;
4. somente depois considerar cancelamento cooperativo interno.

### Restrições

Não:
- adicionar locks na UI;
- bloquear thread esperando worker;
- criar pool/arquitetura complexa sem prova;
- otimizar ao mesmo tempo que type diagnostics estiver sendo implementado.

### Critério de sucesso

Typing contínuo não deve provocar execução completa repetida de inspections que já nasceram stale.

---

## P2 — Custo das native inspections deve ser medido por regra

As regras atuais devem ser tratadas separadamente:

### UnknownClass
- percorre nós relevantes da AST;
- lookup semântico O(1).

### UnknownConstant
- percorre AST;
- lookup semântico O(1).

### DuplicateClass
- percorre declarações de classe;
- lookup no snapshot.

### Argument inspections
- percorrem chamadas;
- resolvem callable;
- inferem tipos dos argumentos;
- percorrem herança quando necessário.

### Próxima auditoria recomendada

Medir tempo por inspection no mesmo worker:
- parse;
- unknown class;
- unknown constant;
- duplicate class;
- argument arity;
- argument type.

Não deixar probes permanentes depois da investigação.

---

# 3. Áreas confirmadas como corretas ou já corrigidas

Os itens abaixo NÃO devem voltar para o backlog como bugs sem uma nova reprodução.

## Snapshot incremental não mantém símbolos antigos do arquivo

`replace_workspace_file(...)` já remove, para o `FileId` substituído:
- classes antigas;
- funções antigas;
- constantes antigas;
- relações antigas de hierarchy;
- trait uses antigos;

antes da nova extração.

Não implementar limpeza duplicada sem um teste que prove um novo stale state.

---

## Constructor herdado em múltiplos níveis

A resolução atual percorre a hierarquia com cycle guard.

Cenário:

`GrandParent::__construct → Parent → Child`

já é suportado semanticamente.

Não criar fallback especial para profundidade > 1 sem reprodução.

---

## Trait cycles já possuem proteção

Os resolvers relevantes possuem `HashSet`/visited guards.

Não adicionar novos guards ou refatorar recursão sem teste que demonstre ciclo ainda não coberto.

---

## Cache já possui versionamento explícito

O cache possui:
- magic header;
- `CACHE_VERSION`;
- validação antes de deserialize/uso.

Não abrir tarefa “adicionar versionamento de cache” sem evidência de outro problema de compatibilidade.

---

## Diagnostics iniciais ao abrir arquivo

O estado atual já agenda native inspections no lifecycle de abertura.

Isso foi validado manualmente: fechar/reabrir arquivo sem editar volta a exibir diagnostics.

Não duplicar scheduling no open/focus sem medir risco de execução repetida.

---

## Range/EOF de scope

Não aplicar genericamente a sugestão “tornar ranges inclusivos”.

O código atual já possui tratamento de boundary/fallback relevante.

Qualquer mudança futura deve provar:
- caso exato que falha;
- FileId;
- ScopeId;
- offset;
- risco de overlap com scopes adjacentes.

---

# 4. Path identity — acompanhar, mas não tratar como bug aberto comprovado

A primeira auditoria levantou WSL/UNC/path identity como bug crítico.

As validações posteriores mostraram que:
- paths lexicalmente equivalentes são normalizados;
- `foo/../bar/file.php` e `bar/file.php` podem produzir a mesma identidade;
- o problema observado anteriormente em completion não era simplesmente “FileId ausente”.

Mesmo assim, path identity continua uma área sensível porque o Axiom roda em ambientes Windows/WSL.

## Futura fase recomendada

Somente testes, sem filesystem, para:
- separadores `\` vs `/`;
- `.` / `..`;
- drive-letter normalization quando aplicável;
- UNC lexical forms;
- workspace-relative identity.

Não inventar equivalência `/mnt/c/... ↔ C:\...` se ela não for parte do contrato atual.

---

# 5. Type diagnostics — concluir antes de otimizações

A infraestrutura semântica atual já possui:

- `SemanticParameter`;
- `SemanticSymbol::structured_parameters`;
- `DeclaredType`;
- `TypeCompatibility`;
- `declared_type_compatibility(...)`;
- `Expression::Literal`;
- inferência de:
  - int;
  - float;
  - string;
  - true/false;
  - null;
  - array;
- resolução de bindings/classes/membros.

## Próximo trabalho funcional prioritário

Integrar type checking de argumentos no pipeline EXISTENTE de native inspections.

Fluxo desejado:

`argument AST`
→ `infer_expression_type`
→ `SemanticParameter.declared_type`
→ `declared_type_compatibility`
→ diagnostic somente quando `Incompatible`

### Regras

- `Compatible` → nenhum diagnostic.
- `Unknown` → nenhum diagnostic.
- `Incompatible` → diagnostic.
- Sem type hint → não validar.
- `mixed` → aceita tudo.
- union → aceita qualquer branch compatível.
- nullable → aceita `null` ou tipo base.
- classe filha → compatível com classe pai.
- actual unknown → não gerar falso positivo.
- variadic tipado → aplicar tipo aos argumentos extras.
- named args/unpacking → ignorar conservadoramente se não houver mapping seguro.

### Range

Type mismatch deve marcar somente o argumento:

```php
new Child(22);
          ^^

$teste->testeParameter(32, 'nome');
                       ^^  ^^^^^^
```

Arity continua usando o range de arguments existente.

### Performance

A integração deve reutilizar:
- AST já parseada;
- snapshot já capturado;
- scope/FileId já existentes;
- callable resolution já existente.

Não adicionar:
- parse;
- filesystem;
- canonicalize;
- `document.content()`;
- project/vendor scan;
- lock;
- trabalho na UI.

---

# 6. Debug/probe debt

Ainda existem probes temporários relacionados às investigações:

- `[COMP FLOW]`
- `[SEM COMP]`
- `[SEM PUB]`
- `[NATIVE INSPECT]`
- `[NATIVE CALL]`
- `[NATIVE TYPE]`, se for adicionado na fase de tipos

Eles devem permanecer apenas enquanto forem úteis para investigação.

## Fase futura de cleanup

Depois que:
- type diagnostics estiverem funcionando;
- lifecycle estiver estável;
- performance manual em release estiver validada;

remover todos os probes temporários e helpers de debug associados.

Não remover antes, porque ainda podem ser úteis para validar o pipeline final.

---

# 7. Ordem recomendada de trabalho

## Fase A — Finalizar argument type diagnostics
Objetivo funcional atual.

Validar manualmente:
- constructor;
- member method;
- inherited constructor;
- inherited method;
- union;
- nullable;
- mixed;
- unknown;
- variadic.

---

## Fase B — Cleanup de probes
Somente depois da fase A estar estável.

---

## Fase C — Hot path: `document.content()`
Auditoria focada + menor redução possível de Rope → String.

Não misturar com completion indexing.

---

## Fase D — Prefix index para `new` e global completion
Criar estrutura residente simples e medir.

Não alterar `->`/`::` sem necessidade.

---

## Fase E — Wasted native inspection work
Reduzir jobs stale mantendo worker/debounce simples.

---

## Fase F — Path identity regression suite
Somente testes primeiro.

---

# 8. Validação final de uma fase estável

Evitar rodar a suíte completa repetidamente durante desenvolvimento.

Quando uma fase estiver realmente estável:

```bash
cargo test -p axiom-index
cargo test -p axiom-app
cargo test -p axiom-app --bin axiom
cargo check -p axiom-app
cargo fmt --all -- --check
git diff --check
```

Se precisar validar performance/manual:

```bash
cargo build --release -p axiom-app --quiet
.\target\release\axiom.exe .
```

Garantir que o executável testado é realmente `target\release\axiom.exe`, não debug.

---

# 9. Cenários manuais de regressão importantes

## Completion
- inherited `->`;
- trait method;
- global assignment `$child = new Child(); $child->`;
- `new` prefix filtering;
- non-PHP (`README.md`, `.txt`) não deve receber completion PHP.

## Diagnostics
- constructor arity;
- inherited constructor arity;
- member arity;
- close/reopen sem edição;
- type mismatch;
- valid union sem erro;
- nullable sem falso positivo;
- mixed sem erro;
- unknown actual sem erro.

## Performance
- typing rápido em release;
- arquivo PHP grande;
- projeto Composer com `vendor`;
- completion `new`;
- completion global;
- typing contínuo enquanto native inspections rodam.

---

# 10. Regra para futuros agentes

Antes de implementar qualquer item deste backlog:

1. provar o estado atual do source;
2. identificar o caminho exato;
3. criar teste/reprodução quando possível;
4. fazer a menor alteração;
5. validar somente o necessário;
6. não misturar duas áreas de risco na mesma fase;
7. não assumir que uma recomendação antiga ainda é válida;
8. não transformar “risco potencial” em bug sem evidência.

O objetivo principal é evoluir o Axiom sem sacrificar fluidez de digitação e sem reintroduzir regressões de concorrência, semântica ou hot path.

---

# 11. Execução do backlog (2026-08-30)

Status das fases:

- [x] Fase A — argument type diagnostics corrigido e coberto para constructor, member method, herança, union, nullable, mixed, unknown, variadic e range exato do argumento.
- [x] Fase B — probes `[COMP FLOW]`, `[SEM COMP]`, `[SEM PUB]`, `[NATIVE INSPECT]`, `[NATIVE CALL]` e `[NATIVE TYPE]`, além dos helpers exclusivos, removidos.
- [x] Fase C — o snapshot de texto já criado em `after_edit` passou a ser reutilizado por trigger LSP, signature help e native completion.
- [x] Fase D — índices residentes de prefixo adicionados para Project, Vendor/Composer e PHP Runtime; atualização incremental e remoção cobertas por teste.
- [x] Fase E — jobs stale são descartados após o debounce e também cooperativamente entre as regras no worker por generation token sem lock.
- [x] Fase F — regressões lexicais cobertas para separadores, `.`/`..`, drive letter, UNC e não equivalência WSL-drive.

Validação automatizada concluída:

- `cargo test -p axiom-index` — 115 testes passaram; 2 audits long-running já marcados como ignored.
- `cargo test -p axiom-app` — 14 testes passaram.
- `cargo test -p axiom-app --bin axiom` — 63 testes passaram.
- `cargo check -p axiom-app` — passou.
- `cargo fmt --all -- --check` — passou.
- `git diff --check` — passou (somente avisos de normalização LF/CRLF do Git no Windows).
- `cargo build --release -p axiom-app --quiet` — passou.
- `target\release\axiom.exe .` — smoke test de inicialização passou e o processo foi encerrado de forma controlada.

Validação visual/manual ainda recomendada: interação prolongada de typing e completion em projeto Composer grande. O smoke test confirma inicialização do binário release, mas não substitui avaliação humana de fluidez da UI.
