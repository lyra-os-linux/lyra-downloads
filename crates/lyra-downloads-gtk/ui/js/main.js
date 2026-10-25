// Lyra Downloads — interface Tauri no padrão do Vega. Fala com o serviço
// pelo protocolo de crates/lyra-downloads-ipc (Op, Snapshot, TaskView);
// as regras de cada ação seguem a antiga interface GTK.

import { backend, call, on, pickFolder, win } from "./api.js";
import { svg, registerIcons } from "./icons.js";
import { loadCatalog, tr, trf } from "./i18n.js";
import {
  h, icon, clear, bytes, rate, relative, dayLabel, dateTime, normalize, button, iconButton, badge, toggle,
  segmented, list, section, meter, banner, empty, toast, overlay, confirm, sheet, popover, menuItem, fail, busy,
} from "./ui.js";

registerIcons({
  link: '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>',
  grip: '<circle cx="9" cy="6" r="1"/><circle cx="15" cy="6" r="1"/><circle cx="9" cy="12" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="9" cy="18" r="1"/><circle cx="15" cy="18" r="1"/>',
  hash: '<path d="M4 9h16M4 15h16M10 3 8 21M16 3l-2 18"/>',
  "list-all": '<path d="M3 6h.01M3 12h.01M3 18h.01M8 6h13M8 12h13M8 18h13"/>',
  hourglass: '<path d="M5 22h14M5 2h14"/><path d="M17 22v-4.17a2 2 0 0 0-.59-1.42L12 12l-4.41 4.41A2 2 0 0 0 7 17.83V22"/><path d="M7 2v4.17a2 2 0 0 0 .59 1.42L12 12l4.41-4.41A2 2 0 0 0 17 6.17V2"/>',
  "to-top": '<path d="M5 3h14"/><path d="m18 13-6-6-6 6"/><path d="M12 7v14"/>',
  ban: '<circle cx="12" cy="12" r="10"/><path d="m4.9 4.9 14.2 14.2"/>',
});

const $ = (id) => document.getElementById(id);

// ---------- Modelo ----------

const CONNECTIONS = [1, 4, 8, 16];
const PROFILE = { One: 1, Four: 4, Eight: 8, Sixteen: 16 };
const PROFILE_NAME = { 1: "One", 4: "Four", 8: "Eight", 16: "Sixteen" };
const conns = (task) => PROFILE[task.connections] ?? task.connections;

const ACTIVE = ["baixando", "aguardando", "pausado", "verificando"];
const TERMINAL = ["concluido", "cancelado", "erro"];
const VIEWS = [
  { id: "active", title: () => tr("In progress"), icon: "download", accepts: (s) => ACTIVE.includes(s) },
  { id: "done", title: () => tr("Completed"), icon: "check-circle", accepts: (s) => s === "concluido" },
  { id: "failed", title: () => tr("Failed"), icon: "alert", accepts: (s) => s === "erro" },
  { id: "all", title: () => tr("All"), icon: "list-all", accepts: () => true },
];
const byView = new Map(VIEWS.map((v) => [v.id, v]));

const STATE = {
  aguardando: { label: () => tr("Queued"), icon: "hourglass" },
  baixando: { label: () => tr("Downloading"), icon: "arrow-down" },
  pausado: { label: () => tr("Paused"), icon: "pause" },
  verificando: { label: () => tr("Verifying SHA-256"), icon: "hash" },
  concluido: { label: () => tr("Completed"), icon: "check" },
  cancelado: { label: () => tr("Canceled"), icon: "ban" },
  erro: { label: () => tr("Error"), icon: "alert" },
};

let snap = { tasks: [], total_speed: 0, engine_running: true, engine_problem: null };
let serviceError = null;
let settings = null;
let appInfo = { version: "" };
let view = "active";
let selectedId = null;
let filter = "";
const history = Array(40).fill(0);

// ---------- Tema (segue o GNOME) ----------

const ACCENTS = {
  blue: "#3584e4", teal: "#2190a4", green: "#3a944a", yellow: "#c88800", orange: "#ed5b00",
  red: "#e62d42", pink: "#d56199", purple: "#9141ac", slate: "#6f8396",
};
function applyInterface({ color_scheme, accent } = {}) {
  const dark = color_scheme === "prefer-dark";
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  let color = ACCENTS[accent] ?? ACCENTS.blue;
  if (dark) color = `color-mix(in srgb, ${color} 78%, #fff)`;
  document.documentElement.style.setProperty("--accent", color);
}

// ---------- Formatação ----------

const EXT_COLORS = {
  iso: "#3584e4", img: "#3584e4", zip: "#c64600", "tar.gz": "#c64600", "tar.xz": "#c64600", "tar.zst": "#c64600", "tar.bz2": "#c64600", "7z": "#c64600", rar: "#c64600",
  pdf: "#e01b24", mp4: "#9141ac", mkv: "#9141ac", webm: "#9141ac", mp3: "#2190a4", flac: "#2190a4", ogg: "#2190a4",
  png: "#26a269", jpg: "#26a269", jpeg: "#26a269", svg: "#26a269", rpm: "#3a944a", deb: "#a51d2d", appimage: "#613583", exe: "#5e5c64",
};
function ext(name) {
  const m = name.toLowerCase().match(/\.(tar\.(?:gz|xz|zst|bz2))$|\.([a-z0-9]{1,8})$/);
  return m ? (m[1] ?? m[2]) : "";
}
function fileTile(task, cls = "") {
  const e = ext(task.filename);
  const label = e.startsWith("tar.") ? "TAR" : e.slice(0, 4) || "—";
  const el = h("span", { class: `ftile st-${task.state} ${cls}`.trim(), style: { "--tile": EXT_COLORS[e] ?? "#77767b" } }, label);
  el.append(icon(STATE[task.state]?.icon ?? "info", 14));
  return el;
}
const host = (url) => { try { return new URL(url).hostname; } catch { return url; } };
const finalPath = (task) => `${task.destination_dir.replace(/\/$/, "")}/${task.filename}`;
const fraction = (task) => (task.total_bytes ? Math.min(1, task.downloaded_bytes / task.total_bytes) : null);
function secondsLabel(secs) {
  if (secs < 60) return trf("{n} s", { n: Math.max(1, Math.round(secs)) });
  if (secs < 3600) return trf("{n} min", { n: Math.round(secs / 60) });
  return trf("{hours} h {minutes} min", { hours: Math.floor(secs / 3600), minutes: Math.round((secs % 3600) / 60) });
}
const remainingSecs = (task) => (task.download_speed && task.total_bytes ? (task.total_bytes - task.downloaded_bytes) / task.download_speed : null);

// ---------- Ações (mesmas regras da interface GTK) ----------

const can = {
  pause: (s) => s === "baixando" || s === "aguardando",
  resume: (s) => s === "pausado",
  cancel: (s) => !TERMINAL.includes(s),
  retry: (s) => s === "erro" || s === "cancelado",
  newUrl: (s) => s === "erro" || s === "cancelado",
  move: (s) => s === "aguardando",
  connections: (s) => s === "aguardando" || s === "baixando" || s === "pausado",
  remove: (s) => TERMINAL.includes(s),
};

