//! Splitbox — Tauri command layer.

mod core;
mod parser;
mod singbox;
mod store;
mod sysproxy;

use serde::Serialize;
use uuid::Uuid;

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

use core::CoreState;
use parser::Server;
use singbox::Mode;
use store::Settings;

#[derive(Serialize)]
pub struct Status {
    running: bool,
    active_server: Option<String>,
    mode: Option<Mode>,
    bypass_ru: bool,
    core_path: Option<String>,
}

// ---------------------------------------------------------------------------
// server management
// ---------------------------------------------------------------------------

#[tauri::command]
fn list_servers() -> Vec<Server> {
    store::load_servers()
}

/// Parse one or many share-links (or a base64 subscription) and append the
/// successfully parsed servers. Returns the full updated list.
#[tauri::command]
fn add_links(text: String) -> Result<Vec<Server>, String> {
    let mut parsed = parser::parse_many(&text);
    if parsed.is_empty() {
        // Surface a precise error for a single bad link.
        if text.trim().contains("://") {
            return Err(parser::parse_link(text.trim())
                .err()
                .unwrap_or_else(|| "Не удалось распознать ни одной ссылки".into()));
        }
        return Err("Не удалось распознать ни одной ссылки".into());
    }
    let mut servers = store::load_servers();
    for s in parsed.iter_mut() {
        s.id = Uuid::new_v4().to_string();
        servers.push(s.clone());
    }
    store::save_servers(&servers)?;
    Ok(servers)
}

#[tauri::command]
fn delete_server(state: tauri::State<CoreState>, id: String) -> Result<Vec<Server>, String> {
    if state.active_server().as_deref() == Some(id.as_str()) {
        core::stop(&state)?;
    }
    let servers: Vec<Server> = store::load_servers().into_iter().filter(|s| s.id != id).collect();
    store::save_servers(&servers)?;
    Ok(servers)
}

