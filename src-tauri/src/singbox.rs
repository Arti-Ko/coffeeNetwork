//! sing-box config generation + process lifecycle.
//!
//! Targets sing-box 1.11+ schema (route `action` rules + remote rule-sets).
//!
//! Two routing layouts share one principle: Russian domains/IPs and private
//! networks go DIRECT (no VPN), everything else goes through the selected
//! proxy outbound.

use serde_json::{json, Value};
use std::path::PathBuf;

use crate::parser::{Server, PROXY_TAG};

/// Where the user's traffic is captured.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// macOS system proxy (mixed SOCKS+HTTP). No root required.
    SystemProxy,
    /// TUN device — captures all traffic. Requires root.
    Tun,
}

const MIXED_PORT: u16 = 2080;

// SagerNet rule-set sources (binary .srs).
const RS_GEOSITE_RU: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ru.srs";
const RS_GEOIP_RU: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-ru.srs";
const RS_GEOSITE_PRIVATE: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-private.srs";

/// Build a complete sing-box configuration for `server` in the given `mode`.
///
/// `bypass_ru` toggles the RU→direct split. When false, *everything* goes
/// through the proxy (private networks still stay direct).
pub fn build_config(server: &Server, mode: Mode, bypass_ru: bool) -> Value {
    let mut proxy = server.outbound.clone();
    proxy["tag"] = json!(PROXY_TAG);

    let inbounds = match mode {
        Mode::SystemProxy => json!([{
            "type": "mixed",
            "tag": "mixed-in",
            "listen": "127.0.0.1",
            "listen_port": MIXED_PORT,
            "set_system_proxy": true
        }]),
        Mode::Tun => json!([{
            "type": "tun",
            "tag": "tun-in",
            "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
            "auto_route": true,
            "strict_route": true,
            "stack": "system"
        }]),
    };

    json!({
        "log": { "level": "warn", "timestamp": true },
        "dns": build_dns(bypass_ru),
        "inbounds": inbounds,
        "outbounds": [
            proxy,
            { "type": "direct", "tag": "direct" }
        ],
        "route": build_route(mode, bypass_ru),
        "experimental": {
            "cache_file": { "enabled": true, "store_rdrc": true }
        }
    })
}

fn build_dns(bypass_ru: bool) -> Value {
    // Remote DoH resolves through the proxy (no leak); RU domains resolve via a
    // Russian resolver directly so geo-targeting and RU CDNs behave correctly.
    let mut rules = vec![];
    if bypass_ru {
        rules.push(json!({ "rule_set": ["geosite-category-ru"], "server": "local-ru" }));
    }

    json!({
        "servers": [
            { "tag": "remote", "address": "https://1.1.1.1/dns-query", "detour": PROXY_TAG },
            { "tag": "local-ru", "address": "https://77.88.8.8/dns-query", "detour": "direct" }
        ],
        "rules": rules,
        "final": "remote",
        "strategy": "prefer_ipv4",
        "independent_cache": true
    })
}

fn build_route(mode: Mode, bypass_ru: bool) -> Value {
    let mut rules = vec![
        json!({ "action": "sniff" }),
        json!({ "protocol": "dns", "action": "hijack-dns" }),
        // private networks never go through the tunnel
        json!({ "ip_is_private": true, "outbound": "direct" }),
        json!({ "rule_set": ["geosite-private"], "outbound": "direct" }),
    ];

    let mut rule_set = vec![
        remote_rule_set("geosite-private", RS_GEOSITE_PRIVATE),
    ];

    if bypass_ru {
        rules.push(json!({
            "rule_set": ["geosite-category-ru", "geoip-ru"],
            "outbound": "direct"
        }));
        rule_set.push(remote_rule_set("geosite-category-ru", RS_GEOSITE_RU));
        rule_set.push(remote_rule_set("geoip-ru", RS_GEOIP_RU));
    }

    // TUN benefits from resolving sniffed domains to IPs before geoip matching.
    let resolve = matches!(mode, Mode::Tun);

    json!({
        "rules": rules,
        "rule_set": rule_set,
        "final": PROXY_TAG,
        "auto_detect_interface": true,
        "default_domain_resolver": if resolve { json!("local-ru") } else { Value::Null }
    })
}

fn remote_rule_set(tag: &str, url: &str) -> Value {
    json!({
        "type": "remote",
        "tag": tag,
        "format": "binary",
        "url": url,
        "download_detour": "direct",
        "update_interval": "72h"
    })
}

/// Resolve the sing-box binary.
///
/// Priority:
/// 1. The bundled sidecar shipped inside the app (next to the executable) —
///    so users never have to install sing-box themselves.
/// 2. A system install (Homebrew / PATH) — handy in `tauri dev` and for power
///    users who prefer their own sing-box.
pub fn locate_binary() -> Option<PathBuf> {
    let bin_name = if cfg!(windows) { "sing-box.exe" } else { "sing-box" };

    // 1. Bundled sidecar — Tauri places externalBin next to the main binary
    //    (macOS: Contents/MacOS/, Windows: alongside the .exe).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(bin_name);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 2. System install.
    let candidates = [
        "/opt/homebrew/bin/sing-box",
        "/usr/local/bin/sing-box",
        "/usr/bin/sing-box",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    // Fall back to a PATH lookup.
    if let Ok(path) = std::env::var("PATH") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for dir in path.split(sep) {
            let p = PathBuf::from(dir).join(bin_name);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}