async function op(payload, success) {
  try {
    const result = await backend(payload);
    if (success) toast(typeof success === "function" ? success(result) : success);
    refresh();
    return result;
  } catch (error) {
    fail(error);
    refresh();
    return undefined;
  }
}

const pause = (task) => op({ op: "pause", id: task.id });
const resume = (task) => op({ op: "resume", id: task.id });
const cancel = (task) => op({ op: "cancel", id: task.id });
const retry = (task) => op({ op: "retry", id: task.id });
const move = (task, direction) => op({ op: "move", id: task.id, direction });
const togglePause = (task) => (can.resume(task.state) ? resume(task) : can.pause(task.state) ? pause(task) : null);
const removeFromHistory = (task) => op({ op: "remove_from_history", id: task.id }, tr("Removed from history. The file was kept on disk."));
const changeConnections = (task, n) => op({ op: "change_connections", id: task.id, connections: n },
  (r) => (r?.restarted ? tr("Connections changed. The transfer resumed from its previous position.") : tr("Connections changed.")));
const openFile = (task) => call("open_file", { path: finalPath(task) }).catch(fail);
const openFolder = (task) => call("open_folder", { path: finalPath(task), dir: task.destination_dir }).catch(fail);

async function deleteFile(task) {
  const ok = await confirm({
    title: tr("Delete file from disk?"),
    text: trf("“{name}” and its partial data will be deleted from {dir}, and the item will be removed from the list. This cannot be undone.", { name: task.filename, dir: task.destination_dir }),
    ic: "trash", tone: "red", confirmLabel: tr("Delete file"), confirmKind: "danger",
  });
  if (ok) op({ op: "delete_file", id: task.id }, tr("File deleted."));
}

function newLink(task) {
  const input = h("input", { class: "input", placeholder: "https://", "aria-label": tr("Address"), style: { width: "100%" } });
  const create = button(tr("Create task"), { kind: "primary", disabled: true });
  input.addEventListener("input", () => (create.disabled = !validUrl(input.value.trim())));
  input.addEventListener("keydown", (e) => { if (e.key === "Enter" && !create.disabled) create.click(); });
  create.addEventListener("click", () => busy(create, async () => {
    if (await op({ op: "retry_with_new_url", id: task.id, url: input.value.trim() }, tr("New task created.")) !== undefined) close();
  }));
  const close = sheet(tr("Try with a new link"), [
    h("p", { class: "muted", style: { "margin-bottom": "14px" } }, tr("A new task will be created. The previous link's partial file will not be reused because its content cannot be confirmed to be the same.")),
    h("div", { class: "field" }, h("label", {}, tr("Address")), input),
    h("div", { class: "sheet-actions" }, button(tr("Cancel"), { onClick: () => close() }), create),
  ], { wide: false });
  requestAnimationFrame(() => input.focus());
}

// ---------- Linha de download ----------

function primaryAction(task) {
  const s = task.state;
  if (can.pause(s)) return iconButton("pause", tr("Pause"), (e) => { e.stopPropagation(); pause(task); });
  if (can.resume(s)) return iconButton("play", tr("Resume"), (e) => { e.stopPropagation(); resume(task); }, "go");
  if (s === "erro" || s === "cancelado") {
    return [
      button(tr("New link…"), { size: "sm", kind: "flat", icon: "link", onClick: (e) => { e.stopPropagation(); newLink(task); } }),
      button(tr("Try again"), { size: "sm", icon: "refresh", onClick: (e) => { e.stopPropagation(); retry(task); } }),
    ];
  }
  if (s === "concluido") return iconButton("external", tr("Open file"), (e) => { e.stopPropagation(); openFile(task); });
  return null;
}

function statusLine(task) {
  const parts = [];
  const add = (text, cls = "") => parts.push(h("span", { class: `${parts.length ? "dot" : ""} ${cls}`.trim() }, text));
  const total = task.total_bytes;
  const received = total ? trf("{done} of {total}", { done: bytes(task.downloaded_bytes), total: bytes(total) }) : trf("{done} received", { done: bytes(task.downloaded_bytes) });
  switch (task.state) {
    case "baixando": {
      const secs = remainingSecs(task);
      add(received);
      add(rate(task.download_speed));
      add(secs != null ? trf("{time} left", { time: secondsLabel(secs) }) : tr("remaining time unknown"));
      add(trf("{active} of {requested} connections", { active: task.active_connections, requested: conns(task) }));
      break;
    }
    case "aguardando":
      add(trf("Queued · position {n}", { n: queueIndex(task) }));
      if (total) add(bytes(total));
      break;
    case "pausado":
      add(task.pause_reason === "sistema" ? tr("Paused by the system · resumes automatically") : tr("Paused"));
      add(received);
      break;
    case "verificando":
      add(tr("Checking the SHA-256…"));
      if (total) add(bytes(total));
      break;
    case "concluido":
      if (total) add(bytes(total));
      add(relative(task.updated_at));
      add(host(task.url), "host");
      break;
    case "cancelado":
      add(tr("Canceled"));
      add(received);
      break;
    case "erro":
      add(task.error_message ?? tr("Error"), "err");
      break;
  }
  return h("div", { class: "dl-status" }, parts);
}

function rowBadges(task) {
  const out = [];
  if (task.hash_verification === "confere") out.push(badge(tr("SHA-256 matches"), "green", "check"));
  if (task.hash_verification === "nao_confere") out.push(badge(tr("SHA-256 does not match"), "red", "alert"));
  return out;
}

function progressBar(task) {
  if (task.state === "concluido") return null;
  if (task.state === "aguardando" && !task.downloaded_bytes) return null;
  const f = fraction(task);
  const cls = { pausado: "paused", aguardando: "paused", verificando: "verify indeterminate", erro: "error", cancelado: "error" }[task.state] ?? "";
  const m = meter(f ?? 0.3, { warnAt: 2, critAt: 2 });
  m.className = `meter ${cls} ${f == null && task.state === "baixando" ? "indeterminate" : ""}`.trim();
  return h("div", { class: "dl-progress" }, m, h("span", { class: "pct" }, task.state === "verificando" ? "SHA-256" : f == null ? "—" : `${Math.floor(f * 100)}%`));
}

function downloadRow(task, { draggable } = {}) {
  const el = h("div", {
    class: `dl-row ${task.id === selectedId ? "selected" : ""}`.trim(),
    tabindex: "0",
    role: "listitem",
    dataset: { id: task.id },
    onclick: () => select(task.id),
    ondblclick: () => (current(task).state === "concluido" ? openFile(current(task)) : openDetails(current(task))),
    onkeydown: (e) => rowKeys(e, current(task)),
    oncontextmenu: (e) => { e.preventDefault(); openRowMenu(e.currentTarget.querySelector(".more"), current(task)); },
  });
  const paint = (tk) => {
    el.setAttribute("aria-label", `${tk.filename}, ${STATE[tk.state]?.label() ?? tk.state}`);
    clear(
      el,
      draggable && h("span", { class: "drag-handle", title: tr("Drag to reorder the queue") }, icon("grip", 14)),
      fileTile(tk),
      h("div", { class: "dl-main" }, h("div", { class: "dl-name" }, h("strong", { title: finalPath(tk) }, tk.filename), rowBadges(tk)), statusLine(tk), progressBar(tk)),
      h("div", { class: "dl-actions" },
        primaryAction(tk),
        tk.state === "concluido" && iconButton("folder", tr("Open folder"), (e) => { e.stopPropagation(); openFolder(tk); }, "hide-narrow"),
        can.cancel(tk.state) ? iconButton("x", tr("Cancel transfer"), (e) => { e.stopPropagation(); cancel(tk); }, "hide-narrow") : null,
        h("button", { class: "icon-btn more", "aria-label": tr("More actions"), title: tr("More actions"), onclick: (e) => { e.stopPropagation(); openRowMenu(e.currentTarget, current(tk)); } }, icon("more", 17))),
    );
  };
  paint(task);
  el.update = paint;
  if (draggable) dragReorder(el, task);
  return el;
}

