//! Parsers for proxy share-links → sing-box outbound objects.
//!
//! Supported schemes: vless://, hysteria2:// (hy2://), vmess://,
//! ss:// (shadowsocks), trojan://, tuic://.
//!
//! Each parser returns a `Server` whose `outbound` is a ready-to-use
//! sing-box outbound JSON value tagged `"proxy"`.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use url::Url;

pub const PROXY_TAG: &str = "proxy";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub address: String,
    pub port: u16,
    /// The sing-box outbound object, tag = "proxy".
    pub outbound: Value,
    /// Original share-link, kept so we can re-export later.
    pub raw: String,
}

/// Parse a single share-link into a `Server`. `id` is left empty; the store
/// assigns one on save.
pub fn parse_link(link: &str) -> Result<Server, String> {
    // Strip ALL whitespace, not just the ends: a share-link never legitimately
    // contains spaces (they must be %20-encoded), and pasted/synced links can
    // carry stray spaces or newlines that silently corrupt the SNI/pbk — parity
    // with the mobile client, where keyboard autocorrect injects them.
    let cleaned: String = link.chars().filter(|c| !c.is_whitespace()).collect();
    let link = cleaned.as_str();
    let scheme = link.split("://").next().unwrap_or("").to_lowercase();

    match scheme.as_str() {
        "vless" => parse_vless(link),
        "hysteria2" | "hy2" => parse_hysteria2(link),
        "vmess" => parse_vmess(link),
        "ss" => parse_shadowsocks(link),
        "trojan" => parse_trojan(link),
        "tuic" => parse_tuic(link),
        other => Err(format!("Неподдерживаемый протокол: {other}")),
    }
}

/// Parse a blob of text (possibly a base64 subscription) into many servers.
/// Silently skips lines that fail to parse.
pub fn parse_many(input: &str) -> Vec<Server> {
    let text = maybe_decode_subscription(input);
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.contains("://"))
        .filter_map(|l| parse_link(l).ok())
        .collect()
}

/// A subscription body is often base64 of newline-separated links.
fn maybe_decode_subscription(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.contains("://") {
        return trimmed.to_string();
    }
    if let Some(decoded) = try_b64(trimmed) {
        if decoded.contains("://") {
            return decoded;
        }
    }
    trimmed.to_string()
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn try_b64(s: &str) -> Option<String> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    for engine in [
        &STANDARD as &dyn DynEngine,
        &STANDARD_NO_PAD,
        &URL_SAFE_NO_PAD,
    ] {
        if let Ok(bytes) = engine.dyn_decode(&cleaned) {
            if let Ok(text) = String::from_utf8(bytes) {
                return Some(text);
            }
        }
    }
    None
}

// Small trait so we can iterate over heterogeneous base64 engines.
trait DynEngine {
    fn dyn_decode(&self, s: &str) -> Result<Vec<u8>, ()>;
}
impl<T: Engine> DynEngine for T {
    fn dyn_decode(&self, s: &str) -> Result<Vec<u8>, ()> {
        self.decode(s).map_err(|_| ())
    }
}

