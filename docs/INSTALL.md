# Instalação do LegendAI

> Guia de instalação por sistema operacional. O app gera legendas 100% local — **nenhum modelo vem no instalador**; você baixa os recomendados na primeira execução (primeiros-passos).

## Requisitos de sistema

O app detecta seu hardware e recomenda modelos compatíveis (Tier). Requisito mínimo absoluto: **CPU de 64 bits**, **4 GB de RAM** e **~1,5 GB de espaço livre em disco** (só app + ffmpeg; modelos baixam sob demanda).

| Tier  | Hardware             | Uso recomendado                  | Tradução padrão (GPU acelera)              |
| ----- | -------------------- | -------------------------------- | ------------------------------------------ |
| **1** | 4–5 GB RAM, CPU      | Máquinas fracas / legenda apenas | NLLB-200 600M (rápido, CUDA se houver GPU) |
| **2** | 6–15 GB RAM, CPU/GPU | Qualidade boa com contexto       | TowerInstruct 7B q4 (256 layers na GPU)    |
| **3** | 16 GB+ ou GPU        | Qualidade máxima                 | TowerInstruct 7B q6 (512 layers na GPU)    |

> RAM abaixo de 4 GB: o app pode rodar com `Whisper Tiny`, mas a tradução via LLM fica apertada — prefira o modelo NLLB pequeno (ver [MODELS.md](MODELS.md)).

Espaço em disco para modelos (um STT + um de tradução):

| Combinação típica                                      | Espaço  |
| ------------------------------------------------------ | ------- |
| Tier 1: Whisper Small (190 MB) + NLLB q4 (1,3 GB)      | ~1,5 GB |
| Tier 2: Whisper Large (1,1 GB) + Tower 7B q4 (3,8 GB)  | ~4,9 GB |
| Tier 3: Whisper Large (1,1 GB) + Tower 7B q6 (5,15 GB) | ~6,2 GB |

---

## Windows

1. Baixe o instalador **`.exe` (NSIS)** do release mais recente.
2. Execute o arquivo. O instalador **não exige administrador** (instalação por usuário, `currentUser`).
3. Siga os passos do assistente (idioma do instalador: inglês ou português brasileiro).
4. Abra o LegendAI pelo menu Iniciar. Na primeira execução, o app abre os **primeiros-passos** e baixa os modelos recomendados.

### Verificação pós-instalação

- ffmpeg/ffprobe e o runtime ONNX (`onnxruntime.dll`) são embutidos no instalador — nada a configurar.
- Para diagnóstico, use o arquivo de log (tabela em "Onde ficam os arquivos") ou rode `legendai --smoke-test` (instalação por usuário: `%LOCALAPPDATA%\Programs\LegendAI\legendai.exe --smoke-test`).

### Desinstalar

