# Axiom IDE — Roadmap de Funcionalidades Inspirado em IDEs PHP Modernas

## Objetivo

Evoluir o Axiom como uma IDE PHP moderna, rápida e focada em produtividade, sem tentar replicar integralmente o PhpStorm.

A ideia é priorizar funcionalidades que:

* aproveitem a infraestrutura semântica já existente;
* tragam alto ganho de produtividade;
* mantenham a UI simples;
* evitem regressões de performance;
* possam ser implementadas em etapas pequenas;
* reforcem o diferencial do Axiom em velocidade e experiência PHP.

O foco deve ser sempre na necessidade do desenvolvedor, e não em copiar visualmente ou funcionalmente outra IDE.

---

# Fase 1 — Fundamentos de editor moderno

Objetivo: tornar a edição cotidiana rápida e confortável.

## 1. Comment / Uncomment

Implementar atalhos para:

```text
Ctrl+/              comentar/descomentar linha
Ctrl+Shift+/        comentar/descomentar bloco
```

Suporte inicial:

* PHP;
* múltiplas linhas selecionadas;
* preservação correta de indentação;
* toggle previsível.

---

## 2. Duplicate Line / Selection

Comportamento:

```text
Ctrl+D
```

* com seleção → duplicar seleção;
* sem seleção → duplicar linha atual.

---

## 3. Delete Line

Atalho configurável para remover a linha atual sem exigir seleção.

Preservar:

* undo;
* cursor;
* newline;
* múltiplas linhas futuramente.

---

## 4. Move Line / Statement Up and Down

Atalhos para mover:

```text
Ctrl+Shift+↑
Ctrl+Shift+↓
```

Primeira implementação:

* linha atual;
* seleção de linhas.

Evolução:

* usar AST para mover statements inteiros de maneira sintaticamente segura.

---

## 5. Expand / Shrink Selection

Usar Tree-sitter para expandir seleção estruturalmente.

Exemplo:

```php
$user->getProfile()->getName();
```

Sequência possível:

```text
getName
→ getName()
→ $user->getProfile()->getName()
→ statement
→ bloco
→ método
→ classe
```

Também implementar posteriormente a operação inversa.

---

## 6. Recent Files

Popup simples mostrando arquivos recentemente acessados.

Desejável:

* navegação por teclado;
* filtro digitando;
* arquivos fechados recentemente;
* ordenação por recência.

---

## 7. File Structure

Popup da estrutura do arquivo atual.

Para PHP, mostrar:

* namespace;
* classes;
* interfaces;
* traits;
* enums;
* propriedades;
* métodos;
* funções;
* constantes.

Permitir:

* filtro por nome;
* Enter para navegar;
* símbolos agrupados por container.

---

## 8. Highlight Usages in File

Ao posicionar o cursor sobre um símbolo:

* destacar ocorrências semanticamente relacionadas no arquivo;
* permitir navegar entre elas;
* Escape remove os highlights.

Prioridade inicial:

* variáveis locais;
* parâmetros;
* propriedades;
* métodos.

---

# Fase 2 — Navegação PHP semântica

Objetivo: transformar o índice semântico do Axiom em uma experiência real de IDE.

## 9. Go to Declaration

Navegação para declaração de:

* classes;
* interfaces;
* traits;
* enums;
* métodos;
* funções globais;
* propriedades;
* constantes;
* variáveis locais;
* parâmetros.

Casos importantes:

* imports;
* aliases;
* namespaces;
* herança;
* traits;
* Vendor;
* Runtime stubs.

---

## 10. Go to Implementation

Para abstrações como:

```php
interface Repository
{
    public function save(Entity $entity): void;
}
```

Permitir descobrir implementações:

```text
MysqlRepository::save
RedisRepository::save
MemoryRepository::save
```

Suportar:

* interfaces;
* métodos abstratos;
* classes abstratas;
* eventualmente métodos sobrescritos.

---

## 11. Find Usages

Busca completa de referências no projeto.

Aplicável a:

* classe;
* interface;
* método;
* função;
* propriedade;
* constante;
* variável, quando apropriado.

Resultado ideal em tool window:

```text
Find Usages: UserService::save

src/Controller/UserController.php:42
src/Command/ImportUsers.php:81
tests/UserServiceTest.php:33
```

Requisitos:

