#!/usr/bin/env bash
# =============================================================================
# apply-macos-icon.sh — embed a macOS 26 themed .icon into the built .app.
#
# Tauri does not yet support the Liquid Glass `.icon` format directly, so this
# is a post-build step: it compiles `coffeeNetwork.icon` (made in Icon Composer)
# into an `Assets.car` via `actool`, drops it into the app bundle's Resources,
# sets `CFBundleIconName` in Info.plist, re-signs, and refreshes the icon cache.
#
# Result: the app icon adapts to Light / Dark / Tinted / Clear like 1st-party
# macOS apps.
#
# Usage:
#   scripts/apply-macos-icon.sh [path/to/coffeeNetwork.app]
#
# Prereqs:
#   - macOS 26+ with Xcode 26 (provides actool + Icon Composer)
#   - icon-src/coffeeNetwork.icon  (export it from Icon Composer — see README)
# =============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ICON_SRC="$ROOT/icon-src/coffeeNetwork.icon"
APP="${1:-$ROOT/src-tauri/target/release/bundle/macos/coffeeNetwork.app}"
ICON_NAME="AppIcon"

if [[ ! -d "$ICON_SRC" ]]; then
  echo "✗ Не найден $ICON_SRC"
  echo "  Собери его в Icon Composer из icon-src/background.png + icon-src/glyph.png"
  echo "  и экспортируй как coffeeNetwork.icon в папку icon-src/ (см. README)."
  exit 1
fi
if [[ ! -d "$APP" ]]; then
  echo "✗ Не найден бандл приложения: $APP"
  echo "  Сначала собери: npm run tauri build"
  exit 1
fi

ACTOOL="$(xcrun --find actool)"
RES="$APP/Contents/Resources"
PLIST="$APP/Contents/Info.plist"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# 1. Wrap the .icon in a temporary asset catalog and compile it.
CATALOG="$TMP/Assets.xcassets"
mkdir -p "$CATALOG"
cp -R "$ICON_SRC" "$CATALOG/$ICON_NAME.icon"

echo "▸ Компилирую иконку через actool…"
"$ACTOOL" "$CATALOG" \
  --compile "$RES" \
  --app-icon "$ICON_NAME" \
  --output-partial-info-plist "$TMP/partial.plist" \
  --platform macosx \
  --minimum-deployment-target 26.0 \
  --output-format human-readable-text >/dev/null

# 2. Point Info.plist at the compiled icon (CFBundleIconName), keep legacy too.
echo "▸ Прописываю CFBundleIconName=$ICON_NAME в Info.plist…"
/usr/libexec/PlistBuddy -c "Delete :CFBundleIconName" "$PLIST" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :CFBundleIconName string $ICON_NAME" "$PLIST"

# 3. Re-sign (ad-hoc) so the modified bundle stays launchable.
echo "▸ Переподписываю бандл…"
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || \
  echo "  (предупреждение: codesign не прошёл — приложение всё равно запустится локально)"

# 4. Nudge the icon cache so Finder/Dock pick it up.
touch "$APP"
echo "✓ Готово. Themed-иконка встроена в:"
echo "  $APP"
echo "  Если в Dock/Finder ещё старая — выйди/зайди или: killall Finder Dock"