/** Versão mais recente da tarefa (a linha pode ter sido desenhada antes). */
const current = (task) => snap.tasks.find((tk) => tk.id === task.id) ?? task;

function rowKeys(e, task) {
  if (e.target !== e.currentTarget) return;
  if (e.key === "Enter") { e.preventDefault(); task.state === "concluido" ? openFile(task) : openDetails(task); }
  else if (e.key === " ") { e.preventDefault(); togglePause(task); }
  else if (e.key === "Delete" && can.remove(task.state)) { e.preventDefault(); removeFromHistory(task); }
  else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const rows = [...document.querySelectorAll(".dl-row")];
    const next = rows[rows.indexOf(e.currentTarget) + (e.key === "ArrowDown" ? 1 : -1)];
    if (next) { next.focus(); select(next.dataset.id); }
  } else if (e.altKey && (e.key === "PageUp" || e.key === "PageDown") && can.move(task.state)) {
    e.preventDefault(); move(task, e.key === "PageUp" ? "up" : "down");
  } else if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
    e.preventDefault(); openRowMenu(e.currentTarget.querySelector(".more"), task);
  }
}

function select(taskId) {
  selectedId = taskId;
  document.querySelectorAll(".dl-row").forEach((r) => r.classList.toggle("selected", r.dataset.id === taskId));
}

// Arrastar para reordenar a fila: o protocolo só move um passo por vez
// (up/down/top/bottom), então a nova posição vira uma sequência de passos.
let dragging = null;
function dragReorder(el, task) {
  el.draggable = true;
  el.addEventListener("dragstart", (e) => {
    dragging = task.id; el.classList.add("dragging");
    e.dataTransfer.setData("application/x-lyra-task", task.id);
    e.dataTransfer.effectAllowed = "move";
  });
  el.addEventListener("dragend", () => { dragging = null; el.classList.remove("dragging"); document.querySelectorAll(".drop-before").forEach((r) => r.classList.remove("drop-before")); });
  el.addEventListener("dragover", (e) => {
    if (!dragging || dragging === task.id) return;
    e.preventDefault();
    document.querySelectorAll(".drop-before").forEach((r) => r.classList.remove("drop-before"));
    el.classList.add("drop-before");
  });
  el.addEventListener("drop", async (e) => {
    if (!dragging) return;
    e.preventDefault(); e.stopPropagation();
    const queue = waitingQueue();
    const from = queue.findIndex((tk) => tk.id === dragging);
    const to = queue.findIndex((tk) => tk.id === task.id);
    const steps = to - from - (to > from ? 1 : 0);
    const id = dragging;
    dragging = null;
    for (let i = 0; i < Math.abs(steps); i++) {
      try { await backend({ op: "move", id, direction: steps < 0 ? "up" : "down" }); } catch (error) { fail(error); break; }
    }
    refresh();
  });
}

// ---------- Menu da linha ----------

function openRowMenu(anchor, task) {
  const s = task.state;
  const close = () => pop.close();
  const connSeg = can.connections(s) && segmented(CONNECTIONS.map((n) => ({ id: n, label: String(n) })), conns(task), (n) => { close(); changeConnections(task, n); });
  const pop = popover(anchor, [
    menuItem("info", tr("Details"), () => { close(); openDetails(task); }),
    s === "concluido" && menuItem("external", tr("Open file"), () => { close(); openFile(task); }),
    menuItem("folder", tr("Open folder"), () => { close(); openFolder(task); }),
    menuItem("copy", tr("Copy address"), () => { close(); navigator.clipboard?.writeText(task.url).then(() => toast(tr("Address copied."))); }),
    can.newUrl(s) && menuItem("link", tr("Try with a new link…"), () => { close(); newLink(task); }),
    can.move(s) && h("hr"),
    can.move(s) && menuItem("to-top", tr("Move to the front of the queue"), () => { close(); move(task, "top"); }),
    can.move(s) && menuItem("arrow-up", tr("Move up in queue"), () => { close(); move(task, "up"); }, "Alt PgUp"),
    can.move(s) && menuItem("arrow-down", tr("Move down in queue"), () => { close(); move(task, "down"); }, "Alt PgDn"),
    connSeg && h("div", { class: "menu-conn" }, h("span", { class: "small muted" }, tr("Requested connections (maximum)")), connSeg),
    can.remove(s) && h("hr"),
    can.remove(s) && menuItem("trash", tr("Remove from history"), () => { close(); removeFromHistory(task); }, "Del"),
    can.remove(s) && menuItem("trash", tr("Delete file from disk…"), () => { close(); deleteFile(task); }),
  ].filter(Boolean), "menu");
}

// ---------- Novo download ----------