* navegação rápida;
* agrupamento por arquivo;
* atualização eficiente;
* uso dos índices residentes sempre que possível.

---

## 12. Show Usages

Versão rápida de Find Usages.

Mostrar popup próximo ao editor:

```text
UserService::save — 4 usages

UserController.php:42
ImportUsers.php:81
UserServiceTest.php:33
...
```

Enter navega.

A ideia é responder rapidamente:

> Onde isso é usado?

---

## 13. Search Classes

Busca global de classes.

Exemplos:

```text
UserRep
OrderCont
HttpCl
```

Fontes:

* Project;
* Vendor;
* Runtime.

Reaproveitar os índices residentes de prefixo já existentes.

---

## 14. Search Symbols

Busca mais ampla por:

* classes;
* métodos;
* funções;
* propriedades;
* constantes.

Posteriormente combinar com uma experiência de Search Everywhere.

---

## 15. Type Hierarchy

Exibir herança de maneira simples.

Exemplo:

```text
AbstractRepository
└── DatabaseRepository
    ├── UserRepository
    └── OrderRepository
```

Também considerar:

```text
Interface
├── ImplementationA
└── ImplementationB
```

Não precisa começar com UML.

Um popup/tree é suficiente.

---

## 16. Quick Documentation

Mostrar documentação do símbolo sem sair do editor.

Conteúdo possível:

```text
UserRepository::find

public function find(int $id): ?User

@param int $id
@return User|null

vendor/acme/package/src/UserRepository.php
```

Fontes:

* assinatura;
* PHPDoc;
* declared types;
* Runtime stubs;
* Vendor;
* Project.

---

## 17. Quick Definition

Mostrar trecho da declaração em popup.

Exemplo:

```text
┌ UserService.php ───────────────────────┐
│ public function save(User $user): void │
│ {                                      │
│     ...                                │
│ }                                      │
└────────────────────────────────────────┘
```

Pode reutilizar boa parte do mecanismo de Go to Declaration.

---

# Fase 3 — Busca e navegação global

Objetivo: permitir acessar qualquer elemento importante rapidamente.

## 18. Search Everywhere

Busca unificada:

```text
Files
Classes
Symbols
Actions
Recent
```

Não precisa copiar o comportamento de double Shift.

O importante é ter uma interface única de descoberta.

---

## 19. Find Action

Busca por comandos da IDE.

Exemplo:

```text
reformat
rename
terminal
find usages
theme
```

Integrar diretamente ao command registry.

---

## 20. Navigate to File

Busca rápida de arquivos com suporte a:

```text
UserService.php
UserService.php:42
```

Futuramente:

```text
UserService.php:42:18
```

---

## 21. Speed Search em árvores

Ao focar:

* Project Tree;
* Find Usages;
* File Structure;
* Hierarchy;

começar a digitar deve filtrar ou selecionar itens.

---

# Fase 4 — Refactoring seguro

Objetivo: começar um mecanismo de refatoração semântica confiável.

## 22. Rename

Prioridade por nível de dificuldade:

### 22.1 Variável local

Primeiro alvo ideal.

```php
$userName
```

→

```php
$name
```

Somente referências pertencentes ao mesmo binding.

### 22.2 Parâmetro

Atualizar usos dentro do escopo correto.

### 22.3 Métodos privados

Mais controlado semanticamente.

### 22.4 Classe

Atualizar:

* usos;
* imports;
* type hints;
* extends;
* implements;
* `new`;
* static references.

### 22.5 Métodos públicos

Exige Find Usages extremamente confiável.

### 22.6 Propriedades e funções globais

Adicionar quando os índices estiverem maduros.

Sempre considerar preview:

```text
Rename UserService → AccountService

17 usages will be changed

[Apply] [Preview] [Cancel]
```

---

## 23. Extract Variable

Exemplo:

```php
send($user->getProfile()->getName());
```

selecionar:

```php
$user->getProfile()->getName()
```

resultado:

```php
$name = $user->getProfile()->getName();

send($name);
```

Usar AST para identificar expression válida.

---

## 24. Inline Variable

Transformar:

```php
$name = $user->name;

echo $name;
```

em:

```php
echo $user->name;
```

Somente quando semanticamente seguro.

---

## 25. Extract Constant

Exemplo:

```php
timeout(30);
```

→