Windows → Configurações → Apps → LegendAI → Desinstalar. Modelos baixados ficam em `%LOCALAPPDATA%\legendai\models\` (remova manualmente se quiser liberar espaço).

---

## macOS

1. Baixe o **`.dmg`** correspondente à sua arquitetura: `arm64` (Apple Silicon) ou `amd64` (Intel).
2. Abra o `.dmg` e arraste o **LegendAI** para a pasta _Aplicativos_.
3. **Primeira execução:** os builds atuais usam assinatura ad-hoc. Se o Gatekeeper bloquear ("não é de um desenvolvedor verificado"), clique com o botão direito no app → **Abrir** → **Abrir**.
4. Na primeira execução, o app abre os **primeiros-passos** e baixa os modelos recomendados.

### Nota sobre assinatura

Assinatura pública (Developer ID notarizada) está no roadmap — releases atuais são para distribuição informal. Para remover o aviso do Gatekeeper: `xattr -dr com.apple.quarantine "/Applications/LegendAI.app"`.

### Desinstalar

Arraste o LegendAI da pasta _Aplicativos_ para a Lixeira. Modelos ficam em `~/Library/Caches/legendai/models/`.

---

## Linux

Distribuições suportadas: **Ubuntu 22.04 LTS e mais novas** (glibc ≥ 2.35; o AppImage é o formato universal, o `.deb` cobre Debian/Ubuntu). Duas opções:

### Opção A — AppImage (universal)

1. Baixe o **`.AppImage`** do release.
2. Torne executável e rode:

   ```bash
   chmod +x LegendAI_*.AppImage
   ./LegendAI_*.AppImage
   ```

3. **Dependência de sistema:** o AppImage usa o WebKitGTK do sistema. Instale se faltar:

   ```bash
   sudo apt install libwebkit2gtk-4.1-0
   ```

### Opção B — `.deb` (Debian/Ubuntu)

```bash
sudo dpkg -i LegendAI_*.deb
sudo apt-get install -f   # resolve dependências, se necessário
```

Instala o **desktop entry** (menu de aplicativos) e o binário `legendai`. Se o WebKitGTK 4.1 não estiver presente:

```bash
sudo apt install libwebkit2gtk-4.1-0
```

### Verificação

```bash
which legendai          # Opção B (deb)
legendai --version
```

### Desinstalar

```bash
sudo apt remove legendai   # deb
rm LegendAI_*.AppImage     # AppImage (remova o arquivo)
```

Modelos ficam em `~/.cache/legendai/models/`.

---

## Pós-instalação (todos os OS)

1. **Primeiros-passos:** o app detecta hardware, mostra seu Tier e o par de modelos recomendado (STT + tradução). Use **"Baixar recomendados"**.
2. **Ativar modelos:** aba **Modelos** → botão **Ativar** no STT e na tradução escolhidos. O pipeline usa o modelo ativo.
3. **Importar:** aba **Importar** → arraste um vídeo (ou selecione) → escolha trilha de áudio (ou legenda embutida) → **Continuar** → **Processar**.

> Os modelos são baixados do [Hugging Face](https://huggingface.co) (catálogo curado em [`catalog/models.json`](../catalog/models.json)). Após o download, o app funciona **100% offline**.

### Onde ficam os arquivos

| Dado    | Linux                       | macOS                                     | Windows                           |
| ------- | --------------------------- | ----------------------------------------- | --------------------------------- |
| Config  | `~/.config/legendai/`       | `~/Library/Application Support/legendai/` | `%APPDATA%\legendai\`             |
| Modelos | `~/.cache/legendai/models/` | `~/Library/Caches/legendai/models/`       | `%LOCALAPPDATA%\legendai\models\` |
| Logs    | `~/.local/state/legendai/`  | `~/Library/Application Support/legendai/` | `%LOCALAPPDATA%\legendai\`        |

---

## Solução de problemas

- **Janela em branco em displays sem GPU/GBM (Linux):** rodar com rendering por software — `WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1 ./LegendAI_*.AppImage`.
- **AppImage não abre / "permission denied":** `chmod +x` no arquivo.
- **Tradução lenta:** modelos LLM sem GPU são mais lentos que NLLB — com GPU NVIDIA, recompile/instale com `--features cuda` para usar a placa (placa fica ociosa sem isso). Em máquina fraca prefira Tier 1 (ver [MODELS.md](MODELS.md)).
- **Erro de download de modelo:** verifique a conexão — o app só precisa de rede na primeira vez (download de modelos).

Para reportar um problema, inclua o trecho do **arquivo de log** (tabela acima) — isso acelera muito o diagnóstico.

---

## Smoke test manual (checklist)

O CI roda o smoke test automaticamente (`.github/workflows/smoke.yml`) para **Linux (deb)**, **Windows (NSIS)** e **macOS arm64 (DMG)**. Para macOS amd64 e para validação em máquina real (ex: antes de pedir apoio em uma issue), o mesmo check roda manualmente:

1. **Instale o instalador normalmente** (passos da seção do seu OS) — ou use o já instalado.
2. **Rode o smoke test** — verifica, sem rede e sem abrir a janela: (a) o sidecar `ffmpeg` responde a `-version`; (b) a config carrega; (c) o onboarding calcula hardware/tier/recomendações. Sai com `0` = tudo ok.

   ```bash
   # Linux (deb)
   legendai --smoke-test
   # macOS (app instalado em /Applications)
   /Applications/LegendAI.app/Contents/MacOS/legendai --smoke-test
   # Windows (PowerShell; instalação por usuário)
   & "$env:LOCALAPPDATA\Programs\LegendAI\legendai.exe" --smoke-test
   ```

3. **Esperado:** 3 linhas `[ok]` (`ffmpeg sidecar`, `config`, `onboarding`) e `smoke test: PASS` na saída, exit code `0`. Qualquer `[FAIL]` indica instalação corrompida — cole a saída inteira na issue.
4. **Sem rede:** o check não baixa nada; rode com a internet desligada se quiser confirmar.

> O smoke test não substitui o primeiro boot real (que valida o webview/onboarding visual) — ele confirma a **instalação** do binário + sidecars.

---

## Checklist de revisão (docs)

Cada release deve revisar os docs contra o produto real antes de publicar:

- [ ] **INSTALL.md:** passos de instalação conferidos em máquina limpa por OS (Linux em container Ubuntu LTS; Windows/macOS no runner do CI — checklist manual na seção "Smoke test manual" deste arquivo).
- [ ] **INSTALL.md:** caminhos de config/cache/log batem com os desta versão (`src-tauri/src/config.rs`, `logging.rs`, `model_manager/cache.rs`).
- [ ] **MODELS.md:** tabelas refletem [`catalog/models.json`](../catalog/models.json) (nomes, tamanhos, RAM mín., qualidade/velocidade).
- [ ] **MODELS.md:** recomendações por Tier batem com `src-tauri/src/hardware/tier.rs` e `model_manager/recommend.rs`.
- [ ] **TRANSLATION.md:** NLLB vs Tower condizente com o catálogo e com o ADR-001; notas de "só Tower" (glossário/formalidade) conferem com a implementação.
- [ ] **README.md:** link dos releases aponta para o remote real; seção de instalação aponta para os 3 docs.