const validUrl = (s) => /^https?:\/\/[^\s/$.?#]\S*$/i.test(s);
const parseUrls = (text) => [...new Set(text.split(/\s+/).map((s) => s.trim()).filter(Boolean))];

/**
 * Folha "Novo download". `prefill`: { url, suggested_name, request_id }.
 * Um pedido do Firefox chega com a chave de idempotência "firefox:<id>".
 */
async function openNewDownload(prefill = {}) {
  if (!settings) settings = await backend({ op: "get_settings" }).catch(() => null);
  const fromBrowser = Boolean(prefill.request_id || prefill.from_browser);
  const key = prefill.request_id ? `firefox:${prefill.request_id}` : null;

  const urls = h("textarea", { class: "input", rows: "2", placeholder: tr("https://… (one address per line for several)"), "aria-label": tr("Address (http:// or https://)"), spellcheck: "false" });
  urls.value = prefill.url ?? "";
  const urlHint = h("p", { class: "hint" });
  const probeBox = h("div");
  const name = h("input", { class: "input", "aria-label": tr("File name") });
  if (prefill.suggested_name) { name.value = prefill.suggested_name; name.dataset.touched = "1"; }
  const nameField = h("div", { class: "field" }, h("label", {}, tr("File name")), name);
  let folderPath = settings?.default_destination_dir ?? "";
  const folder = h("input", { class: "input mono", value: folderPath, readonly: true, "aria-label": tr("Folder") });
  const sha = h("input", { class: "input mono", placeholder: tr("Optional"), "aria-label": tr("Expected SHA-256 (optional)"), spellcheck: "false" });
  const shaHint = h("p", { class: "hint" }, tr("Verification only confirms that the file matches the supplied hash; trust depends on where the hash came from."));
  let connections = PROFILE[settings?.default_connections] ?? 4;
  const conn = segmented(CONNECTIONS.map((n) => ({ id: n, label: String(n) })), connections, (n) => (connections = n));
  const errorBox = h("div");

  const advanced = h("div", { class: "form-grid", hidden: true },
    h("div", { class: "field full" }, h("label", {}, tr("Expected SHA-256")), sha, shaHint),
    h("div", { class: "field full" }, h("span", { class: "label" }, tr("Connections")), conn, h("p", { class: "hint" }, tr("Maximum requested from the server; it may accept fewer"))),
  );
  const moreBtn = h("button", { class: "more-toggle", "aria-expanded": "false", onclick: () => {
    const open = moreBtn.getAttribute("aria-expanded") !== "true";
    moreBtn.setAttribute("aria-expanded", String(open)); advanced.hidden = !open;
  } }, icon("chevron-right", 15), tr("More options: SHA-256 and connections"));

  const startBtn = button(tr("Download"), { kind: "primary", icon: "download" });
  const shaValid = () => { const v = sha.value.trim(); return !v || /^[0-9a-f]{64}$/i.test(v); };

  let probeSeq = 0;
  let lastProbe = null;
  let debounce;
  const refreshForm = () => {
    const list = parseUrls(urls.value);
    const bad = list.filter((u) => !validUrl(u));
    urlHint.className = `hint ${bad.length ? "err" : ""}`;
    urlHint.textContent = bad.length ? trf("Invalid address: {url}. Use http:// or https://.", { url: bad[0] })
      : list.length > 1 ? trf("{n} addresses, with the same folder and options.", { n: list.length })
      : tr("Tip: paste with Ctrl+V anywhere in the window, or drag a link here.");
    nameField.hidden = list.length > 1;
    startBtn.disabled = !list.length || bad.length > 0 || !shaValid();
    clear(startBtn, icon("download", 16), list.length > 1 ? trf("Download {n} files", { n: list.length }) : tr("Download"));
    if (list.length !== 1 || bad.length) {
      lastProbe = null; probeSeq++;
      clear(probeBox, list.length > 1 && h("div", { class: "probe multi" }, h("span", { class: "lead" }, icon("layers", 20)), h("div", { class: "meta" }, h("strong", {}, tr("Multiple links")), h("span", {}, tr("Each address becomes a download in the queue.")))));
      return;
    }
    if (lastProbe === list[0]) return;
    lastProbe = list[0];
    const mine = ++probeSeq;
    clearTimeout(debounce);
    debounce = setTimeout(async () => {
      if (mine !== probeSeq) return;
      clear(probeBox, h("div", { class: "probe loading" }, h("span", { class: "spinner" }), tr("Contacting the server…")));
      try {
        const info = await backend({ op: "probe", url: list[0] });
        if (mine !== probeSeq) return;
        if (!name.dataset.touched) name.value = info.filename;
        const details = [info.size != null ? trf("Size reported by the server: {s}", { s: bytes(info.size) }) : tr("The server did not report the size."), host(list[0])];
        clear(probeBox, h("div", { class: "probe" },
          fileTile({ filename: info.filename, state: "aguardando" }),
          h("div", { class: "meta" }, h("strong", {}, info.filename), h("span", {}, details.join(" · "))),
          info.accepts_ranges === false ? badge(tr("One connection only"), "yellow", "alert") : null),
          info.accepts_ranges === false && h("p", { class: "hint", style: { "margin-top": "6px" } }, tr("The server does not support downloading in parts: only one connection will be used.")));
      } catch {
        if (mine === probeSeq) clear(probeBox);
      }
    }, 600);
  };
  urls.addEventListener("input", refreshForm);
  name.addEventListener("input", () => (name.dataset.touched = "1"));
  sha.addEventListener("input", () => {
    const v = sha.value.trim();
    shaHint.className = `hint ${shaValid() ? (v ? "ok" : "") : "err"}`;
    shaHint.textContent = !v ? tr("Verification only confirms that the file matches the supplied hash; trust depends on where the hash came from.")
      : shaValid() ? tr("Valid format.") : tr("The expected SHA-256 must contain 64 hexadecimal digits.");
    refreshForm();
  });

  const choose = button(tr("Choose…"), { icon: "folder", onClick: async () => {
    const picked = await pickFolder(tr("Choose destination folder"), folderPath).catch(() => null);
    if (picked) { folderPath = picked; folder.value = picked; }
  } });

  startBtn.addEventListener("click", () => busy(startBtn, async () => {
    clear(errorBox);
    const list = parseUrls(urls.value);
    const filename = list.length === 1 ? name.value.trim() || null : null;
    const shaValue = sha.value.trim() || null;
    const added = [];
    const failures = [];
    for (const [i, url] of list.entries()) {
      try {
        const result = await backend({
          op: "add_download", url, filename, destination_dir: folderPath || null, connections, expected_sha256: shaValue,
          source: fromBrowser ? "navegador" : "interface",
          idempotency_key: key ? (list.length === 1 ? key : `${key}-${i}`) : null,
        });
        added.push(result.filename);
      } catch (error) {
        failures.push(`${url}: ${error.message}`);
      }
    }
    if (added.length) toast(added.length === 1 ? trf("Download added: {name}", { name: added[0] }) : trf("{n} downloads added", { n: added.length }), { ic: "download" });
    refresh();
    if (failures.length) {
      clear(errorBox, banner("danger", "alert-circle", tr("Error"), failures.join("\n")));
      return;
    }
    close();
    navigate("active");
  }));

  const close = sheet(tr("New download"), [
    h("div", { class: "form-grid" },
      h("div", { class: "field full" }, h("label", {}, tr("Address (http:// or https://)")), urls, urlHint),
      h("div", { class: "full" }, probeBox),
      h("div", { class: "field full" }, nameField),
      h("div", { class: "field full" }, h("label", {}, tr("Folder")), h("div", { class: "folder-pick" }, folder, choose))),
    h("div", { style: { "margin-top": "14px" } }, moreBtn),
    advanced,
    h("div", { style: { "margin-top": "14px" } }, errorBox),
    h("div", { class: "sheet-actions" }, button(tr("Cancel"), { onClick: () => close() }), startBtn),
  ]);
  urls.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || !urls.value.includes("\n")) && !startBtn.disabled) { e.preventDefault(); startBtn.click(); }
  });
  refreshForm();
  requestAnimationFrame(() => urls.focus());
}

// ---------- Detalhes ----------

