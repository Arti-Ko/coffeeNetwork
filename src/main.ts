import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// types (mirror the Rust structs)
// ---------------------------------------------------------------------------
type Mode = "system_proxy" | "tun";

interface Server {
  id: string;
  name: string;
  protocol: string;
  address: string;
  port: number;
  raw: string;
}

interface Status {
  running: boolean;
  active_server: string | null;
  mode: Mode | null;
  bypass_ru: boolean;
  core_path: string | null;
}

interface Settings {
  mode: Mode;
  bypass_ru: boolean;
  active_server: string | null;
  accent: string; // named preset ("amber"…) or hex "#rrggbb"
  accent2: string; // secondary accent (background glow) — same value space
  theme: string; // "dark" | "light" | "system"
  excluded_apps: string[]; // process names that bypass the VPN
}

// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------
let servers: Server[] = [];
let settings: Settings = {
  mode: "system_proxy",
  bypass_ru: true,
  active_server: null,
  accent: "amber",
  accent2: "amber",
  theme: "dark",
  excluded_apps: [],
};
let status: Status = { running: false, active_server: null, mode: null, bypass_ru: true, core_path: null };
let selectedId: string | null = null;
let busy = false;
let logTimer: number | null = null;

// ---------------------------------------------------------------------------
// dom helpers
// ---------------------------------------------------------------------------
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

function toast(message: string, isError = false) {
  const el = $("toast");
  el.textContent = message;
  el.classList.toggle("err", isError);
  el.hidden = false;
  requestAnimationFrame(() => el.classList.add("show"));
  window.setTimeout(() => {
    el.classList.remove("show");
    window.setTimeout(() => (el.hidden = true), 250);
  }, 3200);
}

function esc(s: string): string {
  const d = document.createElement("div");
  d.textContent = s;
  return d.innerHTML;
}

// ---------------------------------------------------------------------------
// appearance — accent color + light/dark/system theme
// ---------------------------------------------------------------------------
/** Named accent presets → oklch (fixed L/C, varied hue). */
const ACCENTS: Record<string, string> = {
  amber: "oklch(78% 0.15 72)",
  coffee: "oklch(62% 0.11 50)",
  green: "oklch(76% 0.16 150)",
  teal: "oklch(74% 0.13 195)",
  blue: "oklch(70% 0.15 250)",
  violet: "oklch(68% 0.18 300)",
  pink: "oklch(72% 0.18 350)",
  red: "oklch(64% 0.2 25)",
};

