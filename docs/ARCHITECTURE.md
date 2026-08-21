# Axiom — Arquitetura atual

> The project was originally bootstrapped as RustStorm and was renamed to Axiom.
> Historical stage reports and ADRs may retain the former name intentionally.

## Estado real

O workspace contém seis crates:

```text
axiom-app
└── GPUI 0.2.2
    └── WorkspaceView
        ├── Project explorer / tabs
        └── EditorView por tab
            ├── axiom-editor::Document
            └── axiom-syntax::PhpSyntax

axiom-editor
└── Document
    └── floem-editor-core 0.2.0
        └── lapce-xi-rope 0.3.2

axiom-syntax
└── Tree-sitter 0.25
    └── tree-sitter-php 0.24.2

axiom-project
├── Project / descoberta lazy
├── composer.json
└── mappings PSR-4

axiom-lsp
├── processo stdio em background
├── framing JSON-RPC / requests pendentes
├── sincronização de documentos
└── conversão UTF-8 ↔ posições LSP

axiom-php
├── provider de stubs externos
├── modelo nativo de símbolos PHP
└── índice de símbolos do runtime em memória
```

`axiom-app` depende dos três crates headless. `EditorView` traduz eventos de
teclado, mouse, clipboard e IME em operações públicas de `Document`; a UI não
mantém uma cópia paralela do texto.

O shell aceita `NoProject` como estado normal. A política headless de startup
prioriza argumento CLI, override `AXIOM_PROJECT` (legacy `RUSTSTORM_PROJECT`), um cwd com indicadores de
projeto e, por fim, a Welcome Screen. A mesma `WorkspaceView` abre, fecha e troca
o `Project`; essa transição encerra documentos LSP, descarta o bridge antigo e
limpa explorer/tabs antes de inicializar a nova raiz. Stubs de runtime continuam
globais e independentes do projeto.

## Editor visual

As linhas são apresentadas por `uniform_list`, portanto somente o intervalo
visível é materializado durante a rolagem. O hit-testing horizontal e a posição
do cursor usam as métricas do texto moldado pelo GPUI. A seleção visual pode
atravessar linhas, e o estado de cursor/seleção autoritativo continua no
`Document`.

O adaptador `EntityInputHandler` converte os intervalos UTF-16 exigidos pelo
sistema de entrada para offsets UTF-8 do core. Isso cobre texto composto, dead
keys e IME sem inserir texto diretamente no componente visual.

Arquivos são abertos pelo explorer e salvos em seu caminho original. Um picker
nativo de pasta e LSP permanecem fora do escopo.

## Sintaxe PHP

`PhpSyntax` mantém o texto necessário pelo parser, a árvore Tree-sitter, spans
de highlighting sem cores e símbolos sintáticos locais. Edições são aplicadas
com `InputEdit` e a árvore anterior é reutilizada no novo parse. Offsets e
colunas Tree-sitter são bytes UTF-8.

A query oficial publicada por `tree-sitter-php` produz as categorias. Somente
os spans que interceptam cada linha visível são enviados ao GPUI; a paleta fica
isolada na camada visual. Namespace, class, interface, trait, enum, function e
method são extraídos da árvore sem análise semântica ou indexação de projeto.

## Modelo de projeto

`axiom-project` normaliza a raiz, lê somente um nível de diretório por
operação e modela Composer/PSR-4 sem depender da GUI. `WorkspaceView` mantém a
árvore já carregada fora de `render()`; expandir uma pasta solicita seus filhos.

Cada tab possui uma entidade `EditorView` independente e, portanto, exatamente
um `Document`, uma árvore PHP opcional e estado próprio de cursor, seleção e
scroll. Caminhos canônicos impedem tabs duplicadas. Tabs dirty recusam fechamento
até serem salvas; não existe descarte implícito.

## Language Server Protocol

`axiom-lsp` é independente de GPUI e do Intelephense. O leitor de stdout e
stderr roda em threads próprias, preservando framing e correlacionando respostas
por ID. A aplicação mantém um bridge por projeto e uma versão LSP monotônica por
tab PHP; notificações não são geradas por renderização.

O servidor negocia UTF-8, UTF-16 ou UTF-32, com fallback UTF-16. Completion,
hover, diagnostics, definition e references retornam por uma fila de eventos
consumida periodicamente pela shell GPUI. Tree-sitter permanece responsável por
sintaxe local e highlighting.

O LSP é um provider externo e não é a fonte autoritativa de símbolos do
Axiom. Respostas LSP são consumidas como resultados transitórios da sessão;
elas não formam um índice interno nem substituem `axiom-syntax`, Composer ou
PSR-4. Um futuro provider nativo poderá combinar símbolos de projeto, Composer e
runtime PHP sem depender do Intelephense.

## Símbolos PHP de runtime

`axiom-php` é headless e depende de `axiom-syntax`, nunca de GPUI ou do
cliente LSP. `StubProvider` descobre arquivos PHP sob o caminho externo indicado
por `AXIOM_PHP_STUBS` (legacy `RUSTSTORM_PHP_STUBS`), deriva a extensão pelo primeiro diretório e constrói
um `RuntimeSymbolIndex` em memória. Leitura, parsing e indexação acontecem fora
de `render()`; a UI conserva o índice e apresenta apenas seu estado resumido.

O modelo distingue function, class, interface, trait, enum, method, property,
class constant e global constant, registrando origem, extensão, localização,
assinaturas, tipos PHPDoc e disponibilidade quando representável. Classes e
funções usam lookup case-insensitive; constantes globais permanecem
case-sensitive. Definições duplicadas são preservadas, e arquivos inválidos
geram erros isolados sem abortar o restante da carga.

O índice de runtime não é alimentado pelo Intelephense e ainda não participa de
completion ou resolução semântica. A descoberta de interpretadores PHP, o
sistema de tipos e a união com símbolos de projeto/Composer permanecem futuros.

## Editor headless

`Document` é a API pública. O `Buffer` do backend é privado e é a fonte
autoritativa para texto, histórico e estado pristine. O `Cursor` é a fonte
autoritativa para caret e seleção; não existe uma segunda seleção no documento.

### Posições

- offset interno: byte UTF-8;
- limites de edição: fronteiras de Unicode scalar/codepoint;
- offsets públicos inválidos: clamp ao documento e normalização para trás;
- navegação visual por grapheme: não definida;
- LSP: exigirá conversão futura para UTF-16.

### Undo/redo

Inserções consecutivas com o mesmo `EditType` formam um grupo. Mudança de
tipo de edição quebra o grupo. Cada revisão registra cursor/seleção antes e
cursor depois para restauração por undo/redo.

### EOL e arquivos

`Document` detecta o EOL predominante (`LF` ou `CRLF`). Novas linhas são
normalizadas para a política do documento. Save usa arquivo temporário no mesmo
diretório, flush/sync e persist/replace.

## Plataforma

O core headless, o `cargo check` e o link do aplicativo passam em WSL/Linux.
No ambiente atual, o backend Wayland do GPUI 0.2.2 falha por versão de protocolo
não suportada; forçando X11, o smoke test permanece ativo e renderiza sem panic.
A execução visual no Windows e o toolchain MSVC continuam não validados.
