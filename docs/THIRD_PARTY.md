# Dependências de terceiros — baseline atual

Este inventário é técnico e não substitui revisão jurídica.

| Componente | Versão resolvida | Origem | Licença | Uso |
|---|---:|---|---|---|
| GPUI | 0.2.2 | crates.io | Apache-2.0 | Bootstrap GUI |
| floem-editor-core | 0.2.0 | crates.io | MIT | Buffer, cursor, seleção e histórico |
| lapce-xi-rope | 0.3.2 | crates.io, transitiva | Apache-2.0 | Rope |
| tempfile | 3.27.0 | crates.io | MIT OR Apache-2.0 | Save atômico e testes de I/O |
| anyhow | 1.0.104 | crates.io | MIT OR Apache-2.0 | Erros do app |
| tracing | 0.1.44 | crates.io | MIT | Logging |
| tracing-subscriber | 0.3.23 | crates.io | MIT | Inicialização do logging |
| embed-resource | 3.0.11 | crates.io | MIT | Recursos Windows |
| tree-sitter | 0.25.10 | crates.io | MIT | Parsing incremental |
| tree-sitter-php | 0.24.2 | crates.io / tree-sitter/tree-sitter-php | MIT | Grammar PHP e query oficial de highlighting |
| serde | 1.0.229 | crates.io | MIT OR Apache-2.0 | Modelo de Composer |
| serde_json | 1.0.151 | crates.io | MIT OR Apache-2.0 | Parsing de composer.json |
| lsp-types | 0.97.0 | crates.io | MIT OR Apache-2.0 | Tipos Language Server Protocol |
| url | 2.5.8 | crates.io | MIT OR Apache-2.0 | Conversão segura entre paths e file URIs |
| rfd | 0.17.2 | crates.io | MIT | Seletores cross-platform de arquivo e diretório |
| directories | 6.0.0 | crates.io | MIT OR Apache-2.0 | Diretório de configuração específico da plataforma |
| open | 5.3.x | crates.io | MIT OR Apache-2.0 | Abertura segura de URLs no navegador padrão |
| portable-pty | 0.9.0 | crates.io | MIT | PTY/ConPTY para terminal integrado |
| vt100 | 0.16.2 | crates.io | MIT | Parser VT100 e tela em memória do terminal integrado |

## Fonte externa opcional

O Axiom pode ler um checkout fornecido pelo usuário de
`JetBrains/phpstorm-stubs`. Esse conteúdo não é dependência Cargo, não é
vendorizado, baixado ou redistribuído pelo projeto. O repositório upstream é
Apache-2.0 e informa que partes derivadas da documentação PHP estão sob
CC-BY-3.0; qualquer estratégia futura de distribuição exigirá revisão específica
de atribuição e compatibilidade.

As versões exatas de todo o grafo estão em `Cargo.lock`. Não há dependências
Git diretas no baseline. Dependências planejadas, mas não utilizadas, foram
removidas do manifest raiz.
