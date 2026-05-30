//! Safety net for macOS system proxy state.
//!
//! sing-box's `set_system_proxy` normally reverts on clean exit, but if the
//! core is killed hard the proxy can be left pointing at a dead port. These
//! helpers force every network service back to "no proxy".

use std::process::Command;

/// List the names of all network services (e.g. "Wi-Fi", "Ethernet").
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

/// Turn off web/secure/socks proxies on every service. Best-effort.
pub fn clear_all() {
    for svc in network_services() {
        for kind in ["-setwebproxystate", "-setsecurewebproxystate", "-setsocksfirewallproxystate"] {
            let _ = Command::new("/usr/sbin/networksetup")
                .args([kind, &svc, "off"])
                .output();
        }
    }
}