```php
private const DEFAULT_TIMEOUT = 30;

timeout(self::DEFAULT_TIMEOUT);
```

Primeira versão pode atuar somente dentro da classe atual.

---

## 26. Surround With

Selecionar:

```php
process();
```

e oferecer:

```text
if
while
for
foreach
try/catch
function
```

Exemplo:

```php
try {
    process();
} catch (\Throwable $e) {
}
```

---

# Fase 5 — Code Generation

Objetivo: eliminar código repetitivo usando o semantic model.

## 27. Generate Menu

Ação contextual:

```text
Generate...
```

Opções:

```text
Constructor
Getter
Setter
Getter + Setter
Implement Methods
Override Methods
PHPDoc
```

---

## 28. Generate Constructor

A partir de propriedades:

```php
private UserRepository $repository;
private Logger $logger;
```

gerar:

```php
public function __construct(
    private UserRepository $repository,
    private Logger $logger,
) {
}
```

Respeitar versão/configuração PHP futuramente.

---

## 29. Getter / Setter

Gerar métodos a partir de propriedades.

Considerar:

* readonly;
* typed properties;
* nullable;
* fluent setters opcionalmente.

---

## 30. Implement Methods

Para:

```php
class Foo implements SomeInterface
{
}
```

mostrar métodos ausentes e permitir seleção.

---

## 31. Override Methods

Mostrar métodos herdados disponíveis para override.

Preservar:

* assinatura;
* tipos;
* visibilidade;
* `static`;
* return type.

---

# Fase 6 — Completion avançado

Objetivo: fazer completion deixar de ser apenas lookup e começar a entender intenção.

## 32. CamelCase Matching

Permitir:

```text
UR → UserRepository
HC → HttpClient
OCR → OrderCreateRequest
```

Sem remover o matching por prefixo atual.

---

## 33. Fuzzy Matching

Aceitar pequenas omissões:

```text
UsrRepo
```

→

```text
UserRepository
```

Deve ser ranking, não busca indiscriminada.

---

## 34. Expected-Type Completion

Exemplo:

```php
function save(UserRepository $repository): void {}

save(new |
```

Priorizar implementações compatíveis com:

```text
UserRepository
```

Usar:

* expected parameter type;
* hierarchy;
* declared type;
* type compatibility.

---

## 35. Statement Completion

Exemplo:

```php
if ($user|
```

ação:

```php
if ($user) {
    |
}
```

Outros:

```text
foreach
while
try
return
method declaration
```

---

## 36. Postfix Completion

Exemplo:

```php
$user.null
```

poderia sugerir:

```php
if ($user === null) {
}
```

Outro exemplo:

```php
$items.foreach
```

→

```php
foreach ($items as $item) {
}
```

Começar com poucos templates previsíveis.

---

## 37. Live Templates

Exemplos:

```text
fore
if
pubf
prif
try
```

expandidos em estruturas PHP.

Suportar placeholders navegáveis futuramente.

---

# Fase 7 — Imports e Code Style

## 38. Optimize Imports

Permitir:

* remover imports não utilizados;
* ordenar imports;
* eliminar duplicados;
* preservar aliases;
* respeitar namespace.

Importante: separar de Reformat.

---

## 39. Formatter robusto

Evoluir `Ctrl+Alt+L` para um pipeline de providers.

Possíveis providers:

```text
LSP
PHP CS Fixer
PHPCBF
formatter nativo
```

A seleção deve ser explícita e previsível.

Requisitos:

* não bloquear UI;
* validar document revision;
* respeitar language;
* respeitar capabilities;
* fallback seguro.

---

## 40. Format on Save

Somente depois do pipeline de formatting estar confiável.

Deve ser opcional.

---

# Fase 8 — Experiência do projeto

## 41. Recent Locations

Histórico de locais visitados:

```text
UserService::save
OrderRepository::find
routes.php:47
```

Diferente de Recent Files.

---

## 42. Navigation Switcher

Alternar rapidamente entre:

* tabs;
* terminal;
* Project;
* diagnostics;
* Find Usages.

---

## 43. Copy Path / Reference

Exemplos:

```text
src/Service/UserService.php
App\Service\UserService
App\Service\UserService::save()
```

---

## 44. Multiple Carets

Suporte a múltiplos cursores e seleções.

Operações:

