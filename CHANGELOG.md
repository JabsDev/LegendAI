# Changelog

Todas as mudanças notáveis do **LegendAI** são documentadas aqui.

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/),
e o versionamento segue [SemVer](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Added

- **Transcrição STT (Whisper):**
  - Extração de áudio WAV 16kHz mono via sidecar ffmpeg e listagem/seleção de
    trilhas de áudio via ffprobe.
  - Integração `whisper-rs` (GGUF) com detecção de idioma e override manual.
  - Pipeline E2E de transcrição com teste de fixture.
- **Tradução multilíngue:**
  - Trait `TranslationEngine` plugável com engines NLLB (ONNX/`ort`) e
    Qwen2.5 (llama.cpp), factory por tier.
  - Batcher de segmentos numerados, parser com fallback por linha e prompt com
    contexto/glossário.
  - Swap de memória STT ↔ tradução (Tier 1) e pipeline completo com exportação
    SRT/ASS.
- **Formatador profissional:** regras de linhas (máx. 2), ~42 chars/linha,
  CPS alvo, duração mín/máx e zero overlap.
- **Model Manager:** catálogo curado (`catalog/models.json`), download com
  progresso/retomada, checksum SHA256, cache com lock, detecção de hardware e
  recomendação por tier, busca no Hugging Face e seleção de modelo ativo.
- **Config persistente** em TOML com escrita atômica e migração de schema.
- **UI (Svelte 5):** tema dark/light, importação de vídeo (drag-and-drop +
  trilha), tela de progresso por etapa, preview de vídeo com legenda, modo
  duplo original/traduzida, editor de legendas, i18n pt/en, fila de
  processamento e persistência de preferências.
- **Exportação:** SRT, ASS com estilização, legendas duplas, VTT e TXT.
- **Empacotamento:** installers NSIS (Windows), DMG (macOS arm64/amd64) e
  AppImage/`.deb` (Linux) com sidecar ffmpeg.

### Security

- Erros tipados com códigos estáveis para a UI, sem expor caminhos internos.

[Unreleased]: https://github.com/<user>/legendai/compare/v0.1.0...HEAD
