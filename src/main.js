import { invoke } from "@tauri-apps/api/core";
// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------
let servers = [];
let settings = { mode: "system_proxy", bypass_ru: true, active_server: null };
let status = { running: false, active_server: null, mode: null, bypass_ru: true, core_path: null };
let selectedId = null;
let busy = false;
let logTimer = null;
// ---------------------------------------------------------------------------
// dom helpers
// ---------------------------------------------------------------------------
const $ = (id) => document.getElementById(id);
function toast(message, isError = false) {
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
function esc(s) {
    const d = document.createElement("div");
    d.textContent = s;
    return d.innerHTML;
}
/** Short uppercase code shown in the hero (airport-board style). */
function heroCode() {
    if (busy)
        return status.running ? "BYE" : "...";
    if (!status.running)
        return "OFF";
    const active = servers.find((s) => s.id === status.active_server);
    if (!active)
        return "ON";
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
}
function renderHero() {
    const ticket = document.querySelector(".ticket");
    const connected = status.running && !busy;
    ticket.classList.toggle("is-on", connected);
    ticket.classList.toggle("is-busy", busy);
    $("stateCode").textContent = heroCode();
    const dot = $("statusDot");
    dot.classList.toggle("on", connected);
    const label = document.querySelector(".connect__label");
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
    document.querySelectorAll(".seg-btn").forEach((b) => {
        b.classList.toggle("active", b.dataset.mode === settings.mode);
    });
    $("bypassRu").checked = settings.bypass_ru;
}
function renderMeta() {
    const active = servers.find((s) => s.id === status.active_server) ??
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
        if (s.id === selectedId)
            li.classList.add("selected");
        if (s.id === status.active_server && status.running)
            li.classList.add("active-srv");
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
            const t = e.target;
            if (t.classList.contains("rename")) {
                renameServer(s);
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
function renderCore() {
    $("corePath").textContent = status.core_path
        ? `CORE · ${status.core_path}`
        : "⚠ ЯДРО SING-BOX НЕ НАЙДЕНО · brew install sing-box";
}
// ---------------------------------------------------------------------------
// actions
// ---------------------------------------------------------------------------
async function refresh() {
    [servers, settings, status] = await Promise.all([
        invoke("list_servers"),
        invoke("get_settings"),
        invoke("status"),
    ]);
    if (!selectedId)
        selectedId = status.active_server ?? settings.active_server ?? servers[0]?.id ?? null;
    render();
}
async function togglePower() {
    if (busy)
        return;
    if (status.running)
        return disconnect();
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
        status = await invoke("connect", { id });
        toast("Подключено");
    }
    catch (e) {
        toast(String(e), true);
    }
    finally {
        busy = false;
        render();
    }
}
async function disconnect() {
    busy = true;
    render();
    try {
        status = await invoke("disconnect");
        toast("Отключено");
    }
    catch (e) {
        toast(String(e), true);
    }
    finally {
        busy = false;
        render();
    }
}
async function setMode(mode) {
    if (mode === settings.mode)
        return;
    settings = await invoke("set_settings", { mode, bypassRu: settings.bypass_ru });
    render();
    if (status.running) {
        toast("Режим изменён — переподключаюсь…");
        await reconnect();
    }
}
async function setBypass(bypass) {
    settings = await invoke("set_settings", { mode: settings.mode, bypassRu: bypass });
    render();
    if (status.running)
        await reconnect();
}
async function reconnect() {
    const id = status.active_server ?? selectedId;
    if (!id)
        return;
    busy = true;
    render();
    try {
        status = await invoke("connect", { id });
    }
    catch (e) {
        toast(String(e), true);
    }
    finally {
        busy = false;
        render();
    }
}
async function importLinks() {
    const input = $("importInput");
    const msg = $("importMsg");
    const text = input.value.trim();
    if (!text)
        return;
    msg.className = "import__msg mono";
    msg.textContent = "PARSING…";
    try {
        const before = servers.length;
        servers = await invoke("add_links", { text });
        const added = servers.length - before;
        msg.className = "import__msg mono ok";
        msg.textContent = `+${added} ADDED`;
        input.value = "";
        if (!selectedId && servers.length)
            selectedId = servers[servers.length - 1].id;
        render();
    }
    catch (e) {
        msg.className = "import__msg mono err";
        msg.textContent = String(e);
    }
}
async function renameServer(s) {
    const name = window.prompt("Новое имя сервера:", s.name);
    if (!name || name === s.name)
        return;
    servers = await invoke("rename_server", { id: s.id, name });
    render();
}
async function deleteServer(s) {
    if (!window.confirm(`Удалить «${s.name}»?`))
        return;
    servers = await invoke("delete_server", { id: s.id });
    if (selectedId === s.id)
        selectedId = servers[0]?.id ?? null;
    status = await invoke("status");
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
            view.textContent = (await invoke("get_log")) || "(пусто)";
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
            view.textContent = await invoke("preview_config", { id });
        }
        catch (e) {
            view.textContent = String(e);
        }
    }
}
// ---------------------------------------------------------------------------
// wire up
// ---------------------------------------------------------------------------
function bind() {
    $("powerBtn").addEventListener("click", togglePower);
    $("bypassRu").addEventListener("change", (e) => setBypass(e.target.checked));
    document.querySelectorAll(".seg-btn").forEach((b) => b.addEventListener("click", () => setMode(b.dataset.mode)));
    $("importToggle").addEventListener("click", () => {
        const box = $("importBox");
        box.hidden = !box.hidden;
        if (!box.hidden)
            $("importInput").focus();
    });
    $("importBtn").addEventListener("click", importLinks);
    $("logToggle").addEventListener("click", toggleLog);
    $("configToggle").addEventListener("click", toggleConfig);
}
async function checkForUpdate() {
    try {
        const info = await invoke("check_update");
        if (!info.available || !info.version)
            return;
        const ok = window.confirm(`Доступно обновление coffeeNetwork ${info.version}.\n\n` +
            (info.notes ? info.notes + "\n\n" : "") +
            "Установить и перезапустить сейчас?");
        if (!ok)
            return;
        toast(`Загрузка ${info.version}…`);
        await invoke("install_update");
    }
    catch (e) {
        // offline or no release yet — not worth nagging the user about
        console.warn("update check failed:", e);
    }
}
bind();
refresh();
// keep status fresh in case the core dies unexpectedly
window.setInterval(async () => {
    if (busy)
        return;
    const next = await invoke("status");
    if (next.running !== status.running || next.active_server !== status.active_server) {
        status = next;
        render();
    }
}, 2500);
// check for updates shortly after launch
window.setTimeout(checkForUpdate, 2500);
