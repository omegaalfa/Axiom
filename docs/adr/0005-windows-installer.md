# ADR 0005 — Instalador Windows

**Status:** Proposto (Fase 10)
**Data:** 2026-08-19

## Contexto

RustStorm precisa de instalador profissional para Windows:
- Instalar em Program Files
- Criar atalhos no Menu Iniciar e Desktop (opcional)
- Registrar desinstalador
- Suportar upgrades
- Manter configurações do usuário
- Assinar digitalmente

## Decisão (proposta)

Usar **WiX Toolset** para gerar instalador MSI, via crate `cargo-wix` ou
scripts de build custom. Alternativa considerada: **Inno Setup** (mais
simples, gera EXE).

## Motivos

- **WiX (MSI):**
  - Padrão corporativo, bem suportado por admins de TI
  - Instalação silenciosa (`msiexec /quiet`)
  - Upgrades automáticos via MajorUpgrade
  - Integração com Group Policy

- **Inno Setup:**
  - Mais simples de criar (script Pascal)
  - Gera EXE autônomo
  - Bom para distribuição direta ao usuário final

## Trade-offs

- WiX: XML complexo, curva de aprendizado alta, requer WiX Toolset instalado
- Inno Setup: não é MSI, menos integrado com enterprise, mas mais rápido de
  desenvolver

## Decisão final

Começar com **Inno Setup** para prototipar (mais rápido), migrar para
**WiX MSI** para release 1.0.0.

## Consequências

- **Positivas:** Instalação profissional, upgrades suaves.
- **Negativas:** Manter dois scripts durante transição.