#[tauri::command]
fn rename_server(id: String, name: String) -> Result<Vec<Server>, String> {
    let mut servers = store::load_servers();
    for s in servers.iter_mut() {
        if s.id == id {
            s.name = name.clone();
        }
    }
    store::save_servers(&servers)?;
    Ok(servers)
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_settings() -> Settings {
    store::load_settings()
}

#[tauri::command]
fn set_settings(mode: Mode, bypass_ru: bool) -> Result<Settings, String> {
    let mut s = store::load_settings();
    s.mode = mode;
    s.bypass_ru = bypass_ru;
    store::save_settings(&s)?;
    Ok(s)
}

/// Persist appearance preferences (primary + secondary accent, theme).
/// Returns full Settings.
#[tauri::command]
fn set_appearance(accent: String, accent2: String, theme: String) -> Result<Settings, String> {
    let mut s = store::load_settings();
    s.accent = accent;
    s.accent2 = accent2;
    s.theme = theme;
    store::save_settings(&s)?;
    Ok(s)
}

/// The bundled app version (Cargo package version) for the settings screen.
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ---------------------------------------------------------------------------
// per-app split tunneling (exclusions)
// ---------------------------------------------------------------------------

/// An installed macOS app the user can exclude from the VPN.
#[derive(Serialize)]
pub struct AppInfo {
    /// Display name (CFBundleDisplayName / CFBundleName / bundle filename).
    name: String,
    /// Executable name (CFBundleExecutable) — what sing-box matches as process_name.
    exec: String,
    /// App icon as a `data:image/png;base64,…` URI (None if unavailable).
    icon: Option<String>,
}

#[cfg(target_os = "macos")]
struct AppRaw {
    name: String,
    exec: String,
    bundle: std::path::PathBuf,
    icon_file: Option<String>,
}

/// List installed apps the user can exclude from the VPN. Platform-specific:
/// macOS reads `/Applications` bundles (with icons); Windows scans the standard
/// install roots for executables. Other platforms return an empty list.
#[tauri::command]
fn list_apps() -> Vec<AppInfo> {
    list_apps_impl()
}

/// macOS: enumerate `.app` bundles, reading each Info.plist; icons are extracted
/// from `.icns` in parallel for speed.
#[cfg(target_os = "macos")]
fn list_apps_impl() -> Vec<AppInfo> {
    use std::collections::BTreeMap;

    let home = std::env::var("HOME").unwrap_or_default();
    let roots = [
        "/Applications".to_string(),
        "/Applications/Utilities".to_string(),
        "/System/Applications".to_string(),
        "/System/Applications/Utilities".to_string(),
        format!("{home}/Applications"),
    ];

    // de-duplicate by exec (process name — what routing keys on).
    let mut found: BTreeMap<String, AppRaw> = BTreeMap::new();

    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("app") {
                continue;
            }
            let Ok(value) = plist::Value::from_file(path.join("Contents/Info.plist")) else {
                continue;
            };
            let Some(dict) = value.as_dictionary() else { continue };
            let Some(exec) = dict
                .get("CFBundleExecutable")
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
            else {
                continue;
            };
            let name = dict
                .get("CFBundleDisplayName")
                .and_then(|v| v.as_string())
                .or_else(|| dict.get("CFBundleName").and_then(|v| v.as_string()))
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    path.file_stem().and_then(|s| s.to_str()).unwrap_or(&exec).to_string()
                });
            let icon_file = dict
                .get("CFBundleIconFile")
                .and_then(|v| v.as_string())
                .map(|s| s.to_string());

            found.entry(exec.clone()).or_insert(AppRaw {
                name,
                exec,
                bundle: path,
                icon_file,
            });
        }
    }

    let raw: Vec<AppRaw> = found.into_values().collect();

    // Extract icons in parallel — reading/decoding ~150 .icns serially is slow.
    let icons: Vec<Option<String>> = {
        let threads = 8usize.min(raw.len().max(1));
        let chunk = raw.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            let handles: Vec<_> = raw
                .chunks(chunk)
                .map(|ch| {
                    s.spawn(move || {
                        ch.iter()
                            .map(|a| icon_data_uri(&a.bundle, a.icon_file.as_deref()))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
        })
    };

    let mut apps: Vec<AppInfo> = raw
        .into_iter()
        .zip(icons)
        .map(|(a, icon)| AppInfo {
            name: a.name,
            exec: a.exec,
            icon,
        })
        .collect();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

/// Extract a small app icon from the bundle's `.icns` as a base64 PNG data URI.
#[cfg(target_os = "macos")]
fn icon_data_uri(bundle: &std::path::Path, icon_file: Option<&str>) -> Option<String> {
    use base64::Engine as _;
    use std::fs::File;
    use std::io::BufReader;

    let res = bundle.join("Contents/Resources");
    // Resolve the .icns path: named in Info.plist (maybe without extension),
    // else the first .icns in Resources.
    let icns_path = icon_file
        .map(|name| {
            let p = res.join(name);
            if p.extension().is_some() { p } else { p.with_extension("icns") }
        })
        .filter(|p| p.exists())
        .or_else(|| {
            std::fs::read_dir(&res).ok()?.flatten().map(|e| e.path()).find(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("icns")
            })
        })?;

    let family = icns::IconFamily::read(BufReader::new(File::open(&icns_path).ok()?)).ok()?;
    let types = family.available_icons();
    if types.is_empty() {
        return None;
    }
    // Prefer a small icon (~>=32px) to keep payload light; else the largest.
    let chosen = types
        .iter()
        .copied()
        .filter(|t| t.pixel_width() >= 32)
        .min_by_key(|t| t.pixel_width())
        .or_else(|| types.iter().copied().max_by_key(|t| t.pixel_width()))?;

    let image = family.get_icon_with_type(chosen).ok()?;
    let mut png: Vec<u8> = Vec::new();
    image.write_png(&mut png).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png)
    ))
}