function hexToRgba(hex: string, a: number): string {
  const h = hex.replace("#", "");
  const n = parseInt(h.length === 3 ? h.replace(/(.)/g, "$1$1") : h, 16);
  const r = (n >> 16) & 255,
    g = (n >> 8) & 255,
    b = n & 255;
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

/** Perceived luminance 0..1 to pick readable ink over a custom accent. */
function luminance(hex: string): number {
  const h = hex.replace("#", "");
  const n = parseInt(h.length === 3 ? h.replace(/(.)/g, "$1$1") : h, 16);
  const r = ((n >> 16) & 255) / 255,
    g = ((n >> 8) & 255) / 255,
    b = (n & 255) / 255;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Resolve a preset name or hex value to a concrete color string. */
function accentColor(value: string): string {
  return value.startsWith("#") ? value : ACCENTS[value] ?? ACCENTS.amber;
}

/** Same color at a given alpha — works for both hex and preset (oklch) inputs. */
function accentAlpha(value: string, a: number): string {
  return value.startsWith("#")
    ? hexToRgba(value, a)
    : accentColor(value).replace(/\)$/, ` / ${a})`);
}

function applyAccent(value: string) {
  const root = document.documentElement.style;
  const accent = accentColor(value);
  const ink = value.startsWith("#")
    ? luminance(value) > 0.55
      ? "oklch(18% 0.02 60)"
      : "oklch(97% 0.01 80)"
    : "oklch(18% 0.02 60)";
  root.setProperty("--accent", accent);
  root.setProperty("--accent-soft", accentAlpha(value, 0.16));
  root.setProperty("--accent-ink", ink);
}

/** Secondary accent — tints the ambient background glow. */
function applyAccent2(value: string) {
  const root = document.documentElement.style;
  root.setProperty("--accent-2", accentColor(value));
  root.setProperty("--accent-2-glow", accentAlpha(value, 0.2));
}

let themePref = "dark";
const sysDark = window.matchMedia("(prefers-color-scheme: dark)");

function applyTheme(theme: string) {
  themePref = theme;
  const effective =
    theme === "system" ? (sysDark.matches ? "dark" : "light") : theme;
  document.documentElement.dataset.theme = effective;
}
sysDark.addEventListener("change", () => {
  if (themePref === "system") applyTheme("system");
});

function applyAppearance(s: Settings) {
  applyAccent(s.accent || "amber");
  applyAccent2(s.accent2 || "amber");
  applyTheme(s.theme || "dark");
}

/** Short uppercase code shown in the hero (airport-board style). */
function heroCode(): string {
  if (busy) return status.running ? "BYE" : "...";
  if (!status.running) return "OFF";
  const active = servers.find((s) => s.id === status.active_server);
  if (!active) return "ON";
  // Prefer an alpha chunk of the server name; fall back to protocol.
  const alpha = (active.name.match(/[A-Za-zА-Яа-я0-9]+/g) || []).join("");
  return (alpha.slice(0, 3) || active.protocol.slice(0, 3)).toUpperCase();
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------
function render() {
  renderHero();
  renderControls();
  renderMeta();
  renderServers();
  renderCore();
  updateExclCount();
}

function renderHero() {
  const ticket = document.querySelector(".ticket") as HTMLElement;
  const connected = status.running && !busy;
  ticket.classList.toggle("is-on", connected);
  ticket.classList.toggle("is-busy", busy);

  $("stateCode").textContent = heroCode();

  const dot = $("statusDot");
  dot.classList.toggle("on", connected);

  const label = document.querySelector(".connect__label") as HTMLElement;
  label.textContent = busy
    ? status.running ? "DISCONNECTING" : "CONNECTING"
    : status.running ? "DISCONNECT" : "CONNECT";

  $("headStatus").textContent = busy ? "···" : status.running ? "ONLINE" : "OFFLINE";
  $("headMode").innerHTML = settings.mode === "tun" ? "TUN&nbsp;·&nbsp;ALL" : "SYS&nbsp;PROXY";

  const active = servers.find((s) => s.id === status.active_server);
  $("activeName").textContent = active
    ? active.name
    : selectedId
      ? servers.find((s) => s.id === selectedId)?.name ?? "сервер не выбран"
      : "сервер не выбран";

  $("statusText").textContent = busy
    ? "NEGOTIATING TUNNEL"
    : status.running
      ? settings.mode === "tun"
        ? "TUNNEL ACTIVE · ALL TRAFFIC"
        : "PROXY ACTIVE · SYSTEM-WIDE"
      : servers.length
        ? "DISCONNECTED · STANDBY"
        : "NO NODE · STANDBY";
}

function renderControls() {
  document.querySelectorAll<HTMLButtonElement>(".seg-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.mode === settings.mode);
  });
  ($("bypassRu") as HTMLInputElement).checked = settings.bypass_ru;
}

function renderMeta() {
  const active =
    servers.find((s) => s.id === status.active_server) ??
    servers.find((s) => s.id === selectedId);

  $("activeAddr").textContent = active ? `${active.address}:${active.port}` : "—";
  $("activeProto").textContent = active ? active.protocol.toUpperCase() : "—";
  $("routingState").textContent = settings.bypass_ru ? "RU-BYPASS" : "FULL TUNNEL";
}

function renderServers() {
  const list = $("serverList");
  const empty = $("emptyState");
  $("srvCount").textContent = String(servers.length).padStart(2, "0");
  list.innerHTML = "";
  empty.hidden = servers.length > 0;

  for (const s of servers) {
    const li = document.createElement("li");
    li.className = "srv";
    if (s.id === selectedId) li.classList.add("selected");
    if (s.id === status.active_server && status.running) li.classList.add("active-srv");

    li.innerHTML = `
      <span class="srv__badge">${esc(s.protocol.toUpperCase())}</span>
      <div class="srv__body">
        <div class="srv__name">${esc(s.name)}</div>
        <div class="srv__addr">${esc(s.address)}:${s.port}</div>
      </div>
      <div class="srv__actions">
        <button class="icon-btn rename" title="Переименовать">✎</button>
        <button class="icon-btn del" title="Удалить">✕</button>
      </div>`;

    li.addEventListener("click", (e) => {
      const t = e.target as HTMLElement;
      if (t.classList.contains("rename")) {
        startRename(li, s);
        return;
      }
      if (t.classList.contains("del")) {
        deleteServer(s);
        return;
      }
      selectedId = s.id;
      render();
    });

    list.appendChild(li);
  }
}

// ---------------------------------------------------------------------------
// live traffic speed (footer) — polls the sing-box Clash API counters
// ---------------------------------------------------------------------------
let trafTimer: number | null = null;
let trafPrev: { up: number; down: number; t: number } | null = null;

function fmtSpeed(bps: number): string {
  if (bps < 1024) return `${Math.round(bps)} B/s`;
  if (bps < 1024 * 1024) return `${Math.round(bps / 1024)} KB/s`;
  return `${(bps / 1024 / 1024).toFixed(1)} MB/s`;
}

function renderSpeed(down: number, up: number) {
  $("corePath").innerHTML =
    `<span class="spd"><b>↓</b> ${fmtSpeed(down)}</span>` +
    `<span class="spd"><b>↑</b> ${fmtSpeed(up)}</span>`;
}

async function pollTraffic() {
  if (!status.running) return;
  try {
    const t = await invoke<{ up: number; down: number }>("traffic");
    const now = Date.now();
    if (trafPrev) {
      const dt = Math.max(0.25, (now - trafPrev.t) / 1000);
      renderSpeed(
        Math.max(0, (t.down - trafPrev.down) / dt),
        Math.max(0, (t.up - trafPrev.up) / dt)
      );
    }
    trafPrev = { up: t.up, down: t.down, t: now };
  } catch {
    /* core not up yet / api unavailable — ignore */
  }
}

function startTraffic() {
  if (trafTimer) return;
  trafPrev = null;
  renderSpeed(0, 0);
  pollTraffic();
  trafTimer = window.setInterval(pollTraffic, 1000);
}

function stopTraffic() {
  if (trafTimer) {
    window.clearInterval(trafTimer);
    trafTimer = null;
  }
  trafPrev = null;
}

function renderCore() {
  const el = $("corePath");
  if (!status.core_path) {
    stopTraffic();
    el.textContent = "⚠ ЯДРО SING-BOX НЕ НАЙДЕНО";
    return;
  }
  if (status.running) {
    startTraffic(); // poller drives the footer with live ↓/↑ speed
  } else {
    stopTraffic();
    el.textContent = settings.bypass_ru ? "STANDBY · RU-BYPASS" : "STANDBY · FULL TUNNEL";
  }
}

// ---------------------------------------------------------------------------
// actions
// ---------------------------------------------------------------------------
async function refresh() {
  [servers, settings, status] = await Promise.all([
    invoke<Server[]>("list_servers"),
    invoke<Settings>("get_settings"),
    invoke<Status>("status"),
  ]);
  if (!selectedId) selectedId = status.active_server ?? settings.active_server ?? servers[0]?.id ?? null;
  applyAppearance(settings);
  render();
}

async function togglePower() {
  if (busy) return;
  if (status.running) return disconnect();
  return connect();
}

async function connect() {
  const id = selectedId ?? status.active_server;
  if (!id) {
    toast("Сначала выберите сервер", true);
    return;
  }
  if (!status.core_path) {
    toast("Ядро sing-box не установлено: brew install sing-box", true);
    return;
  }
  busy = true;
  render();
  try {
    status = await invoke<Status>("connect", { id });
    toast("Подключено");
  } catch (e) {
    toast(String(e), true);
  } finally {
    busy = false;
    render();
  }
}

async function disconnect() {
  busy = true;
  render();
  try {
    status = await invoke<Status>("disconnect");
    toast("Отключено");
  } catch (e) {
    toast(String(e), true);
  } finally {
    busy = false;
    render();
  }
}

async function setMode(mode: Mode) {
  if (mode === settings.mode) return;
  settings = await invoke<Settings>("set_settings", { mode, bypassRu: settings.bypass_ru });
  render();
  if (status.running) {
    toast("Режим изменён — переподключаюсь…");
    await reconnect();
  }
}

async function setBypass(bypass: boolean) {
  settings = await invoke<Settings>("set_settings", { mode: settings.mode, bypassRu: bypass });
  render();
  if (status.running) await reconnect();
}

async function reconnect() {
  const id = status.active_server ?? selectedId;
  if (!id) return;
  busy = true;
  render();
  try {
    status = await invoke<Status>("connect", { id });
  } catch (e) {
    toast(String(e), true);
  } finally {
    busy = false;
    render();
  }
}

async function importLinks() {
  const input = $("importInput") as HTMLTextAreaElement;
  const msg = $("importMsg");
  const text = input.value.trim();
  if (!text) return;
  msg.className = "import__msg mono";
  msg.textContent = "PARSING…";
  try {
    const before = servers.length;
    servers = await invoke<Server[]>("add_links", { text });
    const added = servers.length - before;
    msg.className = "import__msg mono ok";
    msg.textContent = `+${added} ADDED`;
    input.value = "";
    if (!selectedId && servers.length) selectedId = servers[servers.length - 1].id;
    render();
  } catch (e) {
    msg.className = "import__msg mono err";
    msg.textContent = String(e);
  }
}

/** Inline rename: swap the server name for an input, commit on Enter/blur. */
function startRename(li: HTMLElement, s: Server) {
  const nameEl = li.querySelector(".srv__name") as HTMLElement | null;
  if (!nameEl) return;
  const input = document.createElement("input");
  input.className = "srv__rename";
  input.value = s.name;
  input.spellcheck = false;
  nameEl.replaceWith(input);
  input.focus();
  input.select();

  let done = false;
  const finish = async (save: boolean) => {
    if (done) return;
    done = true;
    const v = input.value.trim();
    if (save && v && v !== s.name) {
      try {
        servers = await invoke<Server[]>("rename_server", { id: s.id, name: v });
      } catch (e) {
        toast(String(e), true);
      }
    }
    render();
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      finish(true);
    } else if (e.key === "Escape") {
      e.preventDefault();
      finish(false);
    }
  });
  input.addEventListener("blur", () => finish(true));
  // don't let clicks inside the field bubble to the row (select/destroy)
  input.addEventListener("click", (e) => e.stopPropagation());
  input.addEventListener("mousedown", (e) => e.stopPropagation());
}

