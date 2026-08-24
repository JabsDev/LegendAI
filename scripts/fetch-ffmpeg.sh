#!/usr/bin/env bash
# Baixa binários estáticos ffmpeg/ffprobe e os instala em src-tauri/binaries/
# com o sufixo de target triple exigido pelo Tauri (bundle.externalBin / ADR-003).
#
# Uso: scripts/fetch-ffmpeg.sh [linux64|win64|winarm64|macos-arm64|macos-amd64]
#   (sem argumento: usa o triple da máquina atual)
#
# Fontes:
#   linux64 / win64 / winarm64 : BtbN/FFmpeg-Builds (GPLv3) — ver src-tauri/binaries/README.md
#   macos-*        : https://ffmpeg.martin-riedl.de — builds estáticos assinadas
#                    para darwin arm64 e amd64 (o BtbN não publica darwin;
#                    fonte definida na tarefa 6.2).
# Compatível com bash 3.2 (macOS) — sem associative arrays.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/src-tauri/binaries"
mkdir -p "$DEST"

get_triple() {
  case "$1" in
    linux64) echo "x86_64-unknown-linux-gnu" ;;
    win64) echo "x86_64-pc-windows-msvc" ;;
    winarm64) echo "aarch64-pc-windows-msvc" ;;
    macos-arm64) echo "aarch64-apple-darwin" ;;
    macos-amd64) echo "x86_64-apple-darwin" ;;
    *) echo "" ;;
  esac
}

HOST_OS="$(uname -s)"
case "$HOST_OS" in
  Linux*) DEFAULT=linux64 ;;
  MINGW*|MSYS*|CYGWIN*) DEFAULT=win64 ;;
  Darwin*) DEFAULT=macos-arm64 ;;
  *) echo "Erro: arquitetura $HOST_OS não suportada pelo script." >&2; exit 1 ;;
esac

PLAT="${1:-$DEFAULT}"
TRIP="$(get_triple "$PLAT")"
[ -z "$TRIP" ] && { echo "Erro: plataforma desconhecida '$PLAT' (use linux64|win64|winarm64|macos-arm64|macos-amd64)." >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fetch_btbn() {
  # BtbN: um arquivo (tar.xz/zip) contendo ffmpeg + ffprobe.
  local BASE="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n9.0-latest-${PLAT}-gpl-9.0"
  local EXT="tar.xz"; case "$PLAT" in win*) EXT="zip" ;; esac
  echo "Baixando $BASE.$EXT ..."
  curl -L --retry 2 -o "$TMP/ffmpeg.$EXT" "$BASE.$EXT"
  case "$EXT" in
    tar.xz) tar xf "$TMP/ffmpeg.tar.xz" -C "$TMP" ;;
    zip) unzip -q "$TMP/ffmpeg.zip" -d "$TMP" ;;
  esac
  # Windows (win64/winarm64) publica .exe (o Tauri exige o sufixo `.exe` no sidecar)
  local BINEXT=""; case "$PLAT" in win*) BINEXT=".exe" ;; esac
  local BIN
  BIN="$(find "$TMP" -type f \( -name "ffmpeg$BINEXT" -o -name "ffprobe$BINEXT" \))"
  [ -n "$BIN" ] || { echo "Erro: binários não encontrados no arquivo." >&2; exit 1; }
  for name in ffmpeg ffprobe; do
    local src
    src="$(echo "$BIN" | grep -m1 "/$name$BINEXT$")"
    install -m 0755 "$src" "$DEST/$name-$TRIP$BINEXT"
  done
}

fetch_martin() {
  # Martin Riedl: um zip por binário, com o binário na raiz do zip.
  local arch="${PLAT#macos-}"   # arm64 | amd64
  for name in ffmpeg ffprobe; do
    local url="https://ffmpeg.martin-riedl.de/redirect/latest/macos/${arch}/snapshot/${name}.zip"
    echo "Baixando $url ..."
    curl -L --retry 2 -o "$TMP/$name.zip" "$url"
    local entry
    entry="$(unzip -Z1 "$TMP/$name.zip")"
    [ -n "$entry" ] || { echo "Erro: $name.zip vazio." >&2; exit 1; }
    unzip -o -q "$TMP/$name.zip" -d "$TMP/$name.dir"
    install -m 0755 "$TMP/$name.dir/$entry" "$DEST/$name-$TRIP"
  done
}

case "$PLAT" in
  macos-*) fetch_martin ;;
  *)       fetch_btbn ;;
esac

echo "Instalado em $DEST/:"
ls -lh "$DEST"
