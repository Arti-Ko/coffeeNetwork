# coffeeNetwork — Living Documentation

Живой документ по обоим репозиториям семейства coffeeNetwork.
Обновляется при каждом значимом изменении кода или архитектуры.

---

## Оглавление

1. [Обзор семейства](#1-обзор-семейства)
2. [Desktop (coffeeNetwork)](#2-desktop-coffeenetwork)
   - [Стек и структура файлов](#21-стек-и-структура-файлов)
   - [Архитектура и потоки данных](#22-архитектура-и-потоки-данных)
   - [Ключевые модули](#23-ключевые-модули)
   - [Хранилище данных](#24-хранилище-данных)
   - [Протоколы и парсинг ссылок](#25-протоколы-и-парсинг-ссылок)
   - [Логика роутинга (sing-box)](#26-логика-роутинга-sing-box)
   - [Жизненный цикл процесса sing-box](#27-жизненный-цикл-процесса-sing-box)
   - [Меню-бар (трей)](#28-меню-бар-трей)
   - [Автообновление](#29-автообновление)
   - [Сборка и релиз](#210-сборка-и-релиз)
3. [Android (coffeeNetwork-android)](#3-android-coffeenetwork-android)
   - [Стек и структура файлов](#31-стек-и-структура-файлов)
   - [Архитектура Flutter ↔ Android ↔ libbox](#32-архитектура-flutter--android--libbox)
   - [UI: экраны и компоненты](#33-ui-экраны-и-компоненты)
   - [VPN-сервис (CoffeeVpnService)](#34-vpn-сервис-coffeevpnservice)
   - [Quick Settings Tile](#35-quick-settings-tile)
   - [Per-app исключения (ИГНОР)](#36-per-app-исключения-игнор)
   - [Автообновление APK](#37-автообновление-apk)
   - [Темы и акцентный цвет](#38-темы-и-акцентный-цвет)
   - [Сборка и подпись](#39-сборка-и-подпись)
4. [Общее: логика роутинга RU-BYPASS](#4-общее-логика-роутинга-ru-bypass)
5. [История изменений (changelog)](#5-история-изменений-changelog)
6. [Известные особенности и нюансы](#6-известные-особенности-и-нюансы)
7. [Дальнейшая работа (задачи)](#7-дальнейшая-работа-задачи)

---

## 1. Обзор семейства

| Репозиторий | Платформа | Технология | Версия |
|---|---|---|---|
| `coffeeNetwork` | macOS / Windows / Linux | Tauri 2 + Rust + Vanilla TS | 0.2.3 |
| `coffeeNetwork-android` | Android | Flutter + Kotlin + sing-box libbox | 0.2.3 |
| `NetForge` *(отдельный репо)* | CLI / VPS | — | — |

> Начиная с 0.2.3 версии синхронизированы между Desktop и Android и двигаются вместе.

**Общая идея**: личный VPN-клиент на базе [sing-box](https://sing-box.sagernet.org/) с умным сплит-туннелингом — российские домены (`geosite-category-ru`) и IP (`geoip-ru`) идут напрямую, весь остальной трафик — через VPN. Одним тумблером RU-BYPASS.

**Дизайн-метафора**: «посадочный талон» (boarding pass) — крупная монохромная типографика, настраиваемый акцентный цвет, светлая и тёмная темы.

---

## 2. Desktop (coffeeNetwork)

Репозиторий: `/Users/purrweb/code/coffeeNetwork`

### 2.1 Стек и структура файлов

```
coffeeNetwork/
├── src/
│   ├── main.ts          # Весь frontend UI (vanilla TypeScript + Vite)
│   ├── styles.css       # Liquid glass дизайн, CSS-переменные
│   └── index.html
├── src-tauri/src/
│   ├── lib.rs           # Tauri-команды + трей + точка входа
│   ├── parser.rs        # Парсеры share-ссылок → sing-box outbound JSON
│   ├── singbox.rs       # Генерация конфига + поиск бинаря sing-box
│   ├── core.rs          # Жизненный цикл процесса sing-box (start/stop/status)
│   ├── store.rs         # JSON-хранилище серверов и настроек
│   ├── sysproxy.rs      # Аварийный сброс системного прокси
│   └── main.rs          # Точка входа (вызывает lib::run)
├── scripts/
│   ├── fetch-sing-box.sh  # Скачивает sing-box бинарь для sidecar
│   └── build-mac.sh       # Сборка .app + ad-hoc подпись + снятие карантина
└── src-tauri/tauri.conf.json
```

**Стек:**
- **Tauri 2** — нативная оболочка (WebView фронтенд + Rust бэкенд)
- **Rust** — весь бэкенд: парсинг, генерация конфига, управление процессом sing-box
- **Vanilla TypeScript + Vite** — фронтенд (без фреймворков)
- **sing-box** как sidecar-бинарь (встроен в приложение скриптом `fetch-sing-box.sh`)

### 2.2 Архитектура и потоки данных

```
┌──────────────────────────────────────────────────────────┐
│  WebView (Vanilla TS)                                    │
│  main.ts: UI, вызовы invoke(), событийная модель         │
└───────────────────────┬──────────────────────────────────┘
                        │ invoke("command", args)
                        ▼
┌──────────────────────────────────────────────────────────┐
│  Rust Backend (lib.rs)                                   │
│  Tauri-команды: list_servers, add_links, connect,        │
│  disconnect, status, traffic, set_appearance...          │
└──┬──────────────┬──────────────┬────────────────────────┘
   │              │              │
   ▼              ▼              ▼
parser.rs     singbox.rs     store.rs
(парсинг)   (генерация      (JSON на диск)
            конфига)
                  │
                  ▼
              core.rs
         (spawn sing-box process)
                  │
                  ▼
         ┌────────────────┐
         │  sing-box      │
         │  (TUN / proxy) │
         └────────────────┘
                  │
                  ▼
         clash_api :19099
         (трафик, статус)
```

### 2.3 Ключевые модули

#### `lib.rs` — Tauri command layer

| Команда | Что делает |
|---|---|
| `list_servers()` | Загружает список серверов из JSON |
| `add_links(text)` | Парсит одну или несколько share-ссылок или base64-подписку, добавляет в список |
| `delete_server(id)` | Удаляет сервер (останавливает VPN если он активен) |
| `rename_server(id, name)` | Переименовывает сервер |
| `connect(id)` | Подключается к серверу: генерирует конфиг → запускает sing-box → обновляет трей |
| `disconnect()` | Останавливает sing-box, сбрасывает системный прокси |
| `status()` | Возвращает `{running, active_server, mode, bypass_ru, core_path}` |
| `get_settings()` / `set_settings()` | Чтение/запись режима и bypass_ru |
| `set_appearance(accent, accent2, theme)` | Сохраняет акцент и тему |
| `list_apps()` | Перечисляет установленные приложения (для per-app split tunnel) |
| `set_exclusions(apps)` | Сохраняет список приложений-исключений |
| `traffic()` | Читает байт-счётчики из clash API (`:19099/connections`) |
| `preview_config(id)` | Возвращает pretty-printed sing-box конфиг для просмотра |
| `check_update()` | Проверяет GitHub Releases |
| `install_update()` | Скачивает + устанавливает обновление с прогрессом |
| `get_log()` | Читает `core.log` |

**Тонкость `list_apps` (macOS):** читает `Info.plist` из `.app`-бандлов в `/Applications`, `/System/Applications` и `~/Applications`. Извлекает иконки (`.icns`) параллельно в 8 потоков, кодирует в base64 PNG. Важно: ключ роутинга — `CFBundleExecutable` (имя процесса), не package name.

**Тонкость `list_apps` (Windows):** сканирует `.lnk`-ярлыки в Start Menu, разрешает цель до `.exe`. Иконки не извлекаются (нет простого способа на Windows без Win32 API).

#### `parser.rs` — Парсеры share-ссылок

Поддерживаемые схемы: `vless://`, `hysteria2://` / `hy2://`, `vmess://`, `ss://`, `trojan://`, `tuic://`

- `parse_link(link)` — парсит одну ссылку
- `parse_many(text)` — парсит блок текста, автоматически декодирует base64-подписки
- Каждый парсер возвращает `Server { id, name, protocol, address, port, outbound: Value, raw }`, где `outbound` — готовый sing-box outbound JSON с тегом `"proxy"`

**Нюансы парсера:**
- Hysteria2: auth может содержать `:` (формат `user:pass`) — парсер явно реджоинит username + password
- VMess: весь payload — base64-encoded JSON (`vmess://base64...`)
- Shadowsocks: поддерживает два layout'а — `method:password` в userinfo и полностью base64-encoded
- TUIC: `congestion_control` по умолчанию `bbr`, `alpn` по умолчанию `["h3"]`

#### `singbox.rs` — Генерация конфига

```
build_config(server, mode, bypass_ru, excluded) → serde_json::Value
```

Режимы (`Mode`):
- `SystemProxy` — `mixed` inbound на `127.0.0.1:2080`, `set_system_proxy: true`. Без root.
- `Tun` — TUN inbound, `auto_route: true`, `strict_route: true`, `mtu: 1400` (важно — стандартный MTU 9000 роняет туннель на мобильных и CGNAT сетях)

DNS:
- Удалённый DoH (`1.1.1.1`) ходит через proxy
- `local-ru` (`77.88.8.8`) используется для РФ-доменов и как `default_domain_resolver`
- `strategy: "ipv4_only"` — критично: AAAA-записи ломают Happy-Eyeballs если прокси не поддерживает IPv6
- `independent_cache: true`

Route:
- `action: "sniff"` — перехват DNS
- `action: "hijack-dns"` — перехват DNS-запросов
- LAN и `geosite-private` → direct
- При `bypass_ru`: `geosite-category-ru` + `geoip-ru` → direct
- Per-app exclusions: `process_name` → direct (в TUN) или systemproxy (частично)
- Всё остальное → proxy

Rule-sets:
- Тип `remote`, формат `binary` (.srs от SagerNet)
- `download_detour: "direct"` — **критично**: нельзя `proxy`, иначе bootstrap-цикл при первом запуске (нет кеша → нужен роутинг → нет rule-sets)
- `update_interval: "72h"`

Clash API: `127.0.0.1:19099` (нестандартный порт чтобы не конфликтовать с другими клиентами)

#### `core.rs` — Жизненный цикл sing-box

**System Proxy mode:**
- `Command::new(bin).arg("run").arg("-c").arg(cfg)` от текущего пользователя
- Запускает из `config_dir` (не из `/`) — иначе sing-box не может создать `cache.db` на read-only `/`
- На Windows: флаг `CREATE_NO_WINDOW` чтобы не мелькало консольное окно
- Ждёт готовности polling'ом до 2500ms (100-200ms шаг)

**TUN mode:**
- macOS: `osascript do shell script "... with administrator privileges"` — один диалог пароля
- Windows: PowerShell `Start-Process -Verb RunAs`
- Получает PID дочернего процесса (для мониторинга и kill)

**Liveness check (TUN):**
- macOS: `/bin/ps -p <pid>` (работает для root-owned процессов, `kill -0` не работает без root)
- Windows: `tasklist /FI "PID eq <pid>"`
- `Liveness::Unknown` ≠ `Liveness::Dead` — ошибка запуска probe не должна интерпретироваться как падение VPN

**Stop:**
- Kill child process (если не-elevated)
- Kill elevated PID через osascript/PowerShell (если TUN)
- `sysproxy::clear_all()` — аварийный сброс, чтобы прокси не остался настроенным после hard kill

#### `store.rs` — Хранилище

| Файл | Что хранит |
|---|---|
| `servers.json` | Массив `Server` объектов |
| `settings.json` | `Settings { mode, bypass_ru, active_server, accent, accent2, theme, excluded_apps }` |
| `config.json` | Последний sing-box конфиг (пишет `core.rs`) |
| `core.log` | stdout/stderr sing-box |

Путь на macOS: `~/Library/Application Support/coffeeNetwork/`
Путь на Windows: `%APPDATA%\coffeeNetwork\`
Путь на Linux: `~/.local/share/coffeeNetwork/`

### 2.4 Хранилище данных

По умолчанию `Settings`:
- `mode: SystemProxy`
- `bypass_ru: true`
- `accent: "amber"`, `accent2: "amber"`
- `theme: "dark"`
- `excluded_apps: []`

### 2.5 Протоколы и парсинг ссылок

| Протокол | Схема | Особенности |
|---|---|---|
| VLESS | `vless://uuid@host:port?...#name` | Поддержка flow, TLS/Reality, uTLS fingerprint, транспорт (ws/grpc/http/httpupgrade) |
| Hysteria2 | `hysteria2://auth@host:port?...#name` | obfs salamander, pinSHA256, up/down_mbps |
| VMess | `vmess://base64json` | Декодирует base64, извлекает поля JSON V2Ray |
| Shadowsocks | `ss://...#name` | Два layout'а (userinfo vs полный base64) |
| Trojan | `trojan://password@host:port?...#name` | TLS всегда включён |
| TUIC v5 | `tuic://uuid:password@host:port?...#name` | BBR congestion, h3 ALPN по умолчанию |

**TLS параметры (общие):** `security`, `sni`/`peer`/`host`, `alpn`, `allowInsecure`/`insecure`, `fp` (uTLS fingerprint), `pbk`+`sid` (REALITY)

### 2.6 Логика роутинга (sing-box)

```
Трафик
  │
  ├── LAN / 10.x / 192.168.x + geosite-private → DIRECT
  │
  ├── excluded_apps (process_name) → DIRECT [если задан ИГНОР]
  │
  ├── geosite-category-ru + geoip-ru → DIRECT [если RU-BYPASS вкл]
  │
  └── всё остальное → PROXY
```

DNS:
```
Domain
  │
  ├── geosite-category-ru → local-ru (77.88.8.8, Яндекс DoH) [если RU-BYPASS]
  │
  └── всё остальное → remote (1.1.1.1 DoH через прокси)
```

### 2.7 Жизненный цикл процесса sing-box

```
connect():
  stop() → генерировать конфиг → записать config.json
       │
       ├── SystemProxy → spawn_plain() → ждать 2.5s → проверить liveness
       └── TUN → spawn_elevated() → ждать 1.2s → проверить liveness

stop():
  kill child/elevated_pid → sysproxy::clear_all()
```

### 2.8 Меню-бар (трей)

Пункты:
- `● Подключено · <имя>` или `○ Отключено · <имя>` (disabled, только индикатор)
- `Остановить подключение` / `Подключиться`
- `Открыть coffeeNetwork`
- `Проверить обновления…`
- `Выйти`

**Поведение при закрытии окна:** окно скрывается (`hide()`), приложение продолжает работать в меню-баре. Выход только через пункт меню «Выйти».

**TUN mode + Подключиться из трея:** выполняется в отдельном потоке (может блокировать на диалоге пароля admin).

### 2.9 Автообновление

- Использует `tauri-plugin-updater` (проверяет `latest.json` из GitHub Releases)
- Ключ минподписи (`minisign`) только в GitHub Secrets
- При найденном обновлении: диалог с описанием изменений + кнопки «Обновить сейчас» / «В следующий раз» / «Пропустить версию»
- Прогресс скачивания: `update-progress` event, `DownloadProgress { downloaded, total, percent }`
- Проверяется при старте (через `check_update`) и из трея

### 2.10 Сборка и релиз

```bash
npm install
npm run tauri dev              # dev-режим
npm run build:mac              # .app + ad-hoc подпись + снятие карантина
npm run build:mac -- --dmg    # + .dmg
npm run tauri build            # кросс-платформа
```

**macOS Apple Silicon:** обязательна ad-hoc подпись (`signingIdentity: "-"` в tauri.conf.json). Без подписи macOS убивает бинарь с `killed: 9`.

**Релиз:** `git tag vX.Y.Z && git push origin vX.Y.Z` → GitHub Actions собирает macOS + Windows, публикует Release + `latest.json`.

---

## 3. Android (coffeeNetwork-android)

Репозиторий: `/Users/purrweb/code/coffeeNetwork-android`

### 3.1 Стек и структура файлов

```
coffeeNetwork-android/
├── lib/
│   └── main.dart              # Весь Flutter UI (один файл, ~1350 строк)
├── android/
│   └── app/src/main/kotlin/com/coffeenetwork/coffeenetwork/
│       ├── App.kt             # Application класс, хранит ссылки на ConnectivityManager
│       ├── MainActivity.kt    # Flutter Activity + MethodChannel обработчик
│       ├── CoffeeVpnService.kt # VpnService + PlatformInterface для libbox
│       ├── CoffeeTileService.kt # Quick Settings Tile
│       ├── SingBoxConfig.kt   # Генерация sing-box конфига (зеркало singbox.rs)
│       ├── DefaultNetworkListener.kt   # Монитор сети (callback при смене сети)
│       └── DefaultNetworkMonitor.kt    # Управление DefaultNetworkListener
├── pubspec.yaml
└── analysis_options.yaml
```

**Стек:**
- **Flutter** — весь UI (один экран с PageView, два листа)
- **Kotlin** — нативный слой (VpnService, MethodChannel, libbox)
- **sing-box libbox** (`libbox.aar`) — golang-compiled, gomobile форк SagerNet
- **MethodChannel** `coffeenetwork/vpn` — мост Flutter ↔ Kotlin
- **EventChannel** `coffeenetwork/update_progress` — прогресс загрузки APK

### 3.2 Архитектура Flutter ↔ Android ↔ libbox

```
Flutter UI (main.dart)
    │
    │ MethodChannel("coffeenetwork/vpn")
    │ .invokeMethod("connect" / "disconnect" / "status" / "traffic" / "parse" / "listApps")
    ▼
MainActivity.kt
    │
    ├── "connect" → startService(ACTION_START, config, exclude)
    ├── "disconnect" → startService(ACTION_STOP)
    ├── "status" → CoffeeVpnService.running + lastError + clash_api status
    ├── "traffic" → clash_api :19099/connections
    ├── "parse" → SingBoxConfig.parseLink(link)
    ├── "listApps" → PackageManager.getInstalledPackages()
    ├── "appVersion" → BuildConfig.VERSION_NAME
    └── "installUpdate" → DownloadManager + FileProvider
    
CoffeeVpnService : VpnService, PlatformInterface, CommandServerHandler
    │
    ├── onStartCommand(ACTION_START) → startVpn(config)
    │   ├── Libbox.setup(basePath, workingPath, tempPath)
    │   ├── DefaultNetworkMonitor.start()
    │   ├── CommandServer.start()
    │   └── server.startOrReloadService(config, override)
    │       └── libbox запускает TUN, применяет excludePackages
    │
    └── PlatformInterface impl
        ├── openTun(options) → VpnService.Builder → establish() → fd
        ├── findConnectionOwner() → ConnectivityManager.getConnectionOwnerUid()
        ├── getInterfaces() → NetworkInterface + ConnectivityManager
        └── systemCertificates() → AndroidCAStore KeyStore
```

### 3.3 UI: экраны и компоненты

**PageView с двумя страницами** (свайп влево/вправо):

**Страница 1: `_TicketPage` (Талон)**
- Шапка: `COFFEE / NETWORK` + иконка самолёта
- STATUS + MODE (горизонтальный блок)
- Огромный `heroCode` (3 буквы активного сервера или OFF/ON/BYE/...)
- Имя активного сервера, NODE, PROTOCOL, ROUTING
- Live speed (↓ KB/s ↑ KB/s) — показывается только когда connected
- Кнопка CONNECT/DISCONNECT
- Сегмент SYS PROXY / TUN · ALL
- Переключатель RU-BYPASS

**Страница 2: `_ServersPage` (Серверы)**
- Счётчик серверов + кнопка `+ ДОБАВИТЬ`
- ListView серверов (протокол-бейдж, имя, адрес:порт, кнопка удаления)
- Кнопки внизу: ИГНОР | LOG | НАСТР

**Modal sheets:**
- `_addSheet` — поле ввода ссылок + кнопка IMPORT
- `_ExclSheet` — список установленных приложений с иконками и поиском
- `_settingsSheet` — тема (тёмная/светлая) + 8 пресетов акцента + HSV color picker (три бара: H, S, V) + проверить обновления

**`_Onboarding`** — полноэкранный туториал при первом запуске (6 шагов, PageView, сохраняется в SharedPreferences `onboarded`)

**`_UpdateDialog`** — диалог обновления с прогресс-баром загрузки APK

### 3.4 VPN-сервис (CoffeeVpnService)

`CoffeeVpnService : VpnService, PlatformInterface, CommandServerHandler`

**Запуск:**
1. `ACTION_START` с extras `config` (JSON) и `exclude` (List<String> package names)
2. `startForeground` нотификация (обязательна на Android)
3. В фоновом потоке: `Libbox.setup()` → `DefaultNetworkMonitor.start()` → `CommandServer.start()` → `server.startOrReloadService(config, override)`
4. После `running = true`: регистрируется `networkTypeCallback` (следит за WiFi↔Cellular переключениями)
5. `libbox` вызывает `PlatformInterface.openTun()` для создания TUN-интерфейса

**NetworkCallback (авто-переподключение при смене типа сети — добавлено в 0.1.7):**
- `networkTypeCallback` слушает все сетевые события через `ConnectivityManager.registerNetworkCallback`
- При срабатывании вызывает `checkNetworkType()` → `currentlyOnCellular()` → сравнивает с `lastWasCellular`
- Если тип сети поменялся (WiFi→Mobile или Mobile→WiFi): вызывает `reconnectWithNewType(isMobile)` в фоновом потоке
- `reconnectWithNewType()`: читает параметры из SharedPreferences (`link`, `bypassRu`, `exclude`) → пересобирает конфиг через `SingBoxConfig.build(..., isMobile)` → вызывает `commandServer.startOrReloadService()` (горячая перезагрузка, TUN не переподнимается)
- Callback снимается в `stopVpn()` перед остановкой

**`openTun()` — создание VPN-туннеля:**
- `VpnService.Builder` настраивает MTU, адреса, маршруты, DNS
- На Android 13+: использует `inet4RouteAddress`/`excludeRoute` API
- Per-app: `addDisallowedApplication(pkg)` для каждого пакета из `excludePackages`
- Возвращает `fd` (file descriptor TUN-интерфейса)

**Per-app исключения:**
- Передаются через `OverrideOptions.excludePackage`
- `StringArray.len()` ДОЛЖЕН возвращать реальный размер — libbox прелоцирует по нему, иначе список молча теряется

**Остановка:**
- `ACTION_STOP` → `commandServer.closeService()` → `DefaultNetworkMonitor.stop()` → `commandServer.close()` → `pfd.close()` → `stopForeground()` → `stopSelf()`
- `onRevoke()` вызывается системой при отзыве VPN permission

### 3.5 Quick Settings Tile

`CoffeeTileService : TileService`

- Показывает иконку VPN в шторке рядом с Wi-Fi
- Тап переключает VPN (старт/стоп через startService)
- Нет постоянного уведомления в статус-баре (только foreground service нотификация во время работы)

### 3.6 Per-app исключения (ИГНОР)

1. Flutter вызывает `_vpn.invokeMethod('listApps')` → MainActivity возвращает JSON массив `{name, package, icon}` через `PackageManager.getInstalledPackages()`
2. `_ExclSheet` показывает список с иконками и поиском
3. При сохранении: `state.excluded = working; state._save()` → пишет в SharedPreferences
4. Если VPN активен — переподключение (disconnect + delay 500ms + connect с новым exclude)
5. Flutter передаёт `excluded.toList()` в `invokeMethod('connect', {... 'exclude': ...})`
6. Kotlin передаёт в сервис как `EXTRA_EXCLUDE` → `OverrideOptions.excludePackage`

### 3.7 Автообновление APK

1. `checkUpdate()` — GET `api.github.com/repos/Arti-Ko/coffeeNetwork-android/releases/latest`
2. Сравнивает semver текущей и удалённой версии
3. Диалог `_UpdateDialog` с notes + прогресс-баром
4. `installUpdate(url)` → нативный `DownloadManager` → FileProvider → система-установщик
5. EventChannel `coffeenetwork/update_progress` эмитит проценты загрузки
6. Если нет разрешения `REQUEST_INSTALL_PACKAGES` → просит открыть настройки

### 3.8 Темы и акцентный цвет

**Класс `Pal`** — рантайм-палитра:
- `Pal.dark` — тёмная/светлая тема
- `Pal.accent` — акцентный цвет (Color)
- `Pal.accentInk` — цвет текста ON акцент (вычисляется через luminance: тёмный на светлом, светлый на тёмном)
- Все цвета фона, карточек, границ вычисляются из `Pal.dark`

**8 пресетов** + произвольный цвет через HSV (три ползунка: оттенок, насыщенность, яркость)

Сохраняется в SharedPreferences: `dark` (bool), `accent` (int).

Смена темы: `rootKey.currentState?.refresh()` → перестраивает MaterialApp + статус-бар.

### 3.9 Сборка и подпись

```bash
export JAVA_HOME=/opt/homebrew/opt/openjdk@17
export ANDROID_HOME=$HOME/Library/Android/sdk
flutter pub get
flutter build apk --release
```

APK: `build/app/outputs/flutter-apk/app-release.apk`

**Подпись:** `android/key.properties` + `android/app/coffee.jks` (в .gitignore). При отсутствии — debug ключ.

**`libbox.aar`:** собран из sing-box (branch `main`) через gomobile. Тег `with_naive_outbound` исключён — cronet-go не линкуется на NDK r27.

**CI:** `.github/workflows/build.yml` — APK при каждом push в main, Release по тегу `v*`.

---

## 4. Общее: логика роутинга RU-BYPASS

Одинакова на десктопе (`singbox.rs`) и Android (`SingBoxConfig.kt`):

| Трафик | Направление |
|---|---|
| LAN, 10.x, 192.168.x | Direct (всегда) |
| `geosite-private` | Direct (всегда) |
| Per-app exclusions | Direct (если задан ИГНОР) |
| `geosite-category-ru` + `geoip-ru` | Direct (если RU-BYPASS вкл) |
| Всё остальное | Proxy |

**DNS без утечек:**
- РФ-домены → `local-ru` (77.88.8.8) напрямую — чтобы Яндекс/VK CDN работали корректно
- Всё остальное → DoH через прокси

**Критичные параметры:**
- `strategy: "ipv4_only"` — без этого AAAA-записи ломают соединение если прокси не умеет IPv6
- `mtu: 1400` — без этого туннель падает на CGNAT/мобильных сетях (дефолт sing-box 9000)
- `download_detour: "direct"` у rule-sets — без этого bootstrap-цикл при первом запуске

---

## 5. История изменений (changelog)

### Desktop (coffeeNetwork)

| Версия | Дата (примерно) | Что изменилось |
|---|---|---|
| 0.1.0 | — | Первый релиз |
| — | — | `fix: ИГНОР first-tap crash + custom color picker + remove splash icon` |
| — | — | `feat: per-app split-tunneling с иконками приложений (кнопка ИГНОР)` |
| — | — | `fix: добавление сервера на Windows — кроссплатформенный каталог данных` |
| — | — | `fix: вкладка «ИГНОР» на Windows — вечная загрузка, не грузились приложения` |
| 0.2.0 | — | `feat: вспомогательный акцент, прогресс-бар обновления, игнор по приложениям` |
| — | — | `fix: рабочий TUN на Windows и macOS + стабильный системный прокси на Windows` |
| — | — | `fix: не показывать «отключено» при временном сбое проверки TUN-процесса` (Liveness::Unknown) |
| — | — | `fix: надёжность подключения — RU-домены, IPv6 и мобильный интернет` |
| 0.2.1 | — | Bump версии |
| 0.2.2 | — | `fix: откат download_detour на direct (мог ломать старт ядра)` |
| **0.2.3** | **2026-07-01** | `docs: добавлена папка docs/ с живой документацией; синхронизация версий с Android` |

### Android (coffeeNetwork-android)

| Версия | Дата | Что изменилось |
|---|---|---|
| 0.1.0 | — | Первый релиз |
| 0.1.1 | — | `feat: визуальный онбординг при первом запуске` |
| 0.1.2 | — | `feat: проверка обновлений (фон + кнопка в настройках)` |
| 0.1.3 | — | `feat: авто-обновление APK в приложении + стабильная подпись релиза` |
| — | — | `fix: VPN не запускался (откат download_detour на direct)` |
| 0.1.5 | — | `fix: ядро падало через ~2 сек — откат конфига к рабочему оригиналу` |
| 0.1.6 | — | `fix: вернуть ipv4_only + mtu 1400 (проверено через adb на устройстве)` |
| **0.2.3** | **2026-07-01** | **`fix: Hysteria2 теперь работает на мобильном интернете + синхронизация версий`** |

**Детали 0.2.3 (Android):**

**Проблема 2 — VPN не работал на мобильном интернете (исправлено):**
- **Причина:** Hysteria2 использует BBR congestion control и агрессивно заполняет буфер. Неограниченный UDP-поток срабатывает DPI/rate-limiter оператора — сессия дропается. На 4G/5G операторы применяют более агрессивный rate limiting, чем на WiFi.
- **Решение:** `SingBoxConfig.build()` теперь принимает `isMobile: Boolean`. Для Hysteria2 автоматически ставит `up_mbps: 25, down_mbps: 25` на мобильном (ниже порога операторского rate-limiter), `100/100` на WiFi. Значения в URL ссылки (`?up=N&down=N`) имеют приоритет над дефолтами.
- **Дополнительно:** `parseHysteria2()` теперь парсит `up`/`down` query params из URL (ранее игнорировались, в отличие от desktop `parser.rs`).
- **Где:** `SingBoxConfig.kt:221` — `build()`, `SingBoxConfig.kt:75-76` — `parseHysteria2()`
- **Обнаружение типа сети:** `MainActivity.isCellular()` обходит все физические сети через `ConnectivityManager.allNetworks`, пропускает VPN-интерфейсы и интерфейсы без INTERNET capability. Если есть WiFi — возвращает `false` (WiFi приоритетнее, даже если Cellular тоже активен).

**Авто-адаптация при смене типа сети (WiFi ↔ Mobile):**
- `CoffeeVpnService.networkTypeCallback` — `ConnectivityManager.NetworkCallback` следит за всеми сетевыми событиями.
- При `onCapabilitiesChanged` и `onLost` вызывается `checkNetworkType()` → сравнивает текущий тип с `lastWasCellular`.
- При смене типа: запускает `reconnectWithNewType(isMobile)` в фоновом потоке.
- `reconnectWithNewType()`: читает `link`, `bypassRu`, `exclude` из SharedPreferences `"coffee"` → пересобирает конфиг через `SingBoxConfig.build(..., isMobile)` → горячая перезагрузка `commandServer.startOrReloadService()` без переподъёма TUN.
- Callback регистрируется в `startVpn()`, снимается в `stopVpn()`.

---

**Проблема 1 — Периодические обрывы соединения (анализ и текущее состояние):**

**Суть проблемы:** QUIC/UDP не имеет встроенного TCP-keepalive. NAT-таблица маршрутизатора/оператора удаляет запись об UDP-сессии после N секунд простоя (типично 30-60 секунд на hardware NAT операторов, в том числе CGNAT на мобильных сетях). После этого пакеты с сервера идут в никуда — туннель "умирает" молча.

**Предложенное Gemini-решение:** установить `ping_interval: 10-15s` (нативный Hysteria2 параметр — заставляет клиент слать QUIC PING-фреймы, сбрасывая счётчик NAT).

**Результат исследования libbox.so:**
- Выполнен `strings` анализ `/jni/arm64-v8a/libbox.so` из `libbox.aar`
- **`ping_interval` как JSON-поле Hysteria2 outbound — ОТСУТСТВУЕТ в этой версии libbox.** Поля в схеме `Hysteria2OutboundOptions`: `up_mbps`, `down_mbps`, `hop_interval`, `hop_interval_max`, `brutal_debug`, `multiplex`, `tls`, `obfs`.
- `keep_alive_period` существует в бинарнике, но является TCP-параметром (`net.(*TCPConn).SetKeepAlivePeriod`) — к QUIC не относится.
- QUIC keepalive обрабатывается внутри `github.com/sagernet/sing-quic.quicConfigWithHandshakeTimeout` — период неизвестен без анализа исходников, управлять им через JSON нельзя.
- `hop_interval` (переключение UDP-порта каждые N сек) есть в схеме, но: (a) требует портового диапазона на сервере, (b) вызывает реконнект при каждом переключении — не подходит как keepalive.
- `multiplex.heartbeat` есть в бинарнике и работало бы как application-layer keepalive — но требует, чтобы **сервер поддерживал sing-box mux** (3X-UI запускает нативный binary hysteria2, который mux не понимает).

**Итог:** клиентская сторона проблемы 1 при использовании 3X-UI / нативного hysteria2 server с текущей версией libbox — **не решается конфигурацией JSON**. Решение на уровне сервера: в 3X-UI/Xray настроить keepalive для Hysteria2 inbound. На практике: исправление проблемы 2 (bandwidth limiting) также сокращает количество idle-периодов — оператор перестаёт дропать сессию агрессивно, BBR стабилизируется на низком bandwidth.

**Что делать если обрывы всё равно есть (сервер 3X-UI):**
1. В панели 3X-UI → настройки Hysteria2 inbound → установить keepalive / `quicKeepalivePeriod: 15s`.
2. Если сервер на sing-box: добавить `?mux=1` в ссылку (нужна доработка клиента — см. pending tasks).
3. Попробовать снизить `up/down` до 10-15 Mbps через параметры URL (`?up=10&down=10`).

---

**Текст описания для GitHub Release (0.2.3, Android):**

```
Теперь VPN работает на мобильном интернете

Раньше Hysteria2 мог не работать через 4G/5G — оператор видел слишком
агрессивный UDP-трафик и дропал соединение. Теперь скорость автоматически
ограничивается до безопасного уровня на мобильных сетях, а при переключении
с Wi-Fi на мобильный интернет (и обратно) всё подстраивается само.

• Исправлен сбой подключения на 4G/5G
• Автоматическая адаптация при переключении Wi-Fi ↔ мобильный интернет
• Скорость на Wi-Fi не ограничивается
• Версии Desktop и Android теперь синхронизированы (0.2.3)
```

### Работа с документацией (этот документ)

| Дата | Действие | Результат |
|---|---|---|
| 2026-07-01 | Клонированы оба репозитория с GitHub (Arti-Ko) | `/code/coffeeNetwork` + `/code/coffeeNetwork-android` |
| 2026-07-01 | Создана первичная документация `coffeeNetwork-docs.md` | Охват всей архитектуры, модулей, нюансов — базис для будущей работы |
| 2026-07-01 | Добавлен технический анализ проблем 1 и 2 (Hysteria2/NAT/libbox) | Секция 5, changelog 0.2.3 |
| 2026-07-01 | Документ перемещён в `coffeeNetwork/docs/` (десктопный репо) | Документация теперь версионируется вместе с кодом |

---

---

## 5.1 Исследование libbox.aar (бинарный анализ, 2026-07-01)

**Цель:** найти поле `ping_interval` для решения NAT-keepalive проблемы.

**Метод:** `unzip libbox.aar` → `strings jni/arm64-v8a/libbox.so | grep ...`

**Найденные JSON-поля Hysteria2 outbound:**
```
json:"up_mbps,omitempty"
json:"down_mbps,omitempty"
json:"hop_interval,omitempty"
json:"hop_interval_max,omitempty"
json:"brutal_debug,omitempty"
json:"multiplex,omitempty"       ← есть, но требует server mux support
```

**Отсутствующие поля (подтверждено):**
- `ping_interval` — нет в схеме Hysteria2 outbound
- `quic_keepalive` — нет

**Поля найденные, но неприменимые:**
- `keep_alive_period` — есть, но рядом с `net.(*TCPConn).SetKeepAlivePeriod` → TCP, не QUIC
- `heartbeat` — есть в контексте `multiplex` (yamux/smux), а не Hysteria2 напрямую

**QUIC internals:** `github.com/sagernet/sing-quic.quicConfigWithHandshakeTimeout`, `github.com/sagernet/sing-quic/hysteria.NewClient` — keepalive обрабатывается внутри go-библиотеки, JSON-поля для управления нет.

**Вывод:** Для решения NAT-keepalive через клиентский JSON config в данной версии libbox возможностей нет. Обходные пути: сервер-side конфигурация или sing-box mux (если сервер поддерживает).

---

## 6. Известные особенности и нюансы

### Desktop

1. **TUN на Apple Silicon требует подписи.** Без `ad-hoc` подписи macOS убивает бинарь с `killed: 9`. `signingIdentity: "-"` в tauri.conf.json — обязательно.

2. **Карантин при скачивании с GitHub.** macOS помечает `.dmg` карантином. Первый запуск: правый клик → Открыть → Открыть. Без `xattr` в терминале.

3. **TUN требует root-пароля каждый раз.** Пароль не сохраняется — один диалог `osascript` при каждом подключении.

4. **sing-box бинарь** встроен как sidecar через `scripts/fetch-sing-box.sh`. В `tauri dev` ищет в Homebrew/PATH.

5. **Clash API порт 19099** (не 9090) — специально чтобы не конфликтовать с другими Clash/sing-box клиентами.

6. **CWD при запуске:** sing-box запускается из `config_dir`, иначе не создаёт `cache.db` на read-only `/` (Finder запускает из `/`).

7. **Per-app на Windows** работает только для TUN. В SystemProxy — sing-box применяет `process_name` только к проксированным соединениям.

### Android

1. **`StringArray.len()` ДОЛЖЕН возвращать реальный размер.** Если вернёт 0 — libbox прелоцирует 0 слотов и молча дропает весь список per-app exclusions. Это сломало ИГНОР в прошлом.

2. **`download_detour: "direct"` у rule-sets** — нельзя менять на proxy. Bootstrap-цикл: роутер нужен для загрузки rule-sets, но загрузка требует готового роутера.

3. **`ipv4_only` стратегия DNS** — обязательна. AAAA-записи ломают соединения через прокси, который не тянет IPv6.

4. **MTU 1400** — обязателен. sing-box дефолт 9000 роняет туннель на CGNAT и мобильных сетях.

5. **`libbox.aar` без `naive`** — тег `with_naive_outbound` исключён, cronet-go не линкуется на NDK r27.

6. **Foreground service** — обязателен на Android для VPN. Без него система убивает сервис при сворачивании.

7. **Per-app reconnect:** при изменении списка ИГНОР пока VPN активен — автоматическое переподключение с задержкой 500ms.

8. **Hysteria2 на мобильном:** без `up_mbps`/`down_mbps` оператор дропает UDP-поток. С 0.1.7 ставится 25/25 Mbps на мобильном. Если VPN всё равно нестабилен на конкретном операторе — попробовать снизить до 10-15 Mbps. Значение в URL ссылки переопределяет авто-дефолт.

9. **`networkTypeCallback` и двойная сеть (WiFi + Cellular):** если оба интерфейса активны одновременно, `currentlyOnCellular()` возвращает `false` (WiFi приоритетнее). Это корректно: если есть WiFi, именно он будет использоваться системой.

---

## 7. Дальнейшая работа (задачи)

### Выполнено (0.2.3)
- [x] **Android**: `SingBoxConfig.build()` — `isMobile` param, bandwidth limits для Hysteria2
- [x] **Android**: `parseHysteria2()` парсит `up`/`down` из URL (как desktop `parser.rs`)
- [x] **Android**: `MainActivity.isCellular()` — определение типа сети (WiFi/Cellular)
- [x] **Android**: `CoffeeVpnService.networkTypeCallback` — авто-перезагрузка конфига при WiFi↔Mobile
- [x] **Docs**: технический анализ проблем 1 и 2, результаты исследования libbox.so
- [x] **Синхронизация версий**: Desktop 0.2.3 + Android 0.2.3

### Pending
- [ ] **Desktop**: добавить опциональные `up_mbps`/`down_mbps` в `Settings` (`store.rs`) и применять для Hysteria2 в `singbox.rs`. Меньший приоритет — на десктопе проблема реже.
- [ ] **Desktop**: отобразить настройку bandwidth в UI (`main.ts`)
- [ ] **Android**: рассмотреть отображение типа сети (WiFi/Mobile) в UI на Ticket-странице
- [ ] **Проблема 1 (NAT keepalive)**: если 3X-UI → сервер-side fix (keepalive в настройках Hysteria2 inbound). Если сервер sing-box → реализовать `?mux=1` URL-параметр, добавляющий `multiplex.heartbeat: "15s"` в конфиг.

---

*Документ обновлён: 2026-07-01*
