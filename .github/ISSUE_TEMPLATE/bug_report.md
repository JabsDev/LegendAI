---
name: Bug report
about: Reporte um bug para nos ajudar a corrigir
title: "[Bug] "
labels: bug
assignees: ""
---

**Descreva o bug**
Uma descrição clara e concisa do que está errado.

**Passos para reproduzir**

1. Abra o app e vá em "Importar"
2. Arraste o vídeo `...`
3. Clique em "Processar"
4. O erro aparece em ...

**Comportamento esperado**
O que deveria acontecer.

**Comportamento atual**
O que aconteceu de fato (cole a mensagem de erro exata).

**Ambiente**

- **SO:** Windows / macOS (arm64/amd64) / Linux (distro + versão)
- **Versão do app:** (do release ou `git describe`)
- **Método de instalação:** NSIS / DMG / AppImage / deb / dev (`npm run tauri dev`)
- **Hardware (Tier):** RAM total, CPU, GPU (se houver)
- **Modelos ativos:** (ex: whisper-small + nllb-200-distilled-600m-q4)

**Trecho do log**
Cole o trecho relevante do arquivo de log (path por OS na tabela "Onde ficam os arquivos" do [docs/INSTALL.md](../../docs/INSTALL.md)):

- Linux: `~/.local/state/legendai/`
- macOS: `~/Library/Application Support/legendai/`
- Windows: `%LOCALAPPDATA%\legendai\`

```text
<cole o trecho do log aqui, com timestamps>
```

**Contexto adicional**
Qualquer outra informação útil (vídeo problemático, comportamento observado antes, etc.).