async function deleteServer(s: Server) {
  if (!window.confirm(`Удалить «${s.name}»?`)) return;
  servers = await invoke<Server[]>("delete_server", { id: s.id });
  if (selectedId === s.id) selectedId = servers[0]?.id ?? null;
  status = await invoke<Status>("status");
  render();
}

async function toggleLog() {
  const view = $("logView");
  view.hidden = !view.hidden;
  $("configView").hidden = true;
  if (logTimer) {
    window.clearInterval(logTimer);
    logTimer = null;
  }
  if (!view.hidden) {
    const pull = async () => {
      view.textContent = (await invoke<string>("get_log")) || "(пусто)";
      view.scrollTop = view.scrollHeight;
    };
    await pull();
    logTimer = window.setInterval(pull, 1500);
  }
}

async function toggleConfig() {
  const view = $("configView");
  view.hidden = !view.hidden;
  $("logView").hidden = true;
  if (logTimer) {
    window.clearInterval(logTimer);
    logTimer = null;
  }
  if (!view.hidden) {
    const id = selectedId ?? status.active_server;
    if (!id) {
      view.textContent = "Выберите сервер, чтобы увидеть конфиг";
      return;
    }
    try {
      view.textContent = await invoke<string>("preview_config", { id });
    } catch (e) {
      view.textContent = String(e);
    }
  }
}

