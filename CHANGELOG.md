# Changelog

Todas as mudanças notáveis do **LegendAI** são documentadas aqui.

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/),
e o versionamento segue [SemVer](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Added

- Nada pendente.

## [v0.1.13] - 2026-09-01

### Fixed

- **Windows (runtime, sidecar ffprobe/ffmpeg):** instalado, o app falhava em
  qualquer importação de vídeo com "sidecar ffprobe não encontrado". O
  `binary_path()` procurava `ffprobe`/`ffmpeg` **sem a extensão `.exe`** que o
  Tauri exige em sidecars Windows — tanto em dev (`src-tauri/binaries/`) quanto
  em produção (`exe_dir/`). Agora resolve `ffprobe.exe`/`ffmpeg.exe` no Windows
  (e mantém os nomes sem extensão nos demais OS), com fallbacks para o
  diretório `Resources` (macOS) e `PATH` (dev sem sidecar).
- **Linux (.deb/AppImage):** o pacote estava **quebrado em instalação limpa** —
  o binário linkava dinamicamente `libllama.so.0`/`libggml*.so.0` (default
  `dynamic-link` do `llama-cpp-4`) mas essas libs não eram distribuídas no
  pacote. O llama.cpp agora é linkado **estaticamente** dentro do `legendai`
  (`default-features = false`, feature `openmp`) em todos os OS: instala e roda
  sem dependências extras. O job de release valida por `ldd` que não há libs
  faltando antes de publicar.
- **CI:** builds voltam a passar nos 3 OS. O `tauri_build` exige os sidecars
  `externalBin` em qualquer `cargo clippy/test/build` — o CI agora baixa
  ffmpeg/ffprobe antes; `.gitattributes` com `eol=lf` evita falha do Prettier
  por CRLF no runner Windows; `binaries/native` é pré-criado no CI.
- **Release (Windows amd-intel/Vulkan):** o gerador Visual Studio do CMake
  quebrava no `ExternalProject` `vulkan-shaders-gen` (MSB8066 "cannot find the
  batch label specified - VCEnd"), mesmo com o `vulkan-shaders-gen.exe`
  compilado. O job agora usa `CMAKE_GENERATOR=Ninja`.
- **Release (macOS intel):** o cross `x86_64` linkava o **OpenSSL ARM64 do
  Homebrew** no `libllama-common` (`_ASN1_*`/`_ERR_*` indefinidos) — a flag
  `LLAMA_CURL` usada antes não existe mais no llama.cpp. Corrigido com toolchain
  `scripts/cmake/no-openssl.cmake` (`LLAMA_OPENSSL=OFF`) via
  `CMAKE_TOOLCHAIN_FILE`; HTTPS do server do llama.cpp não é usado pelo app.
- **Release (Windows ARM64 Snapdragon):** o driver `clang` GNU rejeita `-fPIC`
  para o target `aarch64-pc-windows-msvc` ("unsupported option"); os jobs agora
  usam `clang-cl`, que ignora `-fPIC` com warning (PIC é implícito no Windows).

### Added

- **Guarda contra instalador quebrado:** `tauri build` em release falha com
  instrução clara se os sidecars ffmpeg/ffprobe estiverem ausentes; o workflow
  de release valida o **conteúdo** dos instaladores antes de publicar (NSIS via
  `7z l`: `legendai.exe`/`ffmpeg.exe`/`ffprobe.exe`; DMG: monta e verifica
  `Contents/MacOS`; `.deb`: `ar`+`tar`+`ldd` sem "not found").
- Erros de mídia na importação (`inspect_video`) agora chegam à UI como código
  estável + i18n + hint (ex. `ffmpeg_missing` → "Reinstale o aplicativo..."),
  em vez da mensagem crua de dev.

## [v0.1.12] - 2026-08-30

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

### Fixed

- **Windows:** instalador NSIS agora embarca `llama.dll` + `ggml-*.dll` ao lado
  do exe (via `tauri.windows.conf.json` → `bundle.resources`). Antes o app
  instalado falhava com "llama.dll não encontrada" porque o llama.cpp é
  linkado dinamicamente (`dynamic-link` default do `llama-cpp-4`).
- **Release Windows (build):** o build dos instaladores NSIS voltou a funcionar
  no CI. O glob `target/**/release/*.dll` em `bundle.resources` fazia o
  `tauri-build` tentar copiar as DLLs do onnxruntime que o `ort-sys` cria como
  **symlinks** para `$CARGO_HOME/registry`; o `fs::copy` recaía sobre o próprio
  symlink e o Windows falhava com "os error 32" (sharing violation) — todos os
  jobs `windows` quebravam e o release ficava sem `.exe`. O workflow agora
  compila sem bundle, usa `nullglob` + verificacao e copia as DLLs como arquivos
  reais para `src-tauri/binaries/native/` (`cp -L`) e só então gera o NSIS.
- **Release Windows (variante amd-intel/Vulkan):** o `humbletim/install-vulkan-sdk`
  extrai o instalador com 7z e, no Windows, o SDK fica incompleto (só `Bin/`,
  sem `Include/`/`Lib/`) — o `FindVulkan` do CMake falhava no `whisper-rs-sys`.
  O job agora usa o instalador oficial do SDK em modo silencioso (SDK completo
  em `C:\VulkanSDK\<versão>`) com retry e validacao, tratando falhas como warning
  (best-effort).
- **Release (macOS intel):** cross `x86_64` em runner `arm64` falhava no link de
  `libllama-common.dylib` com `_ERR_*` indefinidos (cpp-httplib TLS sem
  `OpenSSL` para o target); `LLAMA_CURL=OFF` via `CMAKE_ARGS` remove a
  dependencia.
- **Release (Windows ARM64 Snapdragon):** `MSVC is not supported for ARM` no
  `whisper-rs-sys`/`llama-cpp-sys`; jobs agora exigem `clang` (`CC/CXX` +
  `CMAKE_GENERATOR=Ninja`) e two-phase build, restaurando geracao de `.exe`
  `ARM64` (best-effort).

### Security

- Erros tipados com códigos estáveis para a UI, sem expor caminhos internos.

[Unreleased]: https://github.com/<user>/legendai/compare/v0.1.0...HEAD