fn query_map(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn fragment_name(url: &Url, fallback: &str) -> String {
    url.fragment()
        .map(|f| urlencoding::decode(f).map(|c| c.into_owned()).unwrap_or_else(|_| f.to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn require_host(url: &Url) -> Result<String, String> {
    url.host_str()
        .map(|h| h.trim_matches(|c| c == '[' || c == ']').to_string())
        .ok_or_else(|| "В ссылке нет адреса сервера".to_string())
}

fn require_port(url: &Url, default: u16) -> u16 {
    url.port().unwrap_or(default)
}

fn server(name: String, protocol: &str, address: String, port: u16, outbound: Value, raw: &str) -> Server {
    Server {
        id: String::new(),
        name,
        protocol: protocol.to_string(),
        address,
        port,
        outbound,
        raw: raw.to_string(),
    }
}

/// Build a sing-box TLS object from common query parameters.
fn build_tls(q: &HashMap<String, String>, default_sni: &str) -> Option<Value> {
    let security = q.get("security").map(|s| s.as_str()).unwrap_or("");
    let has_reality = q.contains_key("pbk");
    if security != "tls" && security != "reality" && security != "xtls" && !has_reality {
        return None;
    }

    let sni = q
        .get("sni")
        .or_else(|| q.get("peer"))
        .or_else(|| q.get("host"))
        .cloned()
        .unwrap_or_else(|| default_sni.to_string());

    let mut tls = json!({
        "enabled": true,
        "server_name": sni,
    });

    if let Some(alpn) = q.get("alpn") {
        let list: Vec<&str> = alpn.split(',').filter(|s| !s.is_empty()).collect();
        if !list.is_empty() {
            tls["alpn"] = json!(list);
        }
    }

    let insecure = q.get("allowInsecure").or_else(|| q.get("insecure")).map(|v| v == "1" || v == "true");
    if insecure == Some(true) {
        tls["insecure"] = json!(true);
    }

    // uTLS fingerprint
    let fp = q.get("fp").cloned().unwrap_or_else(|| "chrome".to_string());
    tls["utls"] = json!({ "enabled": true, "fingerprint": fp });

    // REALITY
    if has_reality {
        let mut reality = json!({
            "enabled": true,
            "public_key": q.get("pbk").cloned().unwrap_or_default(),
        });
        if let Some(sid) = q.get("sid") {
            reality["short_id"] = json!(sid);
        }
        tls["reality"] = reality;
        // REALITY always pairs with uTLS; insecure is meaningless there.
        tls.as_object_mut().unwrap().remove("insecure");
    }

    Some(tls)
}

/// Build a sing-box v2ray transport object (ws / grpc / http / httpupgrade).
fn build_transport(q: &HashMap<String, String>) -> Option<Value> {
    let net = q.get("type").map(|s| s.as_str()).unwrap_or("tcp");
    match net {
        "ws" => {
            let mut t = json!({ "type": "ws" });
            if let Some(path) = q.get("path") {
                t["path"] = json!(path);
            }
            if let Some(host) = q.get("host") {
                t["headers"] = json!({ "Host": host });
            }
            Some(t)
        }
        "grpc" => {
            let svc = q.get("serviceName").cloned().unwrap_or_default();
            Some(json!({ "type": "grpc", "service_name": svc }))
        }
        "http" | "h2" => {
            let mut t = json!({ "type": "http" });
            if let Some(path) = q.get("path") {
                t["path"] = json!(path);
            }
            if let Some(host) = q.get("host") {
                let hosts: Vec<&str> = host.split(',').collect();
                t["host"] = json!(hosts);
            }
            Some(t)
        }
        "httpupgrade" => {
            let mut t = json!({ "type": "httpupgrade" });
            if let Some(path) = q.get("path") {
                t["path"] = json!(path);
            }
            if let Some(host) = q.get("host") {
                t["host"] = json!(host);
            }
            Some(t)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// VLESS
// ---------------------------------------------------------------------------

fn parse_vless(link: &str) -> Result<Server, String> {
    let url = Url::parse(link).map_err(|e| format!("Неверная vless-ссылка: {e}"))?;
    let uuid = url.username().to_string();
    if uuid.is_empty() {
        return Err("vless: отсутствует UUID".into());
    }
    let host = require_host(&url)?;
    let port = require_port(&url, 443);
    let q = query_map(&url);

    let mut outbound = json!({
        "type": "vless",
        "tag": PROXY_TAG,
        "server": host,
        "server_port": port,
        "uuid": uuid,
    });

    if let Some(flow) = q.get("flow") {
        if !flow.is_empty() {
            outbound["flow"] = json!(flow);
        }
    }
    if let Some(tls) = build_tls(&q, &host) {
        outbound["tls"] = tls;
    }
    if let Some(transport) = build_transport(&q) {
        outbound["transport"] = transport;
    }

    let name = fragment_name(&url, &format!("VLESS {host}"));
    Ok(server(name, "vless", host, port, outbound, link))
}

// ---------------------------------------------------------------------------
// Hysteria2
// ---------------------------------------------------------------------------

fn parse_hysteria2(link: &str) -> Result<Server, String> {
    let url = Url::parse(link).map_err(|e| format!("Неверная hysteria2-ссылка: {e}"))?;
    let host = require_host(&url)?;
    let port = require_port(&url, 443);
    let q = query_map(&url);

    // Hysteria2 auth is a single string in userinfo. It may itself contain a
    // colon (e.g. `user:pass`), which `url` splits into username/password — so
    // we rejoin them to recover the original auth string verbatim.
    let mut password = url.username().to_string();
    if let Some(p) = url.password() {
        password = format!("{password}:{p}");
    }
    let password = urlencoding::decode(&password).map(|c| c.into_owned()).unwrap_or(password);

    let sni = q
        .get("sni")
        .or_else(|| q.get("peer"))
        .cloned()
        .unwrap_or_else(|| host.clone());
    let insecure = q.get("insecure").map(|v| v == "1" || v == "true").unwrap_or(false);

    let mut tls = json!({
        "enabled": true,
        "server_name": sni,
        "alpn": ["h3"],
    });
    if insecure {
        tls["insecure"] = json!(true);
    }
    if let Some(pin) = q.get("pinSHA256") {
        tls["certificate_public_key_sha256"] = json!(pin);
    }

    let mut outbound = json!({
        "type": "hysteria2",
        "tag": PROXY_TAG,
        "server": host,
        "server_port": port,
        "password": password,
        "tls": tls,
    });

    if let Some(up) = q.get("up").and_then(|v| v.parse::<u32>().ok()) {
        outbound["up_mbps"] = json!(up);
    }
    if let Some(down) = q.get("down").and_then(|v| v.parse::<u32>().ok()) {
        outbound["down_mbps"] = json!(down);
    }
    if let Some(obfs_pw) = q.get("obfs-password") {
        let obfs_type = q.get("obfs").cloned().unwrap_or_else(|| "salamander".to_string());
        outbound["obfs"] = json!({ "type": obfs_type, "password": obfs_pw });
    }

    let name = fragment_name(&url, &format!("Hysteria2 {host}"));
    Ok(server(name, "hysteria2", host, port, outbound, link))
}

// ---------------------------------------------------------------------------
// VMess (base64 JSON)
// ---------------------------------------------------------------------------

fn parse_vmess(link: &str) -> Result<Server, String> {
    let body = link.trim_start_matches("vmess://");
    let decoded = try_b64(body).ok_or("vmess: не удалось декодировать base64")?;
    let v: Value = serde_json::from_str(&decoded).map_err(|e| format!("vmess: неверный JSON: {e}"))?;

    let host = v["add"].as_str().unwrap_or("").to_string();
    if host.is_empty() {
        return Err("vmess: отсутствует адрес".into());
    }
    let port = string_or_num_to_u16(&v["port"]).unwrap_or(443);
    let uuid = v["id"].as_str().unwrap_or("").to_string();
    let aid = string_or_num_to_u16(&v["aid"]).unwrap_or(0);
    let net = v["net"].as_str().unwrap_or("tcp");
    let security = v["scy"].as_str().filter(|s| !s.is_empty()).unwrap_or("auto");

    let mut outbound = json!({
        "type": "vmess",
        "tag": PROXY_TAG,
        "server": host,
        "server_port": port,
        "uuid": uuid,
        "alter_id": aid,
        "security": security,
    });

    if v["tls"].as_str() == Some("tls") {
        let sni = v["sni"].as_str().filter(|s| !s.is_empty()).unwrap_or(&host).to_string();
        outbound["tls"] = json!({
            "enabled": true,
            "server_name": sni,
            "utls": { "enabled": true, "fingerprint": "chrome" }
        });
    }

    // transport from vmess json fields
    let mut q: HashMap<String, String> = HashMap::new();
    q.insert("type".into(), net.to_string());
    if let Some(p) = v["path"].as_str() {
        q.insert("path".into(), p.to_string());
    }
    if let Some(h) = v["host"].as_str() {
        if !h.is_empty() {
            q.insert("host".into(), h.to_string());
        }
    }
    if net == "grpc" {
        if let Some(p) = v["path"].as_str() {
            q.insert("serviceName".into(), p.to_string());
        }
    }
    if let Some(transport) = build_transport(&q) {
        outbound["transport"] = transport;
    }

    let name = v["ps"].as_str().filter(|s| !s.is_empty()).unwrap_or(&host).to_string();
    Ok(server(name, "vmess", host, port, outbound, link))
}

fn string_or_num_to_u16(v: &Value) -> Option<u16> {
    if let Some(n) = v.as_u64() {
        return u16::try_from(n).ok();
    }
    v.as_str().and_then(|s| s.parse::<u16>().ok())
}

// ---------------------------------------------------------------------------
// Shadowsocks
// ---------------------------------------------------------------------------

fn parse_shadowsocks(link: &str) -> Result<Server, String> {
    // Two layouts:
    //   ss://base64(method:password)@host:port#name
    //   ss://base64(method:password@host:port)#name
    let without_scheme = link.trim_start_matches("ss://");
    let (main, frag) = match without_scheme.split_once('#') {
        Some((m, f)) => (m, Some(f)),
        None => (without_scheme, None),
    };

    let (method, password, host, port);
    if let Some((userinfo, hostport)) = main.split_once('@') {
        // method:password is base64 (sometimes plain)
        let creds = try_b64(userinfo).unwrap_or_else(|| userinfo.to_string());
        let (m, p) = creds
            .split_once(':')
            .ok_or("ss: неверный формат method:password")?;
        method = m.to_string();
        password = p.to_string();
        let hp = hostport.split('?').next().unwrap_or(hostport);
        let (h, pt) = hp.rsplit_once(':').ok_or("ss: неверный host:port")?;
        host = h.trim_matches(|c| c == '[' || c == ']').to_string();
        port = pt.parse::<u16>().map_err(|_| "ss: неверный порт")?;
    } else {
        // whole thing is base64
        let decoded = try_b64(main).ok_or("ss: не удалось декодировать")?;
        let (creds, hostport) = decoded.split_once('@').ok_or("ss: неверный формат")?;
        let (m, p) = creds.split_once(':').ok_or("ss: неверный method:password")?;
        method = m.to_string();
        password = p.to_string();
        let (h, pt) = hostport.rsplit_once(':').ok_or("ss: неверный host:port")?;
        host = h.trim_matches(|c| c == '[' || c == ']').to_string();
        port = pt.parse::<u16>().map_err(|_| "ss: неверный порт")?;
    }

    let outbound = json!({
        "type": "shadowsocks",
        "tag": PROXY_TAG,
        "server": host,
        "server_port": port,
        "method": method,
        "password": password,
    });

    let name = frag
        .map(|f| urlencoding::decode(f).map(|c| c.into_owned()).unwrap_or_else(|_| f.to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("SS {host}"));
    Ok(server(name, "shadowsocks", host, port, outbound, link))
}

// ---------------------------------------------------------------------------
// Trojan
// ---------------------------------------------------------------------------

fn parse_trojan(link: &str) -> Result<Server, String> {
    let url = Url::parse(link).map_err(|e| format!("Неверная trojan-ссылка: {e}"))?;
    let password = url.username().to_string();
    let host = require_host(&url)?;
    let port = require_port(&url, 443);
    let q = query_map(&url);

    let sni = q.get("sni").or_else(|| q.get("peer")).cloned().unwrap_or_else(|| host.clone());
    let insecure = q.get("allowInsecure").or_else(|| q.get("insecure")).map(|v| v == "1" || v == "true").unwrap_or(false);
    let mut tls = json!({
        "enabled": true,
        "server_name": sni,
        "utls": { "enabled": true, "fingerprint": q.get("fp").cloned().unwrap_or_else(|| "chrome".into()) }
    });
    if insecure {
        tls["insecure"] = json!(true);
    }
    if let Some(alpn) = q.get("alpn") {
        let list: Vec<&str> = alpn.split(',').filter(|s| !s.is_empty()).collect();
        if !list.is_empty() {
            tls["alpn"] = json!(list);
        }
    }

    let mut outbound = json!({
        "type": "trojan",
        "tag": PROXY_TAG,
        "server": host,
        "server_port": port,
        "password": password,
        "tls": tls,
    });
    if let Some(transport) = build_transport(&q) {
        outbound["transport"] = transport;
    }

    let name = fragment_name(&url, &format!("Trojan {host}"));
    Ok(server(name, "trojan", host, port, outbound, link))
}

// ---------------------------------------------------------------------------
// TUIC v5
// ---------------------------------------------------------------------------

fn parse_tuic(link: &str) -> Result<Server, String> {
    let url = Url::parse(link).map_err(|e| format!("Неверная tuic-ссылка: {e}"))?;
    let uuid = url.username().to_string();
    let password = url.password().unwrap_or("").to_string();
    let host = require_host(&url)?;
    let port = require_port(&url, 443);
    let q = query_map(&url);

    let sni = q.get("sni").cloned().unwrap_or_else(|| host.clone());
    let insecure = q.get("allow_insecure").or_else(|| q.get("insecure")).map(|v| v == "1" || v == "true").unwrap_or(false);
    let mut tls = json!({ "enabled": true, "server_name": sni });
    if insecure {
        tls["insecure"] = json!(true);
    }
    if let Some(alpn) = q.get("alpn") {
        let list: Vec<&str> = alpn.split(',').filter(|s| !s.is_empty()).collect();
        if !list.is_empty() {
            tls["alpn"] = json!(list);
        }
    } else {
        tls["alpn"] = json!(["h3"]);
    }

    let mut outbound = json!({
        "type": "tuic",
        "tag": PROXY_TAG,
        "server": host,
        "server_port": port,
        "uuid": uuid,
        "password": password,
        "tls": tls,
    });
    if let Some(cc) = q.get("congestion_control") {
        outbound["congestion_control"] = json!(cc);
    } else {
        outbound["congestion_control"] = json!("bbr");
    }
    if let Some(udp) = q.get("udp_relay_mode") {
        outbound["udp_relay_mode"] = json!(udp);
    }

    let name = fragment_name(&url, &format!("TUIC {host}"));
    Ok(server(name, "tuic", host, port, outbound, link))
}