// ---------------------------------------------------------------------------
// wire up
// ---------------------------------------------------------------------------
function bind() {
  $("powerBtn").addEventListener("click", togglePower);
  $("bypassRu").addEventListener("change", (e) =>
    setBypass((e.target as HTMLInputElement).checked)
  );
  document.querySelectorAll<HTMLButtonElement>(".seg-btn").forEach((b) =>
    b.addEventListener("click", () => setMode(b.dataset.mode as Mode))
  );
  $("importToggle").addEventListener("click", () => {
    const box = $("importBox");
    box.hidden = !box.hidden;
    if (!box.hidden) ($("importInput") as HTMLTextAreaElement).focus();
  });
  $("importBtn").addEventListener("click", importLinks);
  $("logToggle").addEventListener("click", toggleLog);
  $("configToggle").addEventListener("click", toggleConfig);
}

// ---------------------------------------------------------------------------
// auto-update — pretty dialog: «Обновить сейчас / В следующий раз / Пропустить»
// ---------------------------------------------------------------------------
interface UpdateInfo {
  available: boolean;
  version: string | null;
  notes: string | null;
}

const SKIP_KEY = "cn.skipVersion";

/** Render release notes: keep line breaks, bold the first line as a heading. */
function renderNotes(notes: string | null): string {
  if (!notes) return "Подробности об изменениях — на странице релиза на GitHub.";
  const lines = notes.trim().split("\n");
  const [head, ...rest] = lines;
  return `<strong>${esc(head)}</strong>${rest.length ? "\n" + esc(rest.join("\n")) : ""}`;
}