function openDetails(task) {
  const kv = (rows) => h("dl", { class: "kv wide" }, rows.filter(Boolean).map(([k, v]) => [h("dt", {}, k), h("dd", { class: "selectable" }, v ?? "—")]));
  const f = fraction(task);
  const shaState = { confere: [tr("Matches"), "green"], nao_confere: [tr("Does not match"), "red"], nao_verificado: [tr("Not verified"), ""] }[task.hash_verification] ?? [tr("Not verified"), ""];
  const close = sheet(tr("Download details"), [
    h("div", { class: "details-head" }, fileTile(task), h("div", {}, h("h3", { class: "selectable" }, task.filename),
      h("p", {}, STATE[task.state]?.label() ?? task.state, task.state === "baixando" ? ` · ${rate(task.download_speed)}` : ""))),
    task.state !== "concluido" && f != null && h("div", { style: { "margin-bottom": "18px" } }, meter(f, { warnAt: 2, critAt: 2, lg: true })),
    task.error_message && h("div", { style: { "margin-bottom": "16px" } }, banner("danger", "alert-circle", tr("Error"), task.error_message)),
    kv([
      [tr("Address"), h("span", { class: "mono" }, task.url)],
      [tr("Folder"), h("span", { class: "mono" }, task.destination_dir)],
      [tr("Size"), task.total_bytes != null ? bytes(task.total_bytes) : tr("Unknown")],
      [tr("Received"), bytes(task.downloaded_bytes)],
      [tr("Requested connections (maximum)"), String(conns(task))],
      task.state === "baixando" && [tr("Currently active connections"), String(task.active_connections)],
      task.state === "baixando" && [tr("Speed"), rate(task.download_speed)],
      [tr("Expected SHA-256"), task.expected_sha256 ? h("span", { class: "hash" }, task.expected_sha256) : tr("Not provided")],
      [tr("Verification"), badge(shaState[0], shaState[1])],
      [tr("Created"), dateTime(task.created_at)],
      [tr("Updated"), dateTime(task.updated_at)],
      [tr("Task identifier"), h("span", { class: "mono" }, task.id)],
      [tr("aria2 GID"), task.aria2_gid ? h("span", { class: "mono" }, task.aria2_gid) : "—"],
    ]),
    h("div", { class: "sheet-actions" },
      button(tr("Copy address"), { icon: "copy", onClick: () => navigator.clipboard?.writeText(task.url).then(() => toast(tr("Address copied."))) }),
      button(tr("Open folder"), { icon: "folder", onClick: () => openFolder(task) }),
      task.state === "concluido" && button(tr("Open file"), { kind: "primary", icon: "external", onClick: () => { close(); openFile(task); } }),
      can.retry(task.state) && button(tr("Try again"), { kind: "primary", icon: "refresh", onClick: () => { close(); retry(task); } })),
  ]);
}

// ---------- Preferências ----------

async function openPreferences() {
  let s;
  try { s = await backend({ op: "get_settings" }); } catch (error) { fail(error); return; }
  settings = s;
  const save = async (changes) => {
    const next = { ...settings, ...changes };
    try { await backend({ op: "set_settings", settings: next }); settings = next; } catch (error) { fail(error); }
  };
  const number = (value, min, max, onChange, label, suffix) => {
    const el = h("input", { class: "input", type: "number", min: String(min), max: String(max), value: String(value), "aria-label": label, style: { width: "110px" } });
    el.addEventListener("change", () => { const v = Math.min(max, Math.max(min, Math.round(Number(el.value) || 0))); el.value = String(v); onChange(v); });
    return h("span", { class: "row-flex" }, el, suffix && h("span", { class: "small muted" }, suffix));
  };
  const sw = (key, label) => toggle(settings[key], (v) => save({ [key]: v }), label);
  const row = (title, subtitle, suffix) => h("div", { class: "list-row" }, h("div", { class: "body" }, h("div", { class: "title" }, title), subtitle && h("div", { class: "subtitle wrap" }, subtitle)), h("div", { class: "suffix" }, suffix));
  const folder = h("span", { class: "mono small" }, settings.default_destination_dir);
  const choose = button(tr("Choose…"), { size: "sm", icon: "folder", onClick: async () => {
    const picked = await pickFolder(tr("Choose default folder"), settings.default_destination_dir).catch(() => null);
    if (picked) { await save({ default_destination_dir: picked }); folder.textContent = settings.default_destination_dir; }
  } });
  const restart = button(tr("Restart"), { size: "sm", icon: "refresh" });
  restart.addEventListener("click", () => busy(restart, () => op({ op: "restart_engine" }, tr("Download engine restarted."))));
  sheet(tr("Preferences"), [
    h("div", { class: "pref-group" }, h("h4", {}, tr("Downloads")), list(
      row(tr("Default folder"), folder, choose),
      row(tr("Connections per download"), tr("Default for new downloads; a maximum, not a guarantee"),
        segmented(CONNECTIONS.map((n) => ({ id: n, label: String(n) })), PROFILE[settings.default_connections] ?? 4, (n) => save({ default_connections: PROFILE_NAME[n] }))),
      row(tr("Simultaneous downloads"), tr("Independent of the number of connections for each download"),
        number(settings.max_concurrent_downloads, 1, 10, (v) => save({ max_concurrent_downloads: v }), tr("Simultaneous downloads"))),
      row(tr("Total speed limit (KiB/s)"), tr("0 = unlimited"),
        number(Math.round(settings.global_speed_limit_bytes / 1024), 0, 10000000, (v) => save({ global_speed_limit_bytes: v * 1024 }), tr("Total speed limit (KiB/s)"), "KiB/s")),
    )),
    h("div", { class: "pref-group" }, h("h4", {}, tr("Behavior")), list(
      row(tr("Resume downloads automatically"), tr("Resumes downloads paused by the system (engine restart or reboot). Downloads you paused stay paused."), sw("auto_resume_on_start", tr("Resume downloads automatically"))),
      row(tr("Notify when finished"), null, sw("notifications_enabled", tr("Notify when finished"))),
    )),
    h("div", { class: "pref-group" }, h("h4", {}, tr("Service")), list(
      row(tr("Download engine"), snap.engine_running ? tr("aria2 is running") : tr("Stopped"), restart),
    )),
  ], { wide: false });
}

// ---------- Busca (downloads e ações) ----------

