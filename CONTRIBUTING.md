# Contribuindo com o LegendAI

> Guia para contribuir com o projeto: setup de desenvolvimento, build com features de GPU, como rodar os testes e as convenções de código e de PR. O planejamento completo está em [`PLANNING.md`](PLANNING.md).

## Código de conduta

Ao participar deste projeto, você concorda em manter um ambiente respeitoso e construtivo. Assédio, discriminação e comportamento agressivo não são tolerados. Se você observar uma violação, reporte ao mantenedor por issue ou e-mail.

## Setup de desenvolvimento

Pré-requisitos do [Tauri v2](https://tauri.app/start/prerequisites/):

- **Rust** (toolchain estável) — com `rustfmt` e `clippy`
- **Node.js** 22+
- **Linux:** libs de sistema do WebKitGTK 4.1 (Ubuntu/Debian):

  ```bash
  sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
    libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libgtk-3-dev patchelf
  ```

Clone e instale:

```bash
git clone https://github.com/<user>/legendai.git
cd legendai
npm install
bash scripts/fetch-ffmpeg.sh   # baixa os sidecars ffmpeg/ffprobe locais (ADR-003)
npm run tauri dev              # abre o app em modo dev
```

> **Sidecars:** os binários `ffmpeg`/`ffprobe` são baixados por `scripts/fetch-ffmpeg.sh` (com argumento por OS: `linux64`, `win64`, `macos-arm64`, `macos-amd64`) e **não são commitados** (ADR-003). Sem eles o app roda, mas o pipeline de áudio/vídeo falha.

## Estrutura do projeto

| Caminho               | O que é                                            |
| --------------------- | -------------------------------------------------- |
| `src/`                | Frontend Svelte 5 + TypeScript (componentes, i18n) |
| `src-tauri/src/`      | Backend Rust (comandos IPC, pipeline, modelos)     |
| `catalog/models.json` | Catálogo curado de modelos (STT e tradução)        |
| `scripts/`            | Scripts de dev/release (`fetch-ffmpeg.sh`)         |
| `docs/`               | Guias de usuário (instalação, modelos, tradução)   |
| `PLANNING.md`         | Quadro de tarefas e ADRs                           |
| `.github/workflows/`  | CI e release                                       |

Principais módulos do backend (`src-tauri/src/`):

- `audio/` — extração WAV (ffmpeg) e inspeção de trilhas (ffprobe)
- `stt/` — engine whisper.cpp via `whisper-rs`
- `translate/` — engines de tradução (NLLB via `ort` com CUDA EP, TowerInstruct via `llama_cpp` com GPU), batcher, parser, prompt, glossário
- `pipeline/` — orquestração (STT, tradução com swap de memória, fila de jobs)
- `model_manager/` — catálogo, download com retomada, checksum, cache
- `format/` e `subtitles/` — formatação profissional e serializers (SRT/ASS/VTT/TXT)
- `commands/` — comandos IPC expostos ao frontend

## Build com features

Os backends pesados ficam **atrás de feature flags** para manter o build rápido em dev. A feature default é **CPU apenas** (`cargo build` não compila nenhum backend).

| Feature  | O que ativa                                    | Requisito do sistema          |
| -------- | ---------------------------------------------- | ----------------------------- |
| `stt`    | `whisper-rs` (whisper.cpp)                     | cmake (compilado na hora)     |
| `llama`  | `llama_cpp` (TowerInstruct via llama.cpp, GPU) | cmake                         |
| `ort`    | `ort` + `tokenizers` (NLLB ONNX)               | baixa o runtime ONNX no build |
| `full`   | `stt` + `llama` + `ort` (CPU)                  | —                             |
| `cuda`   | backends com CUDA                              | CUDA toolkit (`nvcc`)         |
| `metal`  | backends com Metal                             | macOS apenas                  |
| `vulkan` | backends com Vulkan                            | Vulkan SDK (`glslc`)          |

Buildar com os três backends (CPU):

```bash
cargo build --features full        # em src-tauri/
```

Build com GPU (combinável com `stt`/`llama`/`ort`):

```bash
cargo build --features full,cuda    # exige CUDA toolkit no PATH (nvcc)
cargo build --features full,metal   # macOS apenas
cargo build --features full,vulkan  # exige Vulkan SDK (glslc)
```

> A feature `full` **não** inclui `cuda`/`metal`/`vulkan` deliberadamente: `cargo build --features full` precisa compilar sem toolkit GPU. As features de GPU são repassadas aos backends via `whisper-rs?/cuda`, `llama_cpp?/cuda`, `ort?/cuda` etc. — ativam só quando o backend correspondente está ligado.

Frontend:

```bash
npm run build          # build de produção do frontend
npm run dev            # Vite dev isolado (sem Tauri)
```

## Como rodar os testes

Gate de CI (ver `.github/workflows/ci.yml`):

```bash
cargo fmt -- --check            # em src-tauri/
cargo clippy --all-targets -- -D warnings   # em src-tauri/
cargo test                      # em src-tauri/ (features default)
npm run lint
npm run format                  # Prettier --check
npm run check                   # svelte-check
npm run build
```

Backend com STT (cobre também os testes sob `--features stt`):

```bash
cargo test --features stt       # em src-tauri/
```

### Testes que exigem modelo real

Testes que dependem de modelos baixados da rede são `#[ignore]` + env var — **nunca rodam no CI** e nunca baixam modelo:

| Teste                     | Env vars                                                    | Como rodar                                              |
| ------------------------- | ----------------------------------------------------------- | ------------------------------------------------------- |
| E2E do pipeline STT       | `LEGENDAI_MODEL_PATH`, `LEGENDAI_FIXTURE` (opcional)        | `cargo test --features stt --test e2e_stt -- --ignored` |
| Transcrição (1.4/1.5)     | `LEGENDAI_MODEL_PATH`, `LEGENDAI_WAV_PATH`, `LEGENDAI_LANG` | `cargo test --features stt -- --ignored`                |
| Download manual de modelo | `LEGENDAI_MODEL_REPO`, `LEGENDAI_MODEL_FILE`                | `cargo test --features stt download -- --ignored`       |
| NLLB (3.2)                | `LEGENDAI_NLLB_ENC/DEC/TOK`                                 | `cargo test --features ort -- --ignored`                |
| LLM (3.3)                 | `LEGENDAI_LLM_PATH`                                         | `cargo test --features llama -- --ignored`              |

Os modelos baixam para o cache do app (`~/.cache/legendai/models/` no Linux) — use-os como fixture local.

## Convenções de código

- **Rust:** `cargo fmt` + `cargo clippy --all-targets -- -D warnings` **limpos** (gate de merge). Sem `unsafe`. Erros tipados com `thiserror` e mensagens estáveis/código para a UI (ADR-006).
- **Frontend:** ESLint + Prettier com plugin Svelte; `npm run check` (svelte-check) sem erros. Svelte 5 com runes (`$state`/`$derived`/`$props`).
- **Arquitetura:** módulos pequenos e coesos; sem dependência desnecessária (stdlib/nativo primeiro). Testes unitários por módulo com fixtures geradas em runtime (sem rede). Ver "Estratégia de Testes" no [PLANNING.md](PLANNING.md).
- **Documentação:** docs em **pt-BR** (idioma do projeto); a UI tem i18n `pt`/`en` (`src/i18n/`).

### Mensagens de commit

Commits pequenos, atômicos e descritivos, no padrão [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(model_manager): retomada de download via Range header
fix(pipeline): clamp do end da última legenda na duração do áudio
docs(contributing): guia de contribuição
```

Formato: `<tipo>(<escopo>): <resumo>`. Tipos comuns: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `build`.

## Como fazer um PR

1. **Converse antes:** para mudanças grandes, abra uma issue discutindo a abordagem antes de codar (ou anote no PLANNING.md).
2. **Branch:** crie uma branch descritiva a partir de `master` (`feat/...`, `fix/...`).
3. **Mantenha o escopo:** um PR = uma mudança (uma tarefa do PLANNING). PRs gigantes demoram a revisar.
4. **Cumpra o gate de CI:** todos os checks do `.github/workflows/ci.yml` precisam passar (fmt, clippy `-D warnings`, testes, lint, build).
5. **Testes:** adicione testes para o que mudou (mesmo padrão dos módulos vizinhos) e, se a mudança é de comportamento, atualize os snapshots do prompt/format.
6. **Documente:** atualize os docs afetados e o `PLANNING.md` (status da tarefa, notas) conforme o padrão das tarefas concluídas.
7. **Preencha o template** de PR (checklist) — o mantenedor usa a checklist para a revisão.

### O que um PR deve incluir

- Descrição do **problema** e da **solução** (referencie a issue/tarefa).
- Mudanças de comportamento listadas.
- Evidência de validação (testes rodados, resultado de `clippy`/`fmt`).
- Screenshots para mudanças de UI.

## Reportando bugs

Use o template de **bug report** em `.github/ISSUE_TEMPLATE/bug_report.md`. Inclua **sempre** o trecho do arquivo de log (caminhos por OS na seção "Onde ficam os arquivos" do [docs/INSTALL.md](docs/INSTALL.md)) — o log registra a etapa do pipeline, o pico de memória e os erros tipados, o que acelera muito o diagnóstico.

## Documentação para usuários

Guias de usuário em `docs/` são parte do produto (público não-técnico): ao mudar comportamento do app (modelos, tier, instaladores, paths), atualize [`docs/INSTALL.md`](docs/INSTALL.md), [`docs/MODELS.md`](docs/MODELS.md) e [`docs/TRANSLATION.md`](docs/TRANSLATION.md) e marque a checklist de revisão no fim do INSTALL.md.