* inserir;
* apagar;
* mover;
* selecionar;
* paste.

É uma feature importante, mas requer bastante cuidado no core do editor.

---

# Fase 9 — Análise de código

## 45. Inspect File

Executar inspections no arquivo atual e apresentar resultado consolidado.

---

## 46. Inspect Project

Executar análise em escopo maior.

Não deve usar UI thread.

Idealmente:

```text
Project
Directory
Module/package
Current file
```

---

## 47. Exception Analysis

Futuramente:

* `@throws`;
* throws explícitos;
* chamadas que propagam exceptions;
* catches.

Pode alimentar highlights e Quick Documentation.

---

# Fase 10 — Debugger

Somente quando editor, navigation e semantic estiverem maduros.

## 48. Breakpoints

* line breakpoints;
* enable/disable;
* conditions.

## 49. Logging Breakpoints

Executar sem suspender e registrar valores.

## 50. Evaluate Expression

Quando pausado no debugger:

```php
$user->getName()
```

mostrar valor.

## 51. Variables / Watches

Tool window com:

```text
Locals
Globals
Watches
Call Stack
```

---

# Fase 11 — Recursos avançados

Prioridade menor.

## 52. Local History

Histórico independente do Git para alterações locais.

---

## 53. Scratch Files

Arquivos temporários fora do projeto.

---

## 54. Regex Editor

Editor contextual para regex com escaping automático.

---

## 55. UML / Diagrams

Gerar hierarquias de classes e relações.

Começar com hierarchy textual antes de qualquer gráfico complexo.

---

## 56. Git Blame

Mostrar autor/commit por linha.

---

## 57. Pull Requests

Integração com GitHub/GitLab somente quando a camada Git principal estiver madura.

---

# Priorização estratégica

## P0 — Fundação

Funcionalidades pequenas que melhoram imediatamente o editor:

```text
Comment / Uncomment
Duplicate Line
Delete Line
Move Line
Expand Selection
Recent Files
File Structure
Highlight Usages
```

## P1 — IDE PHP real

Maior retorno sobre a infraestrutura atual:

```text
Go to Declaration
Find Usages
Show Usages
Go to Implementation
Search Classes
Search Symbols
Quick Documentation
Quick Definition
Type Hierarchy
```

## P2 — Refactoring e geração

```text
Rename
Extract Variable
Surround With
Generate Constructor
Generate Getter/Setter
Implement Methods
Override Methods
Optimize Imports
```

## P3 — Completion inteligente

```text
CamelCase
Fuzzy matching
Expected-type completion
Statement completion
Postfix completion
Live templates
```

## P4 — Plataforma avançada

```text
Inspect Project
Debugger
Local History
Git Blame
Scratch Files
UML
Pull Requests
```

---

# Cinco funcionalidades com maior retorno agora

Considerando o estado atual do Axiom, eu priorizaria:

### 1. Find / Show Usages

Transforma o índice semântico em uma ferramenta de navegação prática.

### 2. File Structure

Relativamente simples e extremamente útil.

### 3. Go to Implementation

Aproveita hierarchy e semantic model já existentes.

### 4. Quick Documentation

Aproveita signatures, PHPDoc, declared types e stubs.

### 5. Rename

Primeiro grande refactoring com impacto direto na percepção de “IDE de verdade”.

---

# Princípios para implementação

Toda nova funcionalidade deve seguir algumas regras.

**Não colocar trabalho pesado na UI thread.**

Preferir:

```text
resident indexes
snapshots
prefix lookup
background workers
revision/generation validation
```

Evitar:

```text
filesystem por tecla
O(project) durante completion
parse completo desnecessário
locks bloqueantes na UI
reconstrução de índices em hot path
```

Cada feature deve ser implementada isoladamente:

```text
auditoria
→ reprodução/contrato
→ teste
→ menor implementação
→ validação focada
→ teste manual
→ suíte ampla somente quando estável
```

E, principalmente, o objetivo do Axiom não deve ser:

> “ter tudo que o PhpStorm tem”.

Deve ser:

> **entregar as ações que desenvolvedores PHP usam todos os dias, com menos peso, menos fricção e excelente responsividade.**

Isso dá ao Axiom uma direção própria, ao mesmo tempo em que aproveita as melhores ideias de UX que IDEs maduras já demonstraram funcionar.