function openPalette(initial = "") {
  const input = h("input", { placeholder: tr("Search downloads or actions…"), "aria-label": tr("Search"), role: "combobox", "aria-expanded": "true", "aria-controls": "palette-list" });
  input.value = initial;
  const listEl = h("div", { class: "palette-list", id: "palette-list", role: "listbox" });
  const actions = [
    { icon: "plus", label: tr("New download"), hint: "Ctrl N", run: () => openNewDownload() },
    { icon: "pause", label: tr("Pause all"), run: () => op({ op: "pause_all" }) },
    { icon: "play", label: tr("Resume all"), run: () => op({ op: "resume_all" }) },
    { icon: "cog", label: tr("Preferences"), hint: "Ctrl ,", run: openPreferences },
    ...VIEWS.map((v) => ({ icon: v.icon, label: v.title(), run: () => navigate(v.id) })),
  ];
  let results = [];
  let selected = 0;
  const render = () => {
    const q = normalize(input.value.trim());
    const tasks = snap.tasks.filter((tk) => !q || normalize(tk.filename).includes(q) || normalize(tk.url).includes(q)).slice(0, 8)
      .map((tk) => ({ section: tr("Downloads"), lead: fileTile(tk), label: tk.filename, where: STATE[tk.state]?.label() ?? "", run: () => reveal(tk) }));
    const acts = actions.filter((a) => !q || normalize(a.label).includes(q)).map((a) => ({ ...a, section: tr("Actions") }));
    results = [...tasks, ...acts];
    selected = Math.min(selected, Math.max(0, results.length - 1));
    let last = null;
    clear(listEl, results.length === 0
      ? h("div", { class: "empty", style: { padding: "30px" } }, h("p", {}, trf("Nothing found for “{query}”", { query: input.value })))
      : results.map((r, i) => {
        const head = r.section !== last && h("div", { class: "palette-section" }, r.section);
        last = r.section;
        return [head, h("button", { class: "palette-item", role: "option", "aria-selected": String(i === selected), onclick: () => choose(i) },
          r.lead ?? icon(r.icon, 17), h("span", {}, r.label), r.where && h("span", { class: "where" }, r.where), r.hint && h("kbd", {}, r.hint))];
      }));
  };
  const choose = (i) => { const r = results[i]; if (!r) return; close(); r.run(); };
  input.addEventListener("input", () => { selected = 0; render(); });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") { e.preventDefault(); selected = Math.min(results.length - 1, selected + 1); render(); }
    else if (e.key === "ArrowUp") { e.preventDefault(); selected = Math.max(0, selected - 1); render(); }
    else if (e.key === "Enter") { e.preventDefault(); choose(selected); }
  });
  const close = overlay(h("div", { class: "palette", role: "dialog", "aria-label": tr("Search") },
    h("div", { class: "palette-input" }, icon("search", 19), input, h("kbd", {}, "Esc")), listEl,
    h("div", { class: "palette-foot" }, h("span", {}, h("kbd", {}, "↑↓"), " ", tr("navigate")), h("span", {}, h("kbd", {}, "Enter"), " ", tr("open")))), { top: true });
  render();
  requestAnimationFrame(() => input.focus());
}

function reveal(task) {
  navigate(ACTIVE.includes(task.state) ? "active" : task.state === "concluido" ? "done" : task.state === "erro" ? "failed" : "all");
  requestAnimationFrame(() => { select(task.id); document.querySelector(`.dl-row[data-id="${task.id}"]`)?.focus(); });
}

// ---------- Páginas ----------

const matches = (tk) => !filter || normalize(tk.filename).includes(normalize(filter)) || normalize(tk.url).includes(normalize(filter));
const waitingQueue = () => snap.tasks.filter((tk) => tk.state === "aguardando").sort((a, b) => a.queue_position - b.queue_position);
const queueIndex = (task) => waitingQueue().findIndex((tk) => tk.id === task.id) + 1;
const rowsList = (tasks, opts) => h("div", { class: "list", role: "list" }, tasks.map((tk) => downloadRow(tk, opts)));

function serviceBanner() {
  if (serviceError) return h("div", { style: { "margin-bottom": "18px" } }, banner("danger", "alert-circle", tr("Download service unavailable"), serviceError));
  if (!snap.engine_problem) return null;
  const restart = button(tr("Try starting the engine"));
  restart.addEventListener("click", () => busy(restart, () => op({ op: "restart_engine" })));
  return h("div", { style: { "margin-bottom": "18px" } }, banner("danger", "alert-circle", tr("The download engine stopped"), snap.engine_problem, restart));
}

function emptyFor(v) {
  const add = button(tr("New download"), { kind: "primary pill", icon: "plus", onClick: () => openNewDownload() });
  switch (v) {
    case "active": return empty("download", tr("Nothing in progress"), tr("Paste a link with Ctrl+V, drag a link into the window or use “Download with Lyra Downloads” in a Firefox link menu. Closing the window does not stop downloads."), add);
    case "done": return empty("check-circle", tr("No completed downloads"), tr("Downloaded files appear here, grouped by day."), null, "green");
    case "failed": return empty("check-circle", tr("No errors"), tr("Failed downloads appear here, with an option to try again."), null, "green");
    default: return empty("download", tr("No downloads"), tr("Use “New download” or, in Firefox, “Download with Lyra Downloads” in a link's menu. Closing the window does not stop downloads."), add);
  }
}

function summary() {
  const downloading = snap.tasks.filter((tk) => tk.state === "baixando");
  const queued = snap.tasks.filter((tk) => tk.state === "aguardando").length;
  const paused = snap.tasks.filter((tk) => tk.state === "pausado").length;
  // Termina quando o download ativo mais demorado terminar.
  const secs = Math.max(0, ...downloading.map(remainingSecs).filter((v) => v != null));
  return h("div", { class: "dl-summary" },
    h("span", {}, trf("{n} downloading", { n: downloading.length })),
    queued > 0 && [h("span", { class: "sep" }), h("span", {}, trf("{n} queued", { n: queued }))],
    paused > 0 && [h("span", { class: "sep" }), h("span", {}, trf("{n} paused", { n: paused }))],
    h("span", { class: "sep" }), h("span", {}, h("b", {}, trf("Total speed: {v}", { v: rate(snap.total_speed) }))),
    secs > 0 && [h("span", { class: "sep" }), h("span", {}, trf("finishes in about {time}", { time: secondsLabel(secs) }))]);
}

function renderPage() {
  const tasks = snap.tasks.filter((tk) => byView.get(view).accepts(tk.state) && matches(tk));
  let content;
  if (view === "active") {
    const downloading = tasks.filter((tk) => tk.state === "baixando" || tk.state === "verificando").sort((a, b) => a.queue_position - b.queue_position);
    const waiting = tasks.filter((tk) => tk.state === "aguardando").sort((a, b) => a.queue_position - b.queue_position);
    const paused = tasks.filter((tk) => tk.state === "pausado").sort((a, b) => a.queue_position - b.queue_position);
    content = tasks.length === 0 ? [serviceBanner(), emptyFor(view)] : [
      serviceBanner(),
      summary(),
      downloading.length > 0 && section(tr("Downloading"), {}, rowsList(downloading)),
      waiting.length > 0 && section(tr("Queued"), { sub: waiting.length > 1 ? tr("drag to reorder") : null, actions: [button(tr("Pause all"), { size: "sm", kind: "flat", icon: "pause", onClick: () => op({ op: "pause_all" }) })] }, rowsList(waiting, { draggable: waiting.length > 1 })),
      paused.length > 0 && section(tr("Paused"), { actions: [button(tr("Resume all"), { size: "sm", kind: "flat", icon: "play", onClick: () => op({ op: "resume_all" }) })] }, rowsList(paused)),
    ];
  } else if (view === "done") {
    const groups = new Map();
    for (const tk of [...tasks].sort((a, b) => b.updated_at.localeCompare(a.updated_at))) {
      const day = dayLabel(tk.updated_at);
      if (!groups.has(day)) groups.set(day, []);
      groups.get(day).push(tk);
    }
    content = tasks.length === 0 ? [serviceBanner(), emptyFor(view)] : [serviceBanner(), [...groups].map(([day, items]) => section(day, { sub: trf("{n} files", { n: items.length }) }, rowsList(items)))];
  } else if (view === "failed") {
    content = tasks.length === 0 ? [serviceBanner(), emptyFor(view)] : [
      serviceBanner(),
      h("div", { class: "dl-summary" }, tr("Try again, or use a new link when the previous one expired.")),
      rowsList(tasks),
      tasks.length > 1 && h("div", { class: "row-flex", style: { "margin-top": "14px", "justify-content": "flex-end" } },
        button(tr("Try all again"), { icon: "refresh", onClick: async () => { for (const tk of tasks) await backend({ op: "retry", id: tk.id }).catch(fail); refresh(); } })),
    ];
  } else {
    const order = (tk) => ({ baixando: 0, verificando: 1, aguardando: 2, pausado: 3, erro: 4, cancelado: 5, concluido: 6 }[tk.state] ?? 7);
    content = tasks.length === 0 ? [serviceBanner(), emptyFor(view)] : [serviceBanner(),
      rowsList([...tasks].sort((a, b) => order(a) - order(b) || (order(a) <= 3 ? a.queue_position - b.queue_position : b.updated_at.localeCompare(a.updated_at))))];
  }
  const focusedId = document.activeElement?.closest?.(".dl-row")?.dataset.id;
  clear($("page"), content);
  if (focusedId) document.querySelector(`.dl-row[data-id="${focusedId}"]`)?.focus();
}

