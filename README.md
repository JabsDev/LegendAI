# LegendAI

> App desktop **open-source** que gera legendas sincronizadas com IA — **100% local** (transcrição Whisper + tradução multilíngue), sem nuvem e sem envio de dados.

## O que é

O LegendAI transforma vídeos em legendas profissionais (SRT/ASS) em minutos: transcreve o áudio com **Whisper**, traduz para outros idiomas e aplica regras de formatação de qualidade (máximo 2 linhas, ~42 chars/linha, timing otimizado). Tudo roda localmente na sua máquina.

**Fluxo principal:** importar vídeo → escolher trilha de áudio (ou usar legenda embutida) → transcrever → traduzir (opcional) → formatar → exportar `.srt`/`.ass`.

## Stack

| Camada      | Tecnologia                                                                  |
| ----------- | --------------------------------------------------------------------------- |
| Frontend    | Svelte 5 + TypeScript + Vite                                                |
| Shell       | Tauri v2 (Rust)                                                             |
| Transcrição | whisper.cpp via `whisper-rs`                                                |
| Tradução    | NLLB-200 via ONNX Runtime (`ort`, CUDA) e TowerInstruct via llama.cpp (GPU) |
| Áudio/Vídeo | ffmpeg/ffprobe (sidecar)                                                    |

## Tiers de hardware

O app detecta seu hardware (GPU via `nvidia-smi`) e recomenda modelos compatíveis — tradução usa **GPU quando disponível** (build `--features cuda`):

| Tier  | Hardware          | Stack de tradução (GPU acelera)               |
| ----- | ----------------- | --------------------------------------------- |
| **1** | ~4GB RAM, CPU     | NLLB-200-600M (ONNX, CUDA quando há GPU)      |
| **2** | ~8GB RAM, CPU/GPU | TowerInstruct-7B q4 (GGUF, 256 layers na GPU) |
| **3** | 16GB+ ou GPU      | TowerInstruct-7B q6 (GGUF, 512 layers na GPU) |

## Instalação

Baixe o instalador do seu sistema no [release mais recente](https://github.com/<user>/legendai/releases) — NSIS (Windows), DMG (macOS arm64/amd64) ou AppImage/`.deb` (Linux). Guias completos:

- [**docs/INSTALL.md**](docs/INSTALL.md) — instalação por OS, requisitos de sistema e solução de problemas
- [**docs/MODELS.md**](docs/MODELS.md) — como escolher os modelos (STT/tradução) por Tier
- [**docs/TRANSLATION.md**](docs/TRANSLATION.md) — NLLB vs TowerInstruct: qual engine usar (GPU)

> Os modelos são baixados na primeira execução (catálogo em [`catalog/models.json`](catalog/models.json)); depois disso o app é **100% offline**.

## Status do projeto

Em desenvolvimento. Veja o planejamento completo e o progresso em [`PLANNING.md`](PLANNING.md).

## Desenvolvimento

Pré-requisitos: [Tauri v2](https://tauri.app/start/prerequisites/) (Rust, Node, libs webkit no Linux).

```bash
npm install
bash scripts/fetch-ffmpeg.sh   # baixa os sidecars ffmpeg/ffprobe locais (ADR-003)
npm run tauri dev
```

- `npm run build` — build do frontend
- `npm run lint` / `npm run format` — ESLint + Prettier
- `cargo check` / `cargo clippy -- -D warnings` — backend (em `src-tauri/`)
- Features GPU do backend: `cargo build --features cuda|metal|vulkan` (em `src-tauri/`)

## Licença

MIT — veja [LICENSE](LICENSE).
