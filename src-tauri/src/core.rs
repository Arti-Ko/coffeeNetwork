//! sing-box process lifecycle: start, stop, status, logs.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use crate::parser::Server;
use crate::singbox::{self, Mode};
use crate::sysproxy;

#[derive(Default)]
pub struct CoreState {
    inner: Mutex<Running>,
}

#[derive(Default)]
struct Running {
    /// Child handle for non-elevated (system proxy) runs.
    child: Option<Child>,
    /// PID for elevated (TUN) runs launched via osascript.
    elevated_pid: Option<u32>,
    mode: Option<Mode>,
    server_id: Option<String>,
}

impl CoreState {
    pub fn is_running(&self) -> bool {
        let mut guard = self.inner.lock().unwrap();
        if let Some(child) = guard.child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    guard.child = None;
                    guard.server_id = None;
                    false
                }
                _ => true,
            }
        } else {
            guard.elevated_pid.is_some()
        }
    }

    pub fn active_server(&self) -> Option<String> {
        self.inner.lock().unwrap().server_id.clone()
    }

    pub fn active_mode(&self) -> Option<Mode> {
        self.inner.lock().unwrap().mode
    }
}

fn config_dir() -> Result<PathBuf, String> {
    let base = dirs_config_home()?.join("coffeeNetwork");
    fs::create_dir_all(&base).map_err(|e| format!("Не удалось создать каталог конфигурации: {e}"))?;
    Ok(base)
}

/// $HOME/Library/Application Support on macOS.
fn dirs_config_home() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "Переменная HOME не задана".to_string())?;
    Ok(PathBuf::from(home).join("Library/Application Support"))
}

fn config_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("config.json"))
}

fn log_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("core.log"))
}

/// Generate config, write it to disk, and launch sing-box.
pub fn connect(
    state: &CoreState,
    server: &Server,
    mode: Mode,
    bypass_ru: bool,
    excluded: &[String],
) -> Result<(), String> {
    stop(state)?; // ensure clean slate

    let bin = singbox::locate_binary()
        .ok_or("Ядро sing-box не найдено. Установите его: brew install sing-box")?;

    let config = singbox::build_config(server, mode, bypass_ru, excluded);
    let cfg_path = config_path()?;
    fs::write(
        &cfg_path,
        serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Не удалось записать конфиг: {e}"))?;

    let log_file = log_path()?;
    let _ = fs::write(&log_file, b""); // truncate previous log

    match mode {
        Mode::SystemProxy => spawn_plain(state, &bin, &cfg_path, &log_file, server, mode),
        Mode::Tun => spawn_elevated(state, &bin, &cfg_path, &log_file, server, mode),
    }
}

fn spawn_plain(
    state: &CoreState,
    bin: &PathBuf,
    cfg: &PathBuf,
    log_file: &PathBuf,
    server: &Server,
    mode: Mode,
) -> Result<(), String> {
    let out = fs::File::create(log_file).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;

    // Run from the writable config dir so sing-box can create its `cache.db`
    // (experimental.cache_file). Launched from Finder the CWD is `/`, which is
    // read-only → "open cache.db: read-only file system" and the core dies.
    let workdir = config_dir()?;

    let child = Command::new(bin)
        .arg("run")
        .arg("-c")
        .arg(cfg)
        .current_dir(&workdir)
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("Не удалось запустить sing-box: {e}"))?;

    {
        let mut guard = state.inner.lock().unwrap();
        guard.child = Some(child);
        guard.mode = Some(mode);
        guard.server_id = Some(server.id.clone());
    }

    // Give the core a moment; if it died immediately, surface the log.
    std::thread::sleep(Duration::from_millis(700));
    if !state.is_running() {
        let log = read_log().unwrap_or_default();
        return Err(format!(
            "sing-box не запустился.\n{}",
            tail(&log, 12)
        ));
    }
    Ok(())
}

/// TUN requires root: launch via osascript so macOS shows one auth prompt.
fn spawn_elevated(
    state: &CoreState,
    bin: &PathBuf,
    cfg: &PathBuf,
    log_file: &PathBuf,
    server: &Server,
    mode: Mode,
) -> Result<(), String> {
    // cd into the writable config dir so sing-box can create cache.db there
    // (otherwise CWD is `/` and cache-file init fails on the read-only fs).
    let workdir = config_dir()?;
    let script = format!(
        "do shell script \"cd '{dir}' && '{bin}' run -c '{cfg}' > '{log}' 2>&1 & echo $!\" with administrator privileges",
        dir = workdir.display(),
        bin = bin.display(),
        cfg = cfg.display(),
        log = log_file.display(),
    );

    let out = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| format!("osascript: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "Не удалось запустить TUN (нужны права администратора): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let pid: u32 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| "Не удалось получить PID процесса TUN".to_string())?;

    {
        let mut guard = state.inner.lock().unwrap();
        guard.elevated_pid = Some(pid);
        guard.mode = Some(mode);
        guard.server_id = Some(server.id.clone());
    }

    std::thread::sleep(Duration::from_millis(900));
    if !pid_alive(pid) {
        let mut guard = state.inner.lock().unwrap();
        guard.elevated_pid = None;
        guard.server_id = None;
        drop(guard);
        let log = read_log().unwrap_or_default();
        return Err(format!("TUN-ядро не запустилось.\n{}", tail(&log, 12)));
    }
    Ok(())
}

/// Stop sing-box and restore proxy state.
pub fn stop(state: &CoreState) -> Result<(), String> {
    let (child, pid) = {
        let mut guard = state.inner.lock().unwrap();
        guard.mode = None;
        guard.server_id = None;
        (guard.child.take(), guard.elevated_pid.take())
    };

    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(pid) = pid {
        // root process — kill via osascript admin.
        let script = format!(
            "do shell script \"kill {pid} 2>/dev/null || kill -9 {pid}\" with administrator privileges"
        );
        let _ = Command::new("/usr/bin/osascript").arg("-e").arg(&script).output();
    }

    // Safety: make sure the system proxy isn't left dangling.
    sysproxy::clear_all();
    Ok(())
}

fn pid_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Read the current core log (tail handled by caller / UI).
pub fn read_log() -> Result<String, String> {
    let path = log_path()?;
    let mut s = String::new();
    if let Ok(mut f) = fs::File::open(&path) {
        let _ = f.read_to_string(&mut s);
    }
    Ok(s)
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}
