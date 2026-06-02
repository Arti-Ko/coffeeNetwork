//! Safety net for system-proxy state.
//!
//! sing-box's `set_system_proxy` normally reverts on clean exit, but if the
//! core is killed hard the proxy can be left pointing at a dead port — breaking
//! all networking until the next connect. These helpers force the OS proxy
//! back off. Best-effort, platform-specific.

#[cfg(target_os = "macos")]
use std::process::Command;

/// macOS: list the names of all network services (e.g. "Wi-Fi", "Ethernet").
#[cfg(target_os = "macos")]
fn network_services() -> Vec<String> {
    let out = Command::new("/usr/sbin/networksetup")
        .arg("-listallnetworkservices")
        .output();
    let Ok(out) = out else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1) // first line is an explanatory header
        .map(|s| s.trim_start_matches('*').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// macOS: turn off web/secure/socks proxies on every service.
#[cfg(target_os = "macos")]
pub fn clear_all() {
    for svc in network_services() {
        for kind in ["-setwebproxystate", "-setsecurewebproxystate", "-setsocksfirewallproxystate"] {
            let _ = Command::new("/usr/sbin/networksetup")
                .args([kind, &svc, "off"])
                .output();
        }
    }
}

/// Windows: reset the WinINET (system) proxy registry so a dangling proxy from a
/// hard-killed core doesn't black-hole all traffic. Applications pick up the
/// change on their next request.
#[cfg(target_os = "windows")]
pub fn clear_all() {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    let _ = Command::new("reg")
        .args(["add", KEY, "/v", "ProxyEnable", "/t", "REG_DWORD", "/d", "0", "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = Command::new("reg")
        .args(["delete", KEY, "/v", "ProxyServer", "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

/// Other platforms: nothing to clear.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn clear_all() {}