function showUpdateModal(info: UpdateInfo) {
  $("updVersion").textContent = info.version ?? "";
  $("updFrom").textContent = `ТЕКУЩАЯ ВЕРСИЯ ${currentVersion}`;
  $("updNotes").innerHTML = renderNotes(info.notes);
  $("updProgress").hidden = true;
  ($("updNow") as HTMLButtonElement).disabled = false;
  $("updateModal").hidden = false;
}

function hideUpdateModal() {
  $("updateModal").hidden = true;
}

interface DownloadProgress {
  downloaded: number;
  total: number | null;
  percent: number | null;
}

/** Drive the determinate progress bar; `percent === null` → indeterminate. */
function setUpdProgress(p: DownloadProgress) {
  const bar = $("updProgress");
  const fill = $("updProgressFill");
  const pct = $("updProgressPct");
  const label = $("updProgressLabel");
  if (p.percent == null) {
    bar.classList.add("indeterminate");
    fill.style.width = "100%";
    pct.textContent = "";
    label.textContent = "ЗАГРУЗКА…";
    return;
  }
  bar.classList.remove("indeterminate");
  const v = Math.round(p.percent);
  fill.style.width = `${v}%`;
  pct.textContent = `${v}%`;
  label.textContent = v >= 100 ? "УСТАНОВКА…" : "ЗАГРУЗКА…";
}

async function runInstall() {
  setUpdProgress({ downloaded: 0, total: null, percent: 0 });
  $("updProgress").hidden = false;
  ($("updNow") as HTMLButtonElement).disabled = true;
  ($("updLater") as HTMLButtonElement).disabled = true;
  ($("updSkip") as HTMLButtonElement).disabled = true;
  try {
    await invoke("install_update"); // downloads, installs and relaunches
  } catch (e) {
    toast(String(e), true);
    hideUpdateModal();
    ($("updNow") as HTMLButtonElement).disabled = false;
    ($("updLater") as HTMLButtonElement).disabled = false;
    ($("updSkip") as HTMLButtonElement).disabled = false;
  }
}

/**
 * @param manual true when triggered from Settings / tray — always reports a
 * result and ignores the per-version skip list.
 */
async function checkForUpdate(manual = false) {
  if (manual) setUpdMsg("Проверяю…");
  try {
    const info = await invoke<UpdateInfo>("check_update");
    if (!info.available || !info.version) {
      if (manual) setUpdMsg(`У вас последняя версия (${currentVersion})`, "ok");
      return;
    }
    if (!manual && localStorage.getItem(SKIP_KEY) === info.version) return;
    if (manual) setUpdMsg(`Доступна версия ${info.version}`, "ok");
    showUpdateModal(info);
  } catch (e) {
    if (manual) setUpdMsg("Не удалось проверить обновления", "err");
    else console.warn("update check failed:", e);
  }
}

function bindUpdateModal() {
  $("updNow").addEventListener("click", runInstall);
  $("updLater").addEventListener("click", hideUpdateModal);
  $("updSkip").addEventListener("click", () => {
    const v = $("updVersion").textContent;
    if (v) localStorage.setItem(SKIP_KEY, v);
    hideUpdateModal();
  });
}

// ---------------------------------------------------------------------------
// settings screen
// ---------------------------------------------------------------------------
let currentVersion = "0.0.0";

function setUpdMsg(text: string, kind: "" | "ok" | "err" = "") {
  const el = $("setUpdMsg");
  el.textContent = text;
  el.className = `set__msg mono${kind ? " " + kind : ""}`;
}

const DEFAULT_CUSTOM = "#e8a33d";

