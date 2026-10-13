import { DEFAULT_SETTINGS, requiredOrigins } from "./rules.js";
export async function loadSettings() {
    const stored = (await browser.storage.local.get("settings")).settings;
    return { ...DEFAULT_SETTINGS, ...(stored ?? {}) };
}
export async function saveSettings(s) {
    await browser.storage.local.set({ settings: s });
}
export function t(key, subs) {
    return browser.i18n.getMessage(key, subs) || key;
}
/** Preenche elementos com `data-i18n="chave"`. */
export function applyI18n(root = document) {
    root.querySelectorAll("[data-i18n]").forEach((el) => {
        el.textContent = t(el.dataset.i18n);
    });
}
/**
 * Aplica as configurações de captura. DEVE ser chamada diretamente dentro de
 * um tratador de clique: o pedido de permissão exige gesto do usuário e é
 * feito antes de qualquer outra espera. Ao desligar, devolve as permissões
 * opcionais ao Firefox.
 */
export function applyCaptureSettings(next) {
    const origins = requiredOrigins(next);
    const request = next.autoCapture
        ? browser.permissions.request({ permissions: ["downloads", "webRequest"], origins })
        : Promise.resolve(true);
    return request.then(async (granted) => {
        if (!granted)
            return { ok: false, message: t("errPermissionDenied") };
        const all = await browser.permissions.getAll();
        const keep = new Set(next.autoCapture ? origins : []);
        const remove = (all.origins ?? []).filter((o) => !keep.has(o));
        if (remove.length)
            await browser.permissions.remove({ origins: remove });
        if (!next.autoCapture)
            await browser.permissions.remove({ permissions: ["webRequest", "downloads"] });
        await saveSettings(next);
        return { ok: true, message: t("saved") };
    });
}
