# Guia de Tradução

> O LegendAI tem **duas engines de tradução**: **NLLB-200** (via ONNX Runtime) e **TowerInstruct** (via llama.cpp) — ambas **especializadas em tradução** (tag `translate` no Hugging Face). Este guia explica a diferença em linguagem simples e quando usar cada uma.

## As duas engines em uma tabela

|                          | **NLLB-200**                                           | **TowerInstruct 7B**                        |
| ------------------------ | ------------------------------------------------------ | ------------------------------------------- |
| O que é                  | Modelo de tradução especializado (Meta)                | LLM especializado em tradução (Unbabel)     |
| Backend                  | ONNX Runtime (`ort`) — com **CUDA EP** quando há GPU   | llama.cpp (`llama`) — com offload GPU       |
| Velocidade               | **Muito rápida** em CPU; **3× mais rápida com CUDA**   | Média em CPU / **rápida com GPU** (offload) |
| RAM                      | 2–3 GB                                                 | 5–8 GB                                      |
| Idiomas                  | **35** (incl. pt, en, es, fr, de, it, ja, zh, ar, ru…) | **17** (pt, en, es, fr, de, it, ja, zh…)    |
| Contexto                 | ✗ traduz segmento isolado                              | ✓ usa os segmentos **anteriores**           |
| Gírias/contexto de série | Média                                                  | **Alta**                                    |
| Roda em Tier 1           | ✓                                                      | ✗ (não cabe em 4 GB com conforto)           |
| Usa GPU?                 | ✓ CUDA EP (requer `--features cuda`)                   | ✓ `n_gpu_layers` (256/512, requer `cuda`)   |

## Quando usar cada uma

### NLLB-200 — "traduzir rápido, em qualquer máquina"

Use quando:

- Seu computador tem **4–5 GB de RAM** (Tier 1) — é a única engine que cabe com folga.
- Você traduz **muitos vídeos** e prefere velocidade.
- Os textos são diretos (documentários, palestras) — NLLB traduz bem frase a frase.
- **Tem GPU NVIDIA?** Com build `--features cuda`, o NLLB usa CUDA EP e fica ainda mais rápido.

Limitação: por não enxergar o contexto (segmentos vizinhos), **gírias, nomes próprios e pronomes** podem sair inconsistentes em conversas/séries.

### TowerInstruct — "melhor qualidade, com contexto"

Use quando:

- Seu computador tem **6 GB ou mais** (Tiers 2/3).
- A qualidade da tradução importa mais que a velocidade (**séries, filmes, conteúdo conversacional**).
- Você quer termos consistentes ao longo do vídeo (o modelo "lembra" o que veio antes).

Benefício vs Qwen genérico: TowerInstruct é **finetuned especificamente para tradução** (tag `translate`), com melhor precisão em terminologia e coerência que LLMs genéricos do mesmo tamanho. Suporta aceleração por GPU: com `--features cuda` e GPU detectada (`nvidia-smi`), descarrega 256 layers (Tier2) ou 512 (Tier3) na placa.

Limitação: mais pesado. Em CPU puro, um vídeo de 1h leva mais tempo que com NLLB; em **GPU** a diferença encolhe muito (até 5× mais rápido).

## Como o app decide

1. Você marca um modelo como **ativo** na aba **Modelos** (ou aceita a recomendação dos primeiros-passos).
2. O pipeline usa **o modelo ativo**: `nllb-*` → engine NLLB; `tower*` → engine Tower (llama).
3. O Tier do seu hardware limita o catálogo, mas a escolha final é sua.
4. Se GPU for detectada (`nvidia-smi` → CUDA), o app loga `CUDA EP habilitado` (NLLB) ou `usando GPU com N layers` (Tower). Sem `--features cuda` no build, loga aviso e usa CPU.

> Se o backend do modelo ativo não estiver disponível (ex.: build sem a feature), o app **não troca silenciosamente** — avisa com erro claro (padrão de erros do app).
> **Dica GPU:** para usar a placa (RTX 3070 etc.), instale o build com `cargo build --features full,cuda` (exige CUDA toolkit) ou baixe o release `*-cuda` quando disponível. Sem isso a placa fica ociosa.

## Qualidade e ajustes

O LegendAI já aplica boas práticas em qualquer engine:

- **Formatação profissional** do texto traduzido (máx. 2 linhas, ~42 chars/linha, timing otimizado) — idêntica à da transcrição.
- **Tradução em lote** com fallback: linhas que o modelo não responder corretamente são **re-tentadas**; se persistir a falha, o app **mantém o original** e marca como `kept_original` — nunca descarta conteúdo.
- **Idioma detectado automaticamente** pelo Whisper vira o idioma de origem; você pode sobrescrever origem/destino antes de processar.

### Glossário

Em **Configurações → Glossário**, você define termos fixos (ex.: `Bob → Roberto`), que são injetados no prompt do Tower. Funciona apenas na engine **Tower** (NLLB não usa prompt — ignora o glossário).

### Instruções e formalidade

Em **Configurações → Tradução**, você ajusta formalidade (formal/coloquial) e instruções livres ("preservar apelidos", "não traduzir nomes de marcas"). Também **apenas no Tower** — NLLB traduz literalmente.

## Metas de velocidade (referência)

Para um vídeo de 1 hora, o tempo de processamento alvo por Tier:

| Tier | Meta (1h de vídeo)                         |
| ---- | ------------------------------------------ |
| 1    | ~30 min (NLLB CPU, ~10 min com CUDA)       |
| 2    | ~10 min (Tower 7B q4 com GPU, ~25 min CPU) |
| 3    | ~3 min (Tower 7B q6 com GPU)               |

Essas metas dependem do hardware real e do tamanho do modelo STT — são orientação de produto, não garantia. Com GPU (CUDA), a tradução acelera 3–5× vs CPU.

---

Para a lista completa de modelos e tamanhos, veja [MODELS.md](MODELS.md). Para requisitos de máquina, veja [INSTALL.md](INSTALL.md).
