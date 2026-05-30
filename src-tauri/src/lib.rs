//! Splitbox — Tauri command layer.

mod core;
mod parser;
mod singbox;
mod store;
mod sysproxy;

use serde::Serialize;
use uuid::Uuid;

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

// ---------------------------------------------------------------------------
// connection
// ---------------------------------------------------------------------------

#[tauri::command]
fn connect(state: tauri::State<CoreState>, id: String) -> Result<Status, String> {
    let server = store::load_servers()
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("Сервер не найден")?;

    let mut settings = store::load_settings();
    core::connect(&state, &server, settings.mode, settings.bypass_ru)?;

    settings.active_server = Some(id);
    store::save_settings(&settings)?;
    Ok(build_status(&state))
}

#[tauri::command]
fn disconnect(state: tauri::State<CoreState>) -> Result<Status, String> {
    core::stop(&state)?;
    let mut settings = store::load_settings();
    settings.active_server = None;
    store::save_settings(&settings)?;
    Ok(build_status(&state))
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(CoreState::default())
        .invoke_handler(tauri::generate_handler![
            list_servers,
            add_links,
            delete_server,
            rename_server,
            get_settings,
            set_settings,
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
