import { DEFAULT_EXTENSIONS, normalizeDomain, normalizeExtension, } from "./rules.js";
import { applyCaptureSettings, applyI18n, loadSettings, t } from "./settings.js";
applyI18n();
const $ = (id) => document.getElementById(id);
const autoCapture = $("auto-capture");
const scopeAll = $("scope-all");
const scopeListed = $("scope-listed");
const anyType = $("any-type");
const listedTypes = $("listed-types");
const cookiesAll = $("cookies-all");
const showApp = $("show-app");
const rulesBody = $("rules");
const newDomain = $("new-domain");
const extensionsInput = $("extensions");
const message = $("message");
let rules = [];
function show(text, isError = false) {
    message.textContent = text;
    message.className = isError ? "error" : "info";
}
function syncVisibility() {
    $("listed-section").hidden = !scopeListed.checked;
    $("cookies-all-row").hidden = !scopeAll.checked;
    $("types-section").hidden = anyType.checked;
    $("capture-details").classList.toggle("disabled", !autoCapture.checked);
}
function renderRules() {
    rulesBody.replaceChildren();
    rules.forEach((rule, i) => {
        const tr = document.createElement("tr");
        const tdDomain = document.createElement("td");
        tdDomain.textContent = rule.domain;
        const tdCookies = document.createElement("td");
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.checked = rule.allowCookies;
        cb.setAttribute("aria-label", t("allowCookiesFor", rule.domain));
        cb.addEventListener("change", () => (rules[i].allowCookies = cb.checked));
        tdCookies.append(cb);
        const tdRemove = document.createElement("td");
        const rm = document.createElement("button");
        rm.textContent = t("remove");
        rm.addEventListener("click", () => {
            rules.splice(i, 1);
            renderRules();
        });
        tdRemove.append(rm);
        tr.append(tdDomain, tdCookies, tdRemove);
        rulesBody.append(tr);
    });
    $("no-rules").hidden = rules.length > 0;
}
function readExtensions() {
    const parts = extensionsInput.value.split(/[,\s]+/).filter(Boolean);
    const normalized = parts.map(normalizeExtension);
    if (normalized.some((e) => e === null) || normalized.length === 0)
        return null;
    return [...new Set(normalized)];
}
for (const el of [autoCapture, scopeAll, scopeListed, anyType, listedTypes]) {
    el.addEventListener("change", syncVisibility);
}
$("add-domain").addEventListener("click", () => {
    const d = normalizeDomain(newDomain.value);
    if (!d) {
        show(t("errInvalidDomain"), true);
        return;
    }
    if (!rules.some((r) => r.domain === d))
        rules.push({ domain: d, allowCookies: false });
    newDomain.value = "";
    renderRules();
});
$("reset-extensions").addEventListener("click", () => {
    extensionsInput.value = DEFAULT_EXTENSIONS.join(", ");
});
$("save").addEventListener("click", () => {
    const extensions = readExtensions();
    if (!anyType.checked && !extensions) {
        show(t("errInvalidExtensions"), true);
        return;
    }
    const next = {
        autoCapture: autoCapture.checked,
        scope: scopeListed.checked ? "listed" : "all",
        rules: rules.map((r) => ({ ...r })),
        anyFileType: anyType.checked,
        extensions: extensions ?? DEFAULT_EXTENSIONS,
        allowCookiesAll: cookiesAll.checked,
        showApp: showApp.checked,
    };
    if (next.autoCapture && next.scope === "listed" && next.rules.length === 0) {
        show(t("errNeedDomain"), true);
        return;
    }
    void applyCaptureSettings(next).then((r) => {
        show(r.message, !r.ok);
        if (!r.ok) {
            autoCapture.checked = false;
            syncVisibility();
        }
    });
});
void loadSettings().then((s) => {
    autoCapture.checked = s.autoCapture;
    scopeAll.checked = s.scope === "all";
    scopeListed.checked = s.scope === "listed";
    anyType.checked = s.anyFileType;
    listedTypes.checked = !s.anyFileType;
    cookiesAll.checked = s.allowCookiesAll;
    showApp.checked = s.showApp;
    extensionsInput.value = s.extensions.join(", ");
    rules = s.rules.map((r) => ({ ...r }));
    renderRules();
    syncVisibility();
});
