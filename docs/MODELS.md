# Guia de Modelos

> Os modelos são baixados do [Hugging Face](https://huggingface.co) a partir de um catálogo curado em [`catalog/models.json`](../catalog/models.json). Cada modelo tem **tamanho**, **RAM mínima**, **qualidade** (1–5) e **velocidade** (1–5). Após o download, tudo roda **100% local** — com aceleração por GPU quando disponível.

## Como o app escolhe

O app detecta seu hardware e classifica em **Tier** (1/2/3) — ver [INSTALL.md](INSTALL.md). Na aba **Modelos**, o catálogo é filtrado por compatibilidade com o seu Tier; na primeira execução, os **recomendados** já aparecem com botão de download.

- **STT (transcrição):** Whisper — escolha entre precisão (qualidade) e velocidade.
- **Tradução:** NLLB (rápido, CPU/GPU via ONNX) ou TowerInstruct (LLM especializado em tradução, com aceleração GPU via llama.cpp).

## Modelos de transcrição (STT — Whisper)

Tecnologia: [whisper.cpp](https://github.com/ggerganov/whisper.cpp) via `whisper-rs`. Modelos `ggml-*.bin` do repo oficial `ggerganov/whisper.cpp`.

| Modelo                      | Tamanho | RAM mín. | Qualidade | Velocidade | Ideal para                      |
| --------------------------- | ------- | -------- | --------- | ---------- | ------------------------------- |
| **Whisper Tiny** (fp16)     | 78 MB   | 1 GB     | 1         | 5          | Testes, máquinas muito fracas   |
| **Whisper Small** (q5_1)    | 190 MB  | 2 GB     | 3         | 4          | Padrão do Tier 1 — boa precisão |
| **Whisper Medium** (q5_0)   | 539 MB  | 3 GB     | 4         | 3          | Melhor precisão com custo médio |
| **Whisper Large v3** (q5_0) | 1,08 GB | 4 GB     | 5         | 2          | Máxima precisão (Tiers 2/3)     |

Quanto maior o modelo, melhor a transcrição de sotaques, ruído de fundo e nomes próprios — e mais lenta. Para legendas de qualidade profissional, prefira **Small** ou maior; **Tiny** serve para conferir o fluxo.

## Modelos de tradução

### NLLB-200 (ONNX Runtime) — rápido e leve, agora com GPU

Repo: `Xenova/nllb-200-distilled-600M`. Traduz entre **35 idiomas** (pt, en, es, fr, de, it, ja, zh, ar, ru e mais). Não usa contexto de fala — traduz segmento a segmento; qualidade boa para texto direto, média para gírias/contexto de série. Quando o app é compilado com `--features cuda` e uma GPU NVIDIA é detectada (`nvidia-smi`), o NLLB usa **CUDA Execution Provider** do ONNX Runtime — até 3× mais rápido que CPU.

| Modelo         | Tamanho | RAM mín. | Qualidade | Velocidade | GPU  |
| -------------- | ------- | -------- | --------- | ---------- | ---- |
| **NLLB q4f16** | 1,27 GB | 2 GB     | 2         | 5          | CUDA |
| **NLLB fp16**  | 1,78 GB | 3 GB     | 3         | 5          | CUDA |

### TowerInstruct 7B (llama.cpp) — tradução dedicada, com GPU

Repo: `mradermacher/TowerInstruct-7B-v0.2-GGUF` (quantizações GGUF de `Unbabel/TowerInstruct-7B-v0.2` — modelo **focado em tradução**, tag `translate` no Hugging Face, finetuned para tradução com contexto, terminologia e coerência). Traduz com **contexto** (segmentos anteriores), preservando nomes, pronomes e tom — ideal para séries. Suporta **17 idiomas**. Quando GPU é detectada, descarrega camadas na GPU (`n_gpu_layers`: 256 no Tier2, 512 no Tier3) — exige build `--features cuda`.

| Modelo                | Tamanho | RAM mín. | Qualidade | Velocidade | GPU  |
| --------------------- | ------- | -------- | --------- | ---------- | ---- |
| **Tower 7B** (q4_k_m) | 4,08 GB | 5 GB     | 4         | 3          | CUDA |
| **Tower 7B** (q5_k_m) | 4,78 GB | 5 GB     | 4         | 3          | CUDA |
| **Tower 7B** (q6_k)   | 5,53 GB | 8 GB     | 5         | 2          | CUDA |

> TowerInstruct é um LLM **especializado em tradução** (não um LLM genérico como Qwen). Benchmarks mostram qualidade superior em tradução com o mesmo custo, e o tag `translate` no HF facilita encontrar alternativas.

## Como escolher (resumo por Tier)

| Tier                 | Transcrição          | Tradução              | Por quê                                             |
| -------------------- | -------------------- | --------------------- | --------------------------------------------------- |
| **1** (4–5 GB)       | Whisper **Small**    | **NLLB q4f16**        | LLM não cabe confortavelmente; NLLB é rápido e leve |
| **2** (6–15 GB)      | Whisper **Large v3** | **Tower 7B** (q4_k_m) | Qualidade alta com contexto; GPU acelera se houver  |
| **3** (16 GB+ / GPU) | Whisper **Large v3** | **Tower 7B** (q6_k)   | Máxima qualidade; GPU acelera muito                 |

> Regras de ouro:
>
> - **Precisão da transcrição importa mais que tudo** → comece pelo modelo STT maior que couber no seu Tier.
> - **Qualidade de tradução em séries** → Tower (contexto + especializado). **Só precisa de tradução rápida** → NLLB.
> - **Muito vídeo / máquina fraca** → modelos menores (Tiny/Small + NLLB q4). Velocidade de processamento é proporcional ao modelo.
> - **Tem GPU NVIDIA?** Recompile/instale o build com `--features cuda` (`cargo build --features full,cuda`) para usar a placa — sem isso, tudo roda em CPU.

## Links para os repositórios

- Whisper: https://huggingface.co/ggerganov/whisper.cpp
- NLLB (ONNX): https://huggingface.co/Xenova/nllb-200-distilled-600M
- TowerInstruct 7B GGUF: https://huggingface.co/mradermacher/TowerInstruct-7B-v0.2-GGUF (origem: https://huggingface.co/Unbabel/TowerInstruct-7B-v0.2)

## Integridade e cache

- Cada download é verificado por **SHA-256** contra o catálogo; download corrompido é removido e baixado de novo.
- Modelos ficam em `~/.cache/legendai/models/<kind>/<id>/` (Linux), `~/Library/Caches/legendai/models/` (macOS) ou `%LOCALAPPDATA%\legendai\models\` (Windows).
- Para liberar espaço: aba **Modelos** → botão **Remover**.
- Modelos são do **usuário** (baixados pós-instalação) — o instalador não inclui nenhum, o que mantém o app pequeno e em conformidade com as licenças.