/** Render one preset row into `boxId`, marking `current` selected. */
function renderSwatchRow(boxId: string, current: string, onPick: (name: string) => void) {
  const box = $(boxId);
  box.innerHTML = "";
  for (const [name, color] of Object.entries(ACCENTS)) {
    const b = document.createElement("button");
    b.className = "swatch";
    b.style.setProperty("--sw", color);
    b.title = name;
    if (current === name) b.classList.add("selected");
    b.addEventListener("click", () => onPick(name));
    box.appendChild(b);
  }
}

function renderSwatches() {
  renderSwatchRow("swatches", settings.accent, (name) => saveAppearance({ accent: name }));
  renderSwatchRow("swatches2", settings.accent2, (name) => saveAppearance({ accent2: name }));
}

function renderThemeSeg() {
  document.querySelectorAll<HTMLButtonElement>("#themeSeg .seg-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.theme === settings.theme);
  });
}

/** Apply a partial appearance change (any of accent / accent2 / theme). */
async function saveAppearance(patch: Partial<Pick<Settings, "accent" | "accent2" | "theme">>) {
  // optimistic: apply instantly, then persist
  settings = { ...settings, ...patch };
  applyAppearance(settings);
  renderSwatches();
  renderThemeSeg();
  // when a preset is picked, reset the matching custom picker to its default
  if (patch.accent && !patch.accent.startsWith("#")) {
    ($("accentCustom") as HTMLInputElement).value = DEFAULT_CUSTOM;
  }
  if (patch.accent2 && !patch.accent2.startsWith("#")) {
    ($("accentCustom2") as HTMLInputElement).value = DEFAULT_CUSTOM;
  }
  try {
    settings = await invoke<Settings>("set_appearance", {
      accent: settings.accent,
      accent2: settings.accent2,
      theme: settings.theme,
    });
  } catch (e) {
    toast(String(e), true);
  }
}

async function openSettings() {
  renderSwatches();
  renderThemeSeg();
  const c1 = $("accentCustom") as HTMLInputElement;
  const c2 = $("accentCustom2") as HTMLInputElement;
  c1.value = settings.accent.startsWith("#") ? settings.accent : DEFAULT_CUSTOM;
  c2.value = settings.accent2.startsWith("#") ? settings.accent2 : DEFAULT_CUSTOM;
  setUpdMsg("");
  $("setVersion").textContent = currentVersion;
  $("settingsModal").hidden = false;
}

function closeSettings() {
  $("settingsModal").hidden = true;
}

function bindSettings() {
  $("settingsBtn").addEventListener("click", openSettings);
  document.querySelectorAll<HTMLElement>("[data-close]").forEach((el) =>
    el.addEventListener("click", closeSettings)
  );
  document.querySelectorAll<HTMLButtonElement>("#themeSeg .seg-btn").forEach((b) =>
    b.addEventListener("click", () => saveAppearance({ theme: b.dataset.theme as string }))
  );
  ($("accentCustom") as HTMLInputElement).addEventListener("input", (e) =>
    saveAppearance({ accent: (e.target as HTMLInputElement).value })
  );
  ($("accentCustom2") as HTMLInputElement).addEventListener("input", (e) =>
    saveAppearance({ accent2: (e.target as HTMLInputElement).value })
  );
  $("setCheckUpd").addEventListener("click", () => checkForUpdate(true));
  // Esc closes settings (and dismisses the update dialog as «later»)
  window.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    if (!$("exclusionsModal").hidden) $("exclusionsModal").hidden = true;
    else if (!$("settingsModal").hidden) closeSettings();
    else if (!$("updateModal").hidden) hideUpdateModal();
  });
}

// ---------------------------------------------------------------------------
// exclusions — pick installed apps whose traffic bypasses the VPN
// ---------------------------------------------------------------------------
interface AppInfo {
  name: string;
  exec: string;
  icon: string | null;
}
let allApps: AppInfo[] = [];
let appsLoaded = false; // distinguishes "still loading" from "loaded, but empty"
let exclSet = new Set<string>(); // working selection while the modal is open

function updateExclCount() {
  const n = settings.excluded_apps?.length ?? 0;
  $("exclCount").textContent = n ? ` ${n}` : "";
}