// Atualização ao vivo: com as mesmas linhas, só repinta cada uma (sem perder
// foco, popover aberto ou rolagem); mudou o conjunto, redesenha a página.
let lastKey = "";
function update() {
  renderNav();
  renderEngine();
  setCrumb();
  const key = snap.tasks.filter((tk) => byView.get(view).accepts(tk.state) && matches(tk)).map((tk) => `${tk.id}:${tk.state}:${tk.queue_position}`).join("|")
    + `|${Boolean(snap.engine_problem)}|${Boolean(serviceError)}|${new Date().toDateString()}`;
  if (document.querySelector(".dl-row.dragging")) return;
  if (key !== lastKey) { lastKey = key; renderPage(); return; }
  const byId = new Map(snap.tasks.map((tk) => [tk.id, tk]));
  // Não repinta a linha cujo botão está com o foco do teclado.
  document.querySelectorAll(".dl-row").forEach((row) => {
    const tk = byId.get(row.dataset.id);
    const focusInside = row.contains(document.activeElement) && row !== document.activeElement;
    if (tk && !focusInside) row.update(tk);
  });
  const sum = document.querySelector(".page > .dl-summary");
  if (sum && view === "active") sum.replaceWith(summary());
}

let refreshing = false;
async function refresh() {
  if (refreshing) return;
  refreshing = true;
  try {
    snap = await backend({ op: "list_tasks" });
    serviceError = null;
  } catch (error) {
    serviceError = error.message;
  } finally {
    refreshing = false;
  }
  history.push(snap.total_speed || 0);
  history.shift();
  update();
}

// ---------- Moldura: navegação, motor, cabeçalho ----------

function counts() {
  const c = {};
  for (const v of VIEWS) c[v.id] = snap.tasks.filter((tk) => v.accepts(tk.state)).length;
  return c;
}

function renderNav() {
  const c = counts();
  clear($("nav"), h("div", { class: "nav-group" }, VIEWS.map((v) =>
    h("button", { class: "nav-item", "aria-current": v.id === view ? "page" : null, onclick: () => navigate(v.id) },
      icon(v.icon, 18), h("span", {}, v.title()),
      c[v.id] > 0 && v.id !== "all" ? h("span", { class: `count ${v.id === "failed" ? "warn" : v.id === "done" ? "muted-count" : ""}`.trim() }, String(c[v.id])) : null))));
}

function renderEngine() {
  $("engine-rate").textContent = rate(snap.total_speed || 0);
  const ok = !serviceError && !snap.engine_problem;
  $("engine-dot").className = `status-dot ${ok ? "" : "bad"}`.trim();
  $("engine-label").textContent = serviceError ? tr("Service unavailable") : snap.engine_problem ? tr("Engine stopped") : tr("aria2 · running");
  const max = Math.max(1, ...history);
  const pts = history.map((v, i) => `${(i / (history.length - 1)) * 120},${26 - (v / max) * 22}`);
  $("engine-spark").querySelector(".line").setAttribute("d", `M${pts.join(" L")}`);
  $("engine-spark").querySelector(".area").setAttribute("d", `M0,26 L${pts.join(" L")} L120,26 Z`);
}

function setCrumb() {
  const c = counts();
  $("page-crumb").textContent = view === "active" && snap.total_speed ? rate(snap.total_speed) : trf("{n} items", { n: c[view] });
}

function headerActions() {
  const search = h("input", { class: "input", placeholder: tr("Search by name or address"), "aria-label": tr("Search by name or address") });
  search.id = "filter";
  search.addEventListener("input", () => { filter = search.value; lastKey = ""; update(); });
  search.addEventListener("keydown", (e) => { if (e.key === "Escape") { search.value = ""; filter = ""; lastKey = ""; update(); search.blur(); } });
  return [
    h("div", { class: "input-icon header-filter" }, icon("filter", 15), search),
    iconButton("pause", tr("Pause all"), () => op({ op: "pause_all" }, tr("All downloads paused."))),
    button(tr("New download"), { kind: "primary", icon: "plus", onClick: () => openNewDownload(), title: tr("New download (Ctrl+N)") }),
    h("span", { class: "header-sep" }),
  ];
}

function navigate(id) {
  if (!byView.has(id)) id = "active";
  view = id;
  window.history.replaceState(null, "", `#/${id}`);
  $("page-title").textContent = byView.get(id).title();
  document.title = `${byView.get(id).title()} — Lyra Downloads`;
  lastKey = "";
  update();
  $("scroller").scrollTop = 0;
  $("page").classList.remove("enter"); void $("page").offsetWidth; $("page").classList.add("enter");
}

function openMainMenu(anchor) {
  const pop = popover(anchor, [
    menuItem("plus", tr("New download"), () => { pop.close(); openNewDownload(); }, "Ctrl N"),
    menuItem("pause", tr("Pause all"), () => { pop.close(); op({ op: "pause_all" }); }),
    menuItem("play", tr("Resume all"), () => { pop.close(); op({ op: "resume_all" }); }),
    h("hr"),
    menuItem("cog", tr("Preferences"), () => { pop.close(); openPreferences(); }, "Ctrl ,"),
    menuItem("keyboard", tr("Keyboard shortcuts"), () => { pop.close(); openShortcuts(); }, "Ctrl ?"),
    menuItem("star", tr("About Lyra Downloads"), () => { pop.close(); openAbout(); }),
    h("hr"),
    menuItem("power", tr("Pause all and stop the service…"), () => { pop.close(); quitService(); }),
  ], "menu");
}

async function quitService() {
  const ok = await confirm({
    title: tr("Pause all and quit?"),
    text: tr("All downloads will be paused and the background service will stop. You can resume them the next time you open Lyra Downloads."),
    ic: "power", confirmLabel: tr("Pause and quit"), confirmKind: "danger",
  });
  if (ok) call("quit_service").catch(fail);
}

