// Página de fundo (event page, MV3). Não carrega código remoto.

import { decide, type Settings, DEFAULT_SETTINGS } from "./rules.js";
import { RequestTracker, headerPresent } from "./tracker.js";
import { callHost, newRequestId, type HostOp } from "./protocol.js";
import { HandoffManager, type HandoffEntry } from "./handoff.js";
import { loadSettings, t } from "./settings.js";

const MENU_ID = "lyra-send-link";
let settings: Settings = DEFAULT_SETTINGS;
const ready = loadSettings().then((s) => (settings = s));
browser.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.settings) settings = { ...DEFAULT_SETTINGS, ...changes.settings.newValue };
});

const sendNative = (host: string, msg: object) => browser.runtime.sendNativeMessage(host, msg);
const host = (op: HostOp, timeoutMs = 8000, requestId = newRequestId()) => callHost(sendNative, requestId, op, timeoutMs);

// ---------------------------------------------------------------- menu --

async function createMenu(): Promise<void> {
  await browser.menus.removeAll();
  browser.menus.create({ id: MENU_ID, title: t("menuSendLink"), contexts: ["link"] });
}
browser.runtime.onInstalled.addListener(() => void createMenu());
browser.runtime.onStartup.addListener(() => void createMenu());

function notify(message: string): void {
  void browser.notifications.create({
    type: "basic",
    iconUrl: browser.runtime.getURL("icons/icon.svg"),
    title: "Lyra Downloads",
    message,
  });
}

browser.menus.onClicked.addListener(async (info) => {
  if (info.menuItemId !== MENU_ID || !info.linkUrl) return;
  let url: URL;
  try {
    url = new URL(info.linkUrl);
  } catch {
    notify(t("errInvalidLink"));
    return;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    notify(t("errOnlyHttp"));
    return;
  }
  const res = await host({ op: "send_download", url: url.href });
  if (res.kind === "host_missing") notify(res.message);
  else if (res.kind === "error") notify(res.message);
  else if (res.kind === "timeout") notify(t("errTimeout"));
});

// ---------------------------------------------------- captura automática --

const tracker = new RequestTracker();

function badge(entries: HandoffEntry[]): void {
  const problem = entries.some((e) => e.state === "parado_no_firefox" || e.state === "pausado_no_firefox");
  void browser.action.setBadgeText({ text: problem ? "!" : "" });
  if (problem) void browser.action.setBadgeBackgroundColor({ color: "#c01c28" });
}

const manager = new HandoffManager({
  pause: (id) => browser.downloads.pause(id),
  resume: (id) => browser.downloads.resume(id),
  cancel: (id) => browser.downloads.cancel(id),
  erase: async (id) => {
    await browser.downloads.erase({ id });
  },
  startDownload: (url) => browser.downloads.download({ url, conflictAction: "uniquify" }),
  call: (requestId, op, timeoutMs) => callHost(sendNative, requestId, op, timeoutMs),
  newId: newRequestId,
  now: Date.now,
  changed: (entries) => {
    badge(entries);
    void browser.storage.session.set({ entries });
  },
});

function onDownloadCreated(item: browser.downloads.DownloadItem): void {
  void ready.then(async () => {
    const info = {
      id: item.id,
      url: item.url,
      filename: item.filename,
      incognito: item.incognito,
      state: item.state,
      byExtensionId: item.byExtensionId,
    };
    if (manager.isGuarded(info)) return;
    const decision = decide(info, settings, tracker.find(item.url), browser.runtime.id);
    if (!decision.capture) {
      if (settings.autoCapture) console.debug("Lyra Downloads: mantido no Firefox —", decision.reason);
      return;
    }
    const entry = await manager.handoff(info);
    if (entry.state === "repassado" && settings.showApp) void host({ op: "open_app" }, 4000);
  });
}

function onSendHeaders(d: browser.webRequest._OnSendHeadersDetails): void {
  tracker.record(d.requestId, {
    url: d.url,
    method: d.method,
    hasCookie: headerPresent(d.requestHeaders, "cookie"),
    hasAuthorization: headerPresent(d.requestHeaders, "authorization"),
    incognito: Boolean((d as { incognito?: boolean }).incognito),
  });
}

function onBeforeRedirect(d: browser.webRequest._OnBeforeRedirectDetails): void {
  tracker.redirect(d.requestId, d.redirectUrl);
}

/** Registra os ouvintes opcionais quando as permissões existem. Chamado no
 * topo (para acordar a página de fundo) e quando permissões são concedidas. */
function registerOptionalListeners(): void {
  if (browser.downloads && !browser.downloads.onCreated.hasListener(onDownloadCreated)) {
    browser.downloads.onCreated.addListener(onDownloadCreated);
  }
  if (browser.webRequest && !browser.webRequest.onSendHeaders.hasListener(onSendHeaders)) {
    // Só recebe eventos dos hosts com permissão concedida (as regras).
    const filter = { urls: ["<all_urls>"], types: ["main_frame", "sub_frame", "other", "xmlhttprequest", "object"] as browser.webRequest.ResourceType[] };
    browser.webRequest.onSendHeaders.addListener(onSendHeaders, filter, ["requestHeaders"]);
    browser.webRequest.onBeforeRedirect.addListener(onBeforeRedirect, filter);
  }
}
registerOptionalListeners();
browser.permissions.onAdded.addListener(registerOptionalListeners);

// ------------------------------------------------------------ mensagens --

type UiMessage =
  | { type: "status" }
  | { type: "settings" }
  | { type: "open-app" }
  | { type: "entries" }
  | { type: "restart"; requestId: string };

browser.runtime.onMessage.addListener((raw: unknown, sender) => {
  if (sender.id !== browser.runtime.id) return undefined;
  const msg = raw as UiMessage;
  switch (msg.type) {
    case "status":
      return host({ op: "health" }, 4000);
    case "settings":
      return ready.then(() => settings);
    case "open-app":
      return host({ op: "open_app" }, 4000);
    case "entries":
      return Promise.resolve(manager.entries);
    case "restart":
      return manager.restartInFirefox(msg.requestId);
    default:
      return undefined;
  }
});