/// Windows: list the user's *installed applications* by reading Start Menu
/// shortcuts (`.lnk`) — the same set the Windows "Installed apps" screen and the
/// Start menu show. Each shortcut resolves to its target executable; sing-box
/// matches `process_name` by that exe file name (e.g. `chrome.exe`), which is
/// what we expose as `exec`. The shortcut's name is the display label. Icons are
/// omitted — the picker falls back to a placeholder glyph.
#[cfg(target_os = "windows")]
fn list_apps_impl() -> Vec<AppInfo> {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    let mut roots: Vec<PathBuf> = Vec::new();
    // All-users Start Menu.
    if let Ok(pd) = std::env::var("ProgramData") {
        roots.push(PathBuf::from(pd).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    // Current-user Start Menu.
    if let Ok(ad) = std::env::var("APPDATA") {
        roots.push(PathBuf::from(ad).join(r"Microsoft\Windows\Start Menu\Programs"));
    }

    // exec (lowercased, e.g. "chrome.exe") -> display name. De-dupes across roots.
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    for root in roots {
        collect_shortcuts(&root, 6, &mut found);
    }

    let mut apps: Vec<AppInfo> = found
        .into_iter()
        .map(|(exec, name)| AppInfo { name, exec, icon: None })
        .collect();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

/// Recursively walk a Start Menu folder (bounded depth) collecting `.lnk`
/// shortcuts that point at an executable, recording exec → display-name.
#[cfg(target_os = "windows")]
fn collect_shortcuts(
    dir: &std::path::Path,
    depth: usize,
    found: &mut std::collections::BTreeMap<String, String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if depth > 1 {
                collect_shortcuts(&path, depth - 1, found);
            }
            continue;
        }
        let is_lnk = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("lnk"))
            == Some(true);
        if !is_lnk {
            continue;
        }
        let Some(target) = shortcut_target(&path) else { continue };
        let Some(file) = target.file_name().and_then(|s| s.to_str()) else { continue };
        let exec = file.to_lowercase();
        // Only real apps: an .exe target, minus installer/updater/helper noise.
        if !exec.ends_with(".exe") || is_noise_exe(&exec) {
            continue;
        }
        // Display name = the shortcut's own name (e.g. "Google Chrome").
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(file)
            .to_string();
        found.entry(exec).or_insert(name);
    }
}

/// Resolve a `.lnk` shortcut to its target path. Prefers the absolute target
/// recorded in the link, falling back to the link-relative path.
#[cfg(target_os = "windows")]
fn shortcut_target(lnk: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::convert::TryFrom;
    let parsed = parselnk::Lnk::try_from(lnk).ok()?;
    if let Some(abs) = parsed.link_info.local_base_path {
        if !abs.is_empty() {
            return Some(std::path::PathBuf::from(abs));
        }
    }
    if let Some(rel) = parsed.string_data.relative_path {
        return Some(lnk.parent().map(|p| p.join(&rel)).unwrap_or(rel));
    }
    None
}

/// Filter out non-user-facing executables (installers, updaters, crash helpers).
#[cfg(target_os = "windows")]
fn is_noise_exe(exec: &str) -> bool {
    const NOISE: [&str; 10] = [
        "unins", "setup", "install", "crashpad", "crashhandler", "repair",
        "elevation", "notification", "helper", "service",
    ];
    NOISE.iter().any(|n| exec.contains(n))
}

/// Other platforms: no app picker yet.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn list_apps_impl() -> Vec<AppInfo> {
    Vec::new()
}

/// Save the set of apps (process names) that bypass the VPN.
#[tauri::command]
fn set_exclusions(apps: Vec<String>) -> Result<Settings, String> {
    let mut s = store::load_settings();
    s.excluded_apps = apps;
    store::save_settings(&s)?;
    Ok(s)
}

// ---------------------------------------------------------------------------
// connection
// ---------------------------------------------------------------------------

/// Shared connect logic used by both the UI command and the tray menu.
/// Persists the active server, refreshes the tray, and notifies the window.
fn do_connect(app: &AppHandle, id: String) -> Result<Status, String> {
    let state = app.state::<CoreState>();
    let server = store::load_servers()
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("Сервер не найден")?;

    let mut settings = store::load_settings();
    core::connect(
        state.inner(),
        &server,
        settings.mode,
        settings.bypass_ru,
        &settings.excluded_apps,
    )?;

    settings.active_server = Some(id);
    store::save_settings(&settings)?;

    let st = build_status(state.inner());
    refresh_tray_async(app);
    let _ = app.emit("status-changed", ());
    Ok(st)
}

