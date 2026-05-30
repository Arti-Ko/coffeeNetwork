# Themed app icon (macOS 26 Liquid Glass)

Чтобы иконка coffeeNetwork **адаптировалась под Light / Dark / Tinted / Clear**
режимы macOS 26 (как системные приложения), нужен слоёный формат `.icon`.
Tauri пока его не поддерживает напрямую, поэтому собираем иконку в **Icon
Composer** и встраиваем в готовый `.app` скриптом.

## Что уже готово

- `background.png` — фоновый слой (тёплый espresso → amber градиент, 1024×1024).
- `glyph.png` — передний слой: белый ромб с разрезом на **прозрачном** фоне.
  Прозрачность критична: из альфы этого слоя система строит тёмную и
  тонированную версии.

## Шаги (5 минут, один раз)

1. Открой **Icon Composer**
   (`/Applications/Xcode.app/Contents/Applications/Icon Composer.app`
   — или Spotlight → «Icon Composer»).
2. New → перетащи `background.png` как нижний слой, `glyph.png` — поверх.
3. Слева включи группы вариантов **Default / Dark / Mono (Tinted)** — Icon
   Composer покажет превью. При желании подкрути «Specular / Blur / Shadow»
   для эффекта жидкого стекла.
4. **File → Export → coffeeNetwork.icon** и сохрани прямо в эту папку
   (`icon-src/coffeeNetwork.icon`).

## Встроить в приложение

```bash
npm run tauri build                 # сначала собрать .app
scripts/apply-macos-icon.sh         # встроит themed-иконку и переподпишет
```

Скрипт скомпилирует `.icon` через `actool`, положит `Assets.car` в
`Contents/Resources`, пропишет `CFBundleIconName=AppIcon` в `Info.plist` и
обновит кеш иконок.

> Если в Dock/Finder осталась старая иконка — `killall Finder Dock` или
> перелогинься.
