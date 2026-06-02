//! sing-box process lifecycle: start, stop, status, logs.
//!
//! Two launch paths:
//!   • System-proxy mode runs sing-box unprivileged ([`spawn_plain`]).
//!   • TUN mode needs admin/root, so it is launched elevated — via `osascript`
//!     on macOS and an elevated `Start-Process` on Windows ([`spawn_elevated`]).

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

use crate::parser::Server;
use crate::singbox::{self, Mode};
use crate::sysproxy;

/// Windows: hide the console window of spawned helper processes.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Default)]
pub struct CoreState {
    inner: Mutex<Running>,
}

#[derive(Default)]
struct Running {
    /// Child handle for non-elevated (system proxy) runs.
    child: Option<Child>,
    /// PID for elevated (TUN) runs launched via the OS elevation helper.
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
        } else if let Some(pid) = guard.elevated_pid {
            if pid_alive(pid) {
                true
            } else {
                guard.elevated_pid = None;
                guard.server_id = None;
                false
            }
        } else {
            false
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

/// Per-platform application-data root (macOS: ~/Library/Application Support,
/// Windows: %APPDATA%, Linux: ~/.local/share). The `coffeeNetwork` subdir is
/// appended by [`config_dir`].
fn dirs_config_home() -> Result<PathBuf, String> {
    dirs::data_dir().ok_or_else(|| "Не удалось определить каталог данных пользователя".to_string())
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

    let mut config = singbox::build_config(server, mode, bypass_ru, excluded);
    let cfg_path = config_path()?;
    let log_file = log_path()?;

    // Elevated launchers can't capture the child's stdout, so for TUN we point
    // sing-box's own logger at the file. System-proxy mode keeps capturing
    // stdout/stderr directly (catches even pre-logger startup errors).
    if matches!(mode, Mode::Tun) {
        if let Some(obj) = config.get_mut("log").and_then(|l| l.as_object_mut()) {
            obj.insert("output".into(), json!(log_file.to_string_lossy()));
        }
    }

    fs::write(
        &cfg_path,
        serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Не удалось записать конфиг: {e}"))?;

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

    let mut cmd = Command::new(bin);
    cmd.arg("run")
        .arg("-c")
        .arg(cfg)
        .current_dir(&workdir)
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .stdin(Stdio::null());
    // Don't flash a console window on Windows (GUI app spawning a console child).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("Не удалось запустить sing-box: {e}"))?;

    {
        let mut guard = state.inner.lock().unwrap();
        guard.child = Some(child);
        guard.mode = Some(mode);
        guard.server_id = Some(server.id.clone());
    }

    // Give the core time to bind (Windows + first-run Defender scan can be slow).
    // Poll instead of one fixed sleep so we fail fast if it dies, and tolerate a
    // slow-but-healthy start.
    if !wait_until_ready(state, Duration::from_millis(2500)) {
        let log = read_log().unwrap_or_default();
        return Err(format!("sing-box не запустился.\n{}", tail(&log, 12)));
    }
    Ok(())
}

/// TUN requires admin/root. Launch via the OS elevation helper (one prompt),
/// capture the elevated child's PID, and confirm it stayed up.
fn spawn_elevated(
    state: &CoreState,
    bin: &PathBuf,
    cfg: &PathBuf,
    _log_file: &PathBuf,
    server: &Server,
    mode: Mode,
) -> Result<(), String> {
    let workdir = config_dir()?;
    let pid = elevate_run(bin, cfg, &workdir)?;

    {
        let mut guard = state.inner.lock().unwrap();
        guard.elevated_pid = Some(pid);
        guard.mode = Some(mode);
        guard.server_id = Some(server.id.clone());
    }

    std::thread::sleep(Duration::from_millis(1200));
    if !pid_alive(pid) {
        {
            let mut guard = state.inner.lock().unwrap();
            guard.elevated_pid = None;
            guard.server_id = None;
        }
        kill_elevated(pid); // reap any half-started process
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
        kill_elevated(pid);
    }

    // Safety: make sure the system proxy isn't left dangling at a dead port
    // (a hard kill gives sing-box no chance to revert it).
    sysproxy::clear_all();
    Ok(())
}

/// Poll until the core is confirmed running, or `budget` elapses. Returns false
/// only if the process actually died within the budget.
fn wait_until_ready(state: &CoreState, budget: Duration) -> bool {
    let step = Duration::from_millis(150);
    let mut waited = Duration::ZERO;
    // Let it get going before the first check.
    std::thread::sleep(step);
    waited += step;
    while waited < budget {
        if !state.is_running() {
            return false;
        }
        std::thread::sleep(step);
        waited += step;
    }
    state.is_running()
}

// ---------------------------------------------------------------------------
// platform: elevation + liveness
// ---------------------------------------------------------------------------

/// macOS: one auth prompt via `osascript`. Backgrounds sing-box as root and
/// echoes its PID. Logging goes to the file configured in `log.output`.
#[cfg(target_os = "macos")]
fn elevate_run(bin: &PathBuf, cfg: &PathBuf, workdir: &PathBuf) -> Result<u32, String> {
    // `exec` so the backgrounded job *becomes* sing-box — `$!` is then the real
    // sing-box PID, so a later `kill` stops it instead of orphaning a root TUN
    // process behind a short-lived wrapper shell.
    let script = format!(
        "do shell script \"cd '{dir}' && exec '{bin}' run -c '{cfg}' >/dev/null 2>&1 & echo $!\" with administrator privileges",
        dir = workdir.display(),
        bin = bin.display(),
        cfg = cfg.display(),
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
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| "Не удалось получить PID процесса TUN".to_string())
}

/// Windows: elevate via `Start-Process -Verb RunAs` (one UAC prompt), run hidden,
/// and return the elevated child's PID. Logging goes to `log.output`.
#[cfg(target_os = "windows")]
fn elevate_run(bin: &PathBuf, cfg: &PathBuf, workdir: &PathBuf) -> Result<u32, String> {
    use std::os::windows::process::CommandExt;
    // The config arg is wrapped in literal double-quotes so sing-box receives a
    // single argument even when the path contains spaces (Start-Process joins
    // ArgumentList with spaces and does not re-quote elements).
    let ps = format!(
        "$ErrorActionPreference='Stop'; \
         $p = Start-Process -FilePath {bin} -ArgumentList @('run','-c',{cfg}) \
         -WorkingDirectory {dir} -WindowStyle Hidden -Verb RunAs -PassThru; \
         [Console]::Out.Write($p.Id)",
        bin = ps_quote(&bin.display().to_string()),
        cfg = ps_quote(&format!("\"{}\"", cfg.display())),
        dir = ps_quote(&workdir.display().to_string()),
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("powershell: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Не удалось запустить TUN (нужны права администратора): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| "Не удалось получить PID процесса TUN".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn elevate_run(_bin: &PathBuf, _cfg: &PathBuf, _workdir: &PathBuf) -> Result<u32, String> {
    Err("Режим TUN на этой платформе не поддерживается".to_string())
}

/// Wrap a string as a PowerShell single-quoted literal (doubling inner quotes).
#[cfg(target_os = "windows")]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// macOS: existence check that works for root-owned processes too. `kill -0`
/// returns EPERM (not success) for a process the caller can't signal, so we use
/// `ps -p`, which reports existence regardless of ownership.
#[cfg(target_os = "macos")]
fn pid_alive(pid: u32) -> bool {
    // `ps -p <pid>` exits 0 iff a matching process exists (any owner), 1 otherwise.
    Command::new("/bin/ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("pid=")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Windows: existence check via `tasklist`. Matching row contains the PID; a
/// no-match prints an "INFO: No tasks…" line that never contains the number.
#[cfg(target_os = "windows")]
fn pid_alive(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("tasklist")
        .args(["/NH", "/FI"])
        .arg(format!("PID eq {pid}"))
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn pid_alive(_pid: u32) -> bool {
    false
}

/// macOS: kill the root sing-box via an elevated `osascript`.
#[cfg(target_os = "macos")]
fn kill_elevated(pid: u32) {
    let script = format!(
        "do shell script \"kill {pid} 2>/dev/null || kill -9 {pid}\" with administrator privileges"
    );
    let _ = Command::new("/usr/bin/osascript").arg("-e").arg(&script).output();
}

/// Windows: kill the elevated sing-box (and its tree) via an elevated `taskkill`.
#[cfg(target_os = "windows")]
fn kill_elevated(pid: u32) {
    use std::os::windows::process::CommandExt;
    let ps = format!(
        "Start-Process -FilePath 'taskkill' -ArgumentList @('/PID','{pid}','/T','/F') \
         -Verb RunAs -WindowStyle Hidden -Wait"
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn kill_elevated(_pid: u32) {}

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
