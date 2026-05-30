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

/// Persist appearance preferences (accent color + theme). Returns full Settings.
#[tauri::command]
fn set_appearance(accent: String, theme: String) -> Result<Settings, String> {
    let mut s = store::load_settings();
    s.accent = accent;
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
    core::connect(state.inner(), &server, settings.mode, settings.bypass_ru)?;

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

/// Download + install the pending update, then relaunch the app.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("Обновлений нет".into());
    };
    update
        .download_and_install(|_chunk, _total| {}, || {})
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
    let config = singbox::build_config(&server, settings.mode, settings.bypass_ru);
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
            connect,
            disconnect,
            status,
            get_log,
            preview_config,
            check_update,
            install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running coffeeNetwork");
}
