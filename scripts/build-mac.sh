#!/usr/bin/env bash
# =============================================================================
# build-mac.sh — собрать coffeeNetwork под macOS так, чтобы Apple Silicon (M-чипы)
# НЕ ругался («приложение повреждено» / "killed: 9") и НЕ требовалось вручную
# вводить `xattr -cr` в терминале.
#
# Почему это нужно: на Apple Silicon любой бинарник обязан иметь валидную подпись
# (хотя бы ad-hoc), иначе ядро убивает процесс. Платного Apple Developer ID для
# нотаризации нет, поэтому подписываем ad-hoc (`-`) — этого достаточно, чтобы
# локально собранное приложение запускалось двойным кликом.
#
# По умолчанию собираем ТОЛЬКО .app (`--bundles app`): это быстро и надёжно.
# Сборку .dmg (`bundle_dmg.sh` через AppleScript/Finder) намеренно пропускаем —
# она флакает при повторных прогонах, а для раздачи .dmg всё равно собирает CI
# (.github/workflows/release.yml на GitHub-раннере). Локально .dmg можно
# принудительно собрать флагом --dmg.
#
# Что делает скрипт:
#   1. npm run tauri build --bundles app  (Tauri сам ad-hoc-подписывает)
#   2. переподписывает .app целиком (--deep --force) на всякий случай
#   3. снимает атрибут карантина com.apple.quarantine
#   4. проверяет подпись и печатает путь к готовому .app
#
# Использование:
#   scripts/build-mac.sh            # собрать .app (быстро, надёжно)
#   scripts/build-mac.sh --open     # + открыть готовый .app
#   scripts/build-mac.sh --dmg      # + собрать .dmg и updater-артефакты
# =============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

OPEN_AFTER="false"
WITH_DMG="false"
for arg in "$@"; do
  case "$arg" in
    --open) OPEN_AFTER="true" ;;
    --dmg)  WITH_DMG="true" ;;
  esac
done

# Чистим зависшие dmg-тома/образы от прошлых упавших сборок — иначе bundle_dmg.sh
# падает с "device busy". (Tauri монтирует временный rw.*.dmg как том dmg.XXXX.)
echo "▸ Чищу зависшие dmg-тома от прошлых сборок…"
hdiutil info 2>/dev/null | awk '/image-path.*rw\.[0-9]+\..*\.dmg/{found=1} /\/dev\/disk/ && found {print $1; found=0}' \
  | while read -r dev; do hdiutil detach "$dev" -force >/dev/null 2>&1 && echo "    отмонтирован $dev"; done
for v in /Volumes/dmg.*; do [ -e "$v" ] && hdiutil detach "$v" -force >/dev/null 2>&1 && echo "    отмонтирован $v"; done
rm -f "$ROOT"/src-tauri/target/*/release/bundle/macos/rw.*.dmg \
      "$ROOT"/src-tauri/target/release/bundle/macos/rw.*.dmg 2>/dev/null || true

BUILD_CMD=(npm run tauri -- build)
if [[ "$WITH_DMG" == "true" ]]; then
  # Полная сборка (.app + .dmg + updater-артефакты). Updater-артефакты требуют
  # minisign-ключ — берём приватный ключ из ~/.coffeeNetwork-updater.key.
  KEY_FILE="$HOME/.coffeeNetwork-updater.key"
  if [[ -f "$KEY_FILE" ]]; then
    export TAURI_SIGNING_PRIVATE_KEY="$KEY_FILE"
    export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
    echo "▸ Updater-ключ найден — подпишу updater-артефакты"
    BUILD_CMD+=(--bundles app,dmg,updater)
  else
    echo "▸ Updater-ключ не найден — собираю .app + .dmg без updater-артефактов"
    BUILD_CMD+=(--bundles app,dmg)
  fi
else
  # Быстрая надёжная сборка: только подписанный .app (без флаки dmg).
  BUILD_CMD+=(--bundles app)
fi

echo "▸ Собираю приложение (${BUILD_CMD[*]})…"
"${BUILD_CMD[@]}"

# Находим собранный бандл (поддерживаем и target/release, и target/aarch64-…).
APP="$(/usr/bin/find src-tauri/target -type d -name 'coffeeNetwork.app' -path '*/bundle/macos/*' 2>/dev/null | head -1)"
if [[ -z "$APP" || ! -d "$APP" ]]; then
  echo "✗ Не нашёл собранный coffeeNetwork.app в src-tauri/target/*/bundle/macos/"
  exit 1
fi
echo "▸ Бандл: $APP"

# Ad-hoc переподпись всего бандла (вложенные .dylib/бинарники тоже).
echo "▸ Переподписываю (ad-hoc, --deep)…"
codesign --force --deep --sign - "$APP"

# Снимаем карантин, чтобы локально собранное приложение открывалось без вопросов.
echo "▸ Снимаю карантин с .app…"
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true

# То же для .dmg, если он собран.
if [[ "$WITH_DMG" == "true" ]]; then
  DMG="$(/usr/bin/find src-tauri/target -type f -name '*.dmg' -path '*/bundle/dmg/*' 2>/dev/null | head -1)"
  if [[ -n "$DMG" && -f "$DMG" ]]; then
    echo "▸ Снимаю карантин с .dmg: $DMG"
    xattr -dr com.apple.quarantine "$DMG" 2>/dev/null || true
  fi
fi

# Проверки.
echo "▸ Проверка подписи:"
codesign --verify --verbose=2 "$APP" 2>&1 | sed 's/^/    /' || true
echo "▸ Атрибуты бандла (карантина быть не должно):"
xattr "$APP" 2>/dev/null | sed 's/^/    /' || echo "    (нет расширенных атрибутов — это хорошо)"

echo ""
echo "✓ Готово. Приложение запускается двойным кликом, терминал не нужен:"
echo "    $APP"
[[ -n "${DMG:-}" ]] && echo "    DMG: $DMG"

if [[ "$OPEN_AFTER" == "true" ]]; then
  echo "▸ Открываю приложение…"
  open "$APP"
fi
