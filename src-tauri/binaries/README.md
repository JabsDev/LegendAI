# Binários sidecar ffmpeg/ffprobe

Os arquivos `ffmpeg-<target-triple>` e `ffprobe-<target-triple>` nesta pasta são
**downloads** (ver ADR-003) e **não são versionados** — são baixados pelo
workflow `fetch-binaries.yml` (a partir da Fase 6) e, em dev, pelo script
`scripts/fetch-ffmpeg.sh` localmente.

## Subpasta `native/` (DLLs de runtime do Windows)

A subpasta `native/` é um **diretório de staging** usado pelo workflow de release
(`.github/workflows/release.yml`, job `windows`) para embarcar as DLLs de runtime
do app no instalador NSIS (`onnxruntime.dll`, `DirectML.dll`, `llama.dll`,
`ggml-*.dll`, etc.). Não é versionada e não deve conter arquivos commitados.

Motivo: as DLLs do onnxruntime são criadas pelos build scripts como **symlinks**
para `$CARGO_HOME/registry` em `target/release`. Fazer o `bundle.resources` do
Tauri apontar diretamente para elas via glob em `target/release` faz o
`tauri-build` executar `fs::copy(src=destino do symlink, dst=o próprio symlink)`,
que falha no Windows com `os error 32` (sharing violation). O workflow copia as
DLLs seguindo o symlink (`cp -L`) para `native/` como arquivos reais e o
`tauri.windows.conf.json` referencia `binaries/native` (diretório) como recurso.

## Origem e versão

- **Linux/Windows (BtbN):** builds estáticos do [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds)
  (release line `ffmpeg-n9.0`, arquivos `ffmpeg-n9.0-latest-linux64-gpl-9.0.tar.xz` e
  equivalentes para os demais triples).
- **macOS (Martin Riedl):** o BtbN não publica builds para darwin. Usamos os builds
  estáticos de [ffmpeg.martin-riedl.de](https://ffmpeg.martin-riedl.de) via
  `redirect/latest/macos/{arm64,amd64}/snapshot/{ffmpeg,ffprobe}.zip` (assinadas e
  notarizadas; um zip por binário, binário na raiz).
- **Versão atual (linux x86_64):** `ffmpeg version n9.0.1-6-...` (build de 2026-08-18).

## Licença e redistribuição

Os binários do BtbN são compilados com `--enable-gpl --enable-version3`
(GPLv3+ para componentes gpl/version3 como libx264, libx265, libvpx).
Ao redistribuir estes binários dentro do instalador do LegendAI, o app passa a
ser um trabalho combinado distribuído — as obrigações da **GPLv3** (e LGPL/BSD
dos demais componentes) se aplicam aos binários redistribuídos.

Consequências práticas para o LegendAI (licença MIT do app):

- O código-fonte do LegendAI permanece MIT; apenas os binários ffmpeg/ffprobe
  embutidos carregam as condições GPL.
- Manter a nota de licença acima junto ao instalador/release e apontar para o
  repositório BtbN (fonte dos binários).
- Alternativa a avaliar na Fase 6 (ADR-003): builds LGPL (sem libx264/libx265)
  reduzem a carga de licença — decidir na tarefa 6.3.
