// Headless screenshot harness for coffeeNetwork.
// Mocks the Tauri IPC layer, loads the real built UI, captures PNGs in two
// states. Output → .github/assets/screenshot-*.png
//
// Usage: npm run build && node scripts/make-screenshots.mjs
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DIST = path.join(ROOT, "dist");
const OUT = path.join(ROOT, ".github", "assets");
const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

mkdirSync(OUT, { recursive: true });

function harnessHTML(state) {
  const indexHtml = readFileSync(path.join(DIST, "index.html"), "utf8");
  const mock = `
<script>
window.__MOCK__ = ${JSON.stringify(state)};
window.__TAURI_INTERNALS__ = {
  invoke: (cmd) => {
    const M = window.__MOCK__;
    switch (cmd) {
      case "list_servers": return Promise.resolve(M.servers);
      case "get_settings": return Promise.resolve(M.settings);
      case "status": return Promise.resolve(M.status);
      case "check_update": return Promise.resolve({ available: false, version: null, notes: null });
      case "get_log": return Promise.resolve(M.log || "");
      case "preview_config": return Promise.resolve(M.config || "{}");
      default: return Promise.resolve(M.status);
    }
  },
  transformCallback: (cb) => cb,
};
</script>`;
  return indexHtml.replace("</head>", mock + "\n</head>");
}

const SERVERS = [
  { id: "1", name: "HY2-arti", protocol: "hysteria2", address: "77.73.135.131", port: 28443, raw: "" },
  { id: "2", name: "Amsterdam Reality", protocol: "vless", address: "nl-01.example.net", port: 443, raw: "" },
  { id: "3", name: "Tokyo TUIC", protocol: "tuic", address: "jp-tokyo.example.net", port: 8443, raw: "" },
  { id: "4", name: "Frankfurt SS", protocol: "shadowsocks", address: "de-fra.example.net", port: 8388, raw: "" },
];

const STATES = {
  connected: {
    servers: SERVERS,
    settings: { mode: "system_proxy", bypass_ru: true, active_server: "1" },
    status: { running: true, active_server: "1", mode: "system_proxy", bypass_ru: true, core_path: "/opt/homebrew/bin/sing-box" },
  },
  idle: {
    servers: SERVERS,
    settings: { mode: "tun", bypass_ru: true, active_server: null },
    status: { running: false, active_server: null, mode: "tun", bypass_ru: true, core_path: "/opt/homebrew/bin/sing-box" },
  },
};

for (const [name, st] of Object.entries(STATES)) {
  writeFileSync(path.join(DIST, `shot-${name}.html`), harnessHTML(st));
}

const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml" };
const server = http.createServer((req, res) => {
  let p = decodeURIComponent(req.url.split("?")[0]);
  if (p === "/") p = "/index.html";
  const fp = path.join(DIST, p);
  if (!existsSync(fp)) { res.writeHead(404); res.end("nf"); return; }
  res.writeHead(200, { "content-type": MIME[path.extname(fp)] || "application/octet-stream" });
  res.end(readFileSync(fp));
});

function shoot(url, out) {
  return new Promise((resolve, reject) => {
    const args = [
      "--headless=old", "--disable-gpu", "--hide-scrollbars",
      "--force-device-scale-factor=2",
      "--window-size=1000,700",
      "--default-background-color=00000000",
      `--screenshot=${out}`,
      "--virtual-time-budget=2000",
      url,
    ];
    const p = spawn(CHROME, args, { stdio: "ignore" });
    p.on("exit", (code) => (code === 0 ? resolve() : reject(new Error("chrome exit " + code))));
    p.on("error", reject);
  });
}

server.listen(4599, async () => {
  try {
    await shoot("http://localhost:4599/shot-connected.html", path.join(OUT, "screenshot-connected.png"));
    console.log("✓ screenshot-connected.png");
    await shoot("http://localhost:4599/shot-idle.html", path.join(OUT, "screenshot-idle.png"));
    console.log("✓ screenshot-idle.png");
  } catch (e) {
    console.error("screenshot failed:", e.message);
    process.exitCode = 1;
  } finally {
    server.close();
  }
});