function openShortcuts() {
  const rows = [[tr("New download"), "Ctrl N"], [tr("Paste a link to download"), "Ctrl V"], [tr("Search"), "Ctrl K"], [tr("Filter the list"), "Ctrl F"],
    [tr("Preferences"), "Ctrl ,"], [tr("Pause or resume the selected download"), "Space"], [tr("Open file or details"), "Enter"],
    [tr("Remove from history"), "Del"], [tr("Move up or down in the queue"), "Alt PgUp/PgDn"], [tr("Go to list 1–4"), "Alt 1…4"],
    [tr("Close the window (downloads continue)"), "Ctrl W"], [tr("Close dialog"), "Esc"]];
  sheet(tr("Keyboard shortcuts"), list(rows.map(([label, keys]) => h("div", { class: "list-row" }, h("div", { class: "body" }, label), h("div", { class: "suffix" }, keys.split(" ").map((k) => h("kbd", {}, k)))))), { wide: false });
}

function openAbout() {
  const link = (label, uri) => h("button", { class: "btn flat sm", onclick: () => call("open_file", { path: uri }).catch(() => {}) }, icon("external", 14), label);
  sheet(tr("About Lyra Downloads"), [
    h("div", { style: { "text-align": "center", padding: "6px 0 4px" } },
      h("div", { class: "brand-mark", style: { width: "72px", height: "72px", "border-radius": "20px", margin: "0 auto 12px" } }, icon("download", 38)),
      h("h2", { style: { "font-size": "22px", "font-weight": "800" } }, "Lyra Downloads"),
      h("p", { class: "muted" }, appInfo.version),
      h("p", { class: "muted" }, tr("Download manager with parallel connections, powered by aria2.")),
      h("p", { class: "small faint" }, `${tr("Lyra OS Project")} · GPL-3.0-or-later`)),
    h("div", { class: "row-flex", style: { "justify-content": "center" } }, link("GitHub", "https://github.com/lyra-os-linux/lyra-downloads")),
  ], { wide: false });
}

// ---------- Janela, colar e soltar ----------

function setupWindow() {
  const controls = $("window-controls");
  const make = (name, label, fn) => h("button", { class: "wc", "aria-label": label, title: label, onclick: fn }, icon(name, 13));
  const maxBtn = make("maximize", tr("Maximize"), () => win.toggleMaximize());
  controls.append(make("minus", tr("Minimize"), () => win.minimize()), maxBtn, make("x", tr("Close"), () => win.close()));
  document.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || !e.target.closest("[data-drag]") || e.target.closest("button, input, select")) return;
    if (e.detail === 2) win.toggleMaximize(); else win.startDragging();
  });
  for (const dir of ["North", "South", "East", "West", "NorthEast", "NorthWest", "SouthEast", "SouthWest"]) {
    document.body.append(h("div", { class: "resize", dataset: { dir }, onmousedown: (e) => { e.preventDefault(); win.startResizeDragging(dir); } }));
  }
  const sync = async () => {
    const max = await win.isMaximized();
    document.body.classList.toggle("maximized", max);
    clear(maxBtn, icon(max ? "restore" : "maximize", 13));
    maxBtn.title = maxBtn.ariaLabel = max ? tr("Restore") : tr("Maximize");
  };
  win.onResized(sync);
  sync();
}

function setupDropAndPaste() {
  const hint = $("drop-hint");
  const links = (text) => parseUrls(text || "").filter(validUrl);
  let depth = 0;
  document.addEventListener("dragenter", (e) => {
    if (dragging || ![...e.dataTransfer.types].some((ty) => ty === "text/uri-list" || ty === "text/plain")) return;
    depth++; hint.classList.add("on");
  });
  document.addEventListener("dragleave", () => { if (--depth <= 0) { depth = 0; hint.classList.remove("on"); } });
  document.addEventListener("dragover", (e) => { if (hint.classList.contains("on")) e.preventDefault(); });
  document.addEventListener("drop", (e) => {
    if (!hint.classList.contains("on")) return;
    e.preventDefault(); depth = 0; hint.classList.remove("on");
    const found = links(e.dataTransfer.getData("text/uri-list") || e.dataTransfer.getData("text/plain"));
    if (found.length) openNewDownload({ url: found.join("\n") });
    else toast(tr("This does not look like an http:// or https:// link."), { ic: "alert-circle" });
  });
  document.addEventListener("paste", (e) => {
    if (e.target.closest("input, textarea") || document.querySelector(".overlay")) return;
    const found = links(e.clipboardData.getData("text/plain"));
    if (found.length) { e.preventDefault(); openNewDownload({ url: found.join("\n") }); }
  });
}

function setupKeys() {
  document.addEventListener("keydown", (e) => {
    if (document.querySelector(".overlay")) return;
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key.toLowerCase();
    if (mod && k === "n") { e.preventDefault(); openNewDownload(); }
    else if (mod && k === "k") { e.preventDefault(); openPalette(); }
    else if (mod && k === "f") { e.preventDefault(); $("filter")?.focus(); }
    else if (mod && e.key === ",") { e.preventDefault(); openPreferences(); }
    else if (mod && e.key === "?") { e.preventDefault(); openShortcuts(); }
    else if (mod && (k === "w" || k === "q")) { e.preventDefault(); win.close(); }
    else if (e.altKey && /^[1-4]$/.test(e.key)) { e.preventDefault(); navigate(VIEWS[Number(e.key) - 1].id); }
  });
}

async function start() {
  appInfo = await call("app_info");
  await loadCatalog(call, appInfo.locale);
  applyInterface(await call("interface_style").catch(() => ({})));
  on("interface-changed", applyInterface);

  $("brand-mark").innerHTML = svg("download", 15);
  $("search-ic").innerHTML = svg("search", 15);
  $("menu-btn").innerHTML = svg("menu", 17);
  $("sidebar-toggle").innerHTML = svg("sidebar", 17);
  $("engine-ic").innerHTML = svg("arrow-down", 15);
  $("drop-ic").innerHTML = svg("link", 34);
  $("search-label").textContent = tr("Search downloads");
  $("drop-title").textContent = tr("Drop the link to download");
  $("drop-sub").textContent = tr("http:// or https:// addresses");
  $("menu-btn").title = $("menu-btn").ariaLabel = tr("Main menu");
  $("sidebar-toggle").title = $("sidebar-toggle").ariaLabel = tr("Show navigation");
  $("menu-btn").addEventListener("click", (e) => openMainMenu(e.currentTarget));
  $("search-trigger").addEventListener("click", () => openPalette());
  $("engine-chip").addEventListener("click", openPreferences);
  $("sidebar-toggle").addEventListener("click", () => $("sidebar").classList.toggle("open"));
  $("scroller").addEventListener("scroll", (e) => $("headerbar").classList.toggle("scrolled", e.target.scrollTop > 4), { passive: true });
  clear($("header-actions"), headerActions());
  setupWindow();
  setupDropAndPaste();
  setupKeys();

  // Pedidos do Firefox: o da primeira execução e os repassados depois.
  on("add-url", (request) => openNewDownload(request));
  navigate(location.hash.split("/")[1] || "active");
  await refresh();
  setInterval(() => { if (!document.hidden) refresh(); }, 1000);
  backend({ op: "get_settings" }).then((s) => (settings = s)).catch(() => {});
  const first = await call("take_launch_request").catch(() => null);
  if (first) openNewDownload(first);
}

start().catch((error) => call("app_log", { message: `start: ${error?.stack ?? error}` }).catch(() => {}));
