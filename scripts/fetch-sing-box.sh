#!/usr/bin/env bash
# =============================================================================
# fetch-sing-box.sh — положить бинарь sing-box в src-tauri/binaries/ как
# Tauri-sidecar (externalBin), чтобы он встроился в приложение и пользователю
# НЕ нужно было ставить sing-box самому (brew/вручную).
#
# Имя файла должно быть `sing-box-<rust-target-triple>[.exe]` — Tauri сам
# подставит нужный триплет при сборке.
#
# Запускается автоматически из beforeBuildCommand (см. tauri.conf.json), так что
# любая сборка (`npm run tauri build`, `npm run build:mac`, CI) получает бинарь.
#
# Использование:
#   scripts/fetch-sing-box.sh [rust-target-triple]   # по умолчанию — триплет хоста
# =============================================================================
set -euo pipefail

# Версия sing-box, которую вшиваем (совпадает с проверенной у пользователя).
SB_VERSION="${SB_VERSION:-1.13.12}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST_DIR="$ROOT/src-tauri/binaries"
mkdir -p "$DEST_DIR"

# Триплет: аргумент или host (host == target на каждом CI-раннере).
TRIPLE="${1:-$(rustc -vV | sed -n 's/^host: //p')}"

case "$TRIPLE" in
  aarch64-apple-darwin)      OSARCH="darwin-arm64";  ARCHIVE="tar.gz"; INNER="sing-box";     OUT="sing-box-$TRIPLE" ;;
  x86_64-apple-darwin)       OSARCH="darwin-amd64";  ARCHIVE="tar.gz"; INNER="sing-box";     OUT="sing-box-$TRIPLE" ;;
  x86_64-pc-windows-msvc)    OSARCH="windows-amd64"; ARCHIVE="zip";    INNER="sing-box.exe"; OUT="sing-box-$TRIPLE.exe" ;;
  aarch64-pc-windows-msvc)   OSARCH="windows-arm64"; ARCHIVE="zip";    INNER="sing-box.exe"; OUT="sing-box-$TRIPLE.exe" ;;
  x86_64-unknown-linux-gnu)  OSARCH="linux-amd64";   ARCHIVE="tar.gz"; INNER="sing-box";     OUT="sing-box-$TRIPLE" ;;
  aarch64-unknown-linux-gnu) OSARCH="linux-arm64";   ARCHIVE="tar.gz"; INNER="sing-box";     OUT="sing-box-$TRIPLE" ;;
  *) echo "✗ fetch-sing-box: неизвестный триплет '$TRIPLE'"; exit 1 ;;
esac

DEST="$DEST_DIR/$OUT"
if [[ -f "$DEST" ]]; then
  echo "▸ sing-box уже на месте: $DEST"
  exit 0
fi

# Быстрый путь: если собираем под хост и sing-box уже установлен локально —
# копируем его (ровно та версия, что у пользователя), без скачивания.
HOST="$(rustc -vV | sed -n 's/^host: //p')"
if [[ "$TRIPLE" == "$HOST" ]] && command -v sing-box >/dev/null 2>&1; then
  echo "▸ Беру локальный sing-box ($(command -v sing-box))"
  cp "$(command -v sing-box)" "$DEST"
  chmod 755 "$DEST"                       # writable — Tauri's bundler runs xattr on it
  xattr -c "$DEST" 2>/dev/null || true    # strip quarantine/provenance attrs
  echo "✓ $DEST"
  exit 0
fi

URL="https://github.com/SagerNet/sing-box/releases/download/v${SB_VERSION}/sing-box-${SB_VERSION}-${OSARCH}.${ARCHIVE}"
echo "▸ Скачиваю sing-box ${SB_VERSION} (${OSARCH})…"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
curl -fsSL "$URL" -o "$TMP/sb.$ARCHIVE"
# Распаковка: .tar.gz через tar; .zip — git-bash GNU tar НЕ умеет zip, поэтому
# на Windows распаковываем через PowerShell Expand-Archive (с конвертацией путей).
if [[ "$ARCHIVE" == "zip" ]]; then
  if command -v unzip >/dev/null 2>&1; then
    unzip -q "$TMP/sb.$ARCHIVE" -d "$TMP"
  elif command -v powershell >/dev/null 2>&1; then
    win_src="$(cygpath -w "$TMP/sb.$ARCHIVE" 2>/dev/null || echo "$TMP/sb.$ARCHIVE")"
    win_dst="$(cygpath -w "$TMP" 2>/dev/null || echo "$TMP")"
    powershell -NoProfile -Command "Expand-Archive -Force -LiteralPath '$win_src' -DestinationPath '$win_dst'"
  else
    tar -xf "$TMP/sb.$ARCHIVE" -C "$TMP"
  fi
else
  tar -xzf "$TMP/sb.$ARCHIVE" -C "$TMP"
fi
SRC="$(find "$TMP" -type f -name "$INNER" | head -1)"
if [[ -z "$SRC" ]]; then
  echo "✗ не нашёл $INNER внутри архива"; exit 1
fi
cp "$SRC" "$DEST"
chmod 755 "$DEST"                       # writable — Tauri's bundler runs xattr on it
xattr -c "$DEST" 2>/dev/null || true    # strip quarantine/provenance attrs
echo "✓ $DEST"