/// Shared disconnect logic. Keeps `active_server` as the last-used server so the
/// tray "Подключиться" can reconnect to it.
fn do_disconnect(app: &AppHandle) -> Result<Status, String> {
    let state = app.state::<CoreState>();
    core::stop(state.inner())?;
    let st = build_status(state.inner());
    refresh_tray_async(app);
    let _ = app.emit("status-changed", ());
    Ok(st)
}

#[tauri::command]
fn connect(app: AppHandle, id: String) -> Result<Status, String> {
    do_connect(&app, id)
}

#[tauri::command]
fn disconnect(app: AppHandle) -> Result<Status, String> {
    do_disconnect(&app)
}

#[tauri::command]
fn status(state: tauri::State<CoreState>) -> Status {
    build_status(&state)
}

#[tauri::command]
fn get_log() -> String {
    core::read_log().unwrap_or_default()
}

/// Cumulative byte counters from the sing-box Clash API. The UI polls this and
/// derives the per-second up/down speed from the deltas.
#[derive(Serialize, Default)]
pub struct Traffic {
    up: u64,
    down: u64,
}

#[tauri::command]
fn traffic() -> Traffic {
    read_traffic_totals()
        .map(|(up, down)| Traffic { up, down })
        .unwrap_or_default()
}

/// Minimal HTTP/1.0 GET to the local Clash API `/connections` endpoint, which
/// returns `uploadTotal`/`downloadTotal`. HTTP/1.0 → the server closes the
/// connection after the body, so we can read to EOF without chunk parsing.
fn read_traffic_totals() -> Option<(u64, u64)> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let addr = "127.0.0.1:19099".parse().ok()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(800)).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(1200))).ok()?;
    stream
        .write_all(b"GET /connections HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw);
    let body = text.split("\r\n\r\n").nth(1)?;
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    Some((
        v.get("uploadTotal").and_then(|x| x.as_u64()).unwrap_or(0),
        v.get("downloadTotal").and_then(|x| x.as_u64()).unwrap_or(0),
    ))
}

/// Result of an update check, surfaced to the UI.
#[derive(Serialize, Default)]
pub struct UpdateInfo {
    available: bool,
    version: Option<String>,
    notes: Option<String>,
}