function renderExclList(query: string) {
  const list = $("exclList");
  const q = query.trim().toLowerCase();
  const apps = q ? allApps.filter((a) => a.name.toLowerCase().includes(q)) : allApps;
  list.innerHTML = "";
  if (!apps.length) {
    const msg = !appsLoaded
      ? "Загрузка…"
      : allApps.length
        ? "Ничего не найдено"
        : "Приложения не найдены";
    list.innerHTML = `<div class="excl-empty mono">${msg}</div>`;
    return;
  }
  for (const a of apps) {
    const row = document.createElement("div");
    row.className = "excl-row" + (exclSet.has(a.exec) ? " on" : "");
    const ico = a.icon
      ? `<img class="excl-ico" src="${a.icon}" alt="" />`
      : `<span class="excl-ico excl-ico--ph"></span>`;
    row.innerHTML = `${ico}<span class="excl-name">${esc(a.name)}</span><span class="excl-box"></span>`;
    row.addEventListener("click", () => {
      if (exclSet.has(a.exec)) exclSet.delete(a.exec);
      else exclSet.add(a.exec);
      row.classList.toggle("on");
    });
    list.appendChild(row);
  }
}

async function openExclusions() {
  exclSet = new Set(settings.excluded_apps);
  $("exclusionsModal").hidden = false;
  ($("exclSearch") as HTMLInputElement).value = "";
  renderExclList("");
  if (!appsLoaded) {
    try {
      allApps = await invoke<AppInfo[]>("list_apps");
      appsLoaded = true;
    } catch (e) {
      toast(String(e), true); // leave appsLoaded false so the next open retries
    }
    renderExclList("");
  }
}

async function saveExclusions() {
  const apps = [...exclSet];
  try {
    settings = await invoke<Settings>("set_exclusions", { apps });
  } catch (e) {
    toast(String(e), true);
    return;
  }
  $("exclusionsModal").hidden = true;
  updateExclCount();
  toast(apps.length ? `В обход VPN: ${apps.length} прил.` : "Список игнора очищен");
  if (status.running) await reconnect();
}

/** Add an exclusion by a file path or raw process name (the basename is used as
 *  sing-box `process_name`). Lets the user cover apps the picker didn't list. */
function addCustomExclusion() {
  const input = $("exclPath") as HTMLInputElement;
  const raw = input.value.trim();
  if (!raw) return;
  // Reduce a path ("C:\…\app.exe" or "/Applications/App.app/…/App") to its name.
  const exec = raw.split(/[\\/]/).pop()!.trim();
  if (!exec) return;
  // Surface it as a row so it's visible and selected, like any listed app.
  if (!allApps.some((a) => a.exec === exec)) {
    allApps = [{ name: exec, exec, icon: null }, ...allApps];
  }
  exclSet.add(exec);
  input.value = "";
  ($("exclSearch") as HTMLInputElement).value = "";
  renderExclList("");
  toast(`Добавлено в игнор: ${exec}`);
}

function bindExclusions() {
  $("exclToggle").addEventListener("click", openExclusions);
  $("exclSave").addEventListener("click", saveExclusions);
  ($("exclSearch") as HTMLInputElement).addEventListener("input", (e) =>
    renderExclList((e.target as HTMLInputElement).value)
  );
  $("exclAddBtn").addEventListener("click", addCustomExclusion);
  ($("exclPath") as HTMLInputElement).addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      addCustomExclusion();
    }
  });
  document.querySelectorAll<HTMLElement>("[data-close-excl]").forEach((el) =>
    el.addEventListener("click", () => ($("exclusionsModal").hidden = true))
  );
}

// ---------------------------------------------------------------------------
// boot
// ---------------------------------------------------------------------------
bind();
bindUpdateModal();
bindSettings();
bindExclusions();
refresh();

invoke<string>("app_version")
  .then((v) => {
    currentVersion = v;
    $("setVersion").textContent = v;
  })
  .catch(() => {});

// react to actions taken from the menu-bar tray (connect/disconnect/check-update)
listen("status-changed", async () => {
  if (busy) return;
  status = await invoke<Status>("status");
  render();
});
listen("tray://check-update", () => checkForUpdate(true));
listen<string>("tray-error", (e) => toast(e.payload, true));
listen<DownloadProgress>("update-progress", (e) => setUpdProgress(e.payload));

// keep status fresh in case the core dies unexpectedly
window.setInterval(async () => {
  if (busy) return;
  const next = await invoke<Status>("status");
  if (next.running !== status.running || next.active_server !== status.active_server) {
    status = next;
    render();
  }
}, 2500);

// check for updates shortly after launch
window.setTimeout(() => checkForUpdate(false), 2500);