/// Check GitHub Releases for a newer signed build. Non-fatal: any error
/// (offline, no release yet) is reported so the UI can stay silent.
#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(UpdateInfo {
            available: true,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
        }),
        Ok(None) => Ok(UpdateInfo::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Download progress, emitted to the UI as `update-progress` so it can drive a
/// real progress bar. `percent` is `None` while the total size is still unknown.
#[derive(Serialize, Clone)]
pub struct DownloadProgress {
    downloaded: u64,
    total: Option<u64>,
    percent: Option<f64>,
}

/// Download + install the pending update, then relaunch the app. Emits
/// `update-progress` events throughout the download so the UI can show a bar.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("Обновлений нет".into());
    };

    let mut downloaded: u64 = 0;
    let app_dl = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                let percent = total.and_then(|t| {
                    (t > 0).then(|| (downloaded as f64 / t as f64 * 100.0).min(100.0))
                });
                let _ = app_dl.emit(
                    "update-progress",
                    DownloadProgress { downloaded, total, percent },
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

/// Pretty-printed sing-box config for the selected server (for inspection).
#[tauri::command]
fn preview_config(id: String) -> Result<String, String> {
    let server = store::load_servers()
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("Сервер не найден")?;
    let settings = store::load_settings();
    let config = singbox::build_config(&server, settings.mode, settings.bypass_ru, &settings.excluded_apps);
    serde_json::to_string_pretty(&config).map_err(|e| e.to_string())
}

fn build_status(state: &CoreState) -> Status {
    let settings = store::load_settings();
    Status {
        running: state.is_running(),
        active_server: state.active_server().or(settings.active_server),
        mode: state.active_mode().or(Some(settings.mode)),
        bypass_ru: settings.bypass_ru,
        core_path: singbox::locate_binary().map(|p| p.display().to_string()),
    }
}

// ---------------------------------------------------------------------------
// menu-bar tray
// ---------------------------------------------------------------------------

/// (is_running, name-of-active-or-last-server) for the tray labels.
fn connection_label(app: &AppHandle) -> (bool, String) {
    let state = app.state::<CoreState>();
    let running = state.is_running();
    let settings = store::load_settings();
    let active_id = state.active_server().or(settings.active_server.clone());
    let name = active_id.and_then(|id| {
        store::load_servers()
            .into_iter()
            .find(|s| s.id == id)
            .map(|s| s.name)
    });
    (running, name.unwrap_or_else(|| "сервер не выбран".to_string()))
}

/// Build the menu-bar menu reflecting the current connection state.
fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let (running, name) = connection_label(app);
    let status_text = if running {
        format!("● Подключено · {name}")
    } else {
        format!("○ Отключено · {name}")
    };

    let status = MenuItemBuilder::with_id("status", status_text)
        .enabled(false)
        .build(app)?;
    let toggle = MenuItemBuilder::with_id(
        "toggle",
        if running { "Остановить подключение" } else { "Подключиться" },
    )
    .build(app)?;
    let open = MenuItemBuilder::with_id("open", "Открыть coffeeNetwork").build(app)?;
    let update = MenuItemBuilder::with_id("update", "Проверить обновления…").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Выйти").build(app)?;

    MenuBuilder::new(app)
        .items(&[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &toggle,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &update,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ])
        .build()
}

/// Rebuild the tray menu + tooltip from current state. MUST run on the main
/// thread (macOS UI requirement) — use [`refresh_tray_async`] off-thread.
fn refresh_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("main") else { return };
    if let Ok(menu) = build_tray_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
    let (running, name) = connection_label(app);
    let tip = if running {
        format!("coffeeNetwork · {name}")
    } else {
        "coffeeNetwork · отключено".to_string()
    };
    let _ = tray.set_tooltip(Some(&tip));
}

/// Schedule a tray refresh on the main thread; safe to call from any thread.
fn refresh_tray_async(app: &AppHandle) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || refresh_tray(&a));
}

fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// Handle a tray-menu click.
fn on_tray_menu(app: &AppHandle, id: &str) {
    match id {
        "toggle" => {
            // Connecting (esp. TUN) can block on an admin prompt — do it off the
            // main thread so the menu/UI stays responsive.
            let app = app.clone();
            std::thread::spawn(move || {
                let running = app.state::<CoreState>().is_running();
                let result = if running {
                    do_disconnect(&app).map(|_| ())
                } else {
                    match store::load_settings().active_server {
                        Some(id) => do_connect(&app, id).map(|_| ()),
                        None => {
                            show_main_window(&app);
                            Ok(())
                        }
                    }
                };
                if let Err(e) = result {
                    let _ = app.emit("tray-error", e);
                }
            });
        }
        "open" => show_main_window(app),
        "update" => {
            show_main_window(app);
            let _ = app.emit("tray://check-update", ());
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(CoreState::default())
        .setup(|app| {
            // menu-bar tray
            let menu = build_tray_menu(app.handle())?;
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("coffeeNetwork")
                .menu(&menu)
                .on_menu_event(|app, event| on_tray_menu(app, event.id().as_ref()));
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;

            // Closing the window hides it to the menu bar instead of quitting,
            // so the app keeps running as a menu-bar utility. "Выйти" quits.
            if let Some(win) = app.get_webview_window("main") {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_servers,
            add_links,
            delete_server,
            rename_server,
            get_settings,
            set_settings,
            set_appearance,
            app_version,
            list_apps,
            set_exclusions,
            connect,
            disconnect,
            status,
            get_log,
            traffic,
            preview_config,
            check_update,
            install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running coffeeNetwork");
}
