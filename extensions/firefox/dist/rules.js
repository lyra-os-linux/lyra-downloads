// Regras da captura automática (lógica pura, testável sem navegador).
export const DEFAULT_EXTENSIONS = [
    "iso", "img", "zip", "7z", "rar", "tar", "tar.gz", "tgz", "tar.xz", "tar.zst",
    "gz", "xz", "zst", "bz2", "rpm", "deb", "appimage", "flatpak", "dmg", "exe", "msi",
];
export const DEFAULT_SETTINGS = {
    autoCapture: false,
    scope: "all",
    rules: [],
    anyFileType: true,
    extensions: DEFAULT_EXTENSIONS,
    allowCookiesAll: false,
    showApp: true,
};
export const ALL_SITES_ORIGINS = ["http://*/*", "https://*/*"];
/** Origens que a captura precisa acompanhar com as configurações dadas. */
export function requiredOrigins(s) {
    return s.scope === "all" ? ALL_SITES_ORIGINS : hostPatterns(s.rules);
}
export function normalizeDomain(input) {
    const d = input.trim().toLowerCase().replace(/^\*\./, "").replace(/\.$/, "");
    if (!/^[a-z0-9-]+(\.[a-z0-9-]+)+$/.test(d))
        return null;
    return d;
}
export function normalizeExtension(input) {
    const e = input.trim().toLowerCase().replace(/^\.+/, "");
    return /^[a-z0-9]+(\.[a-z0-9]+)?$/.test(e) ? e : null;
}
/** O host é o domínio da regra ou um subdomínio dele. */
export function hostMatches(host, domain) {
    const h = host.toLowerCase();
    return h === domain || h.endsWith("." + domain);
}
export function findRule(host, rules) {
    return rules.find((r) => hostMatches(host, r.domain));
}
/** Nome-base do caminho final escolhido pelo Firefox (já considera
 * Content-Disposition e tipo MIME, não só o sufixo da URL). */
export function basename(path) {
    const parts = path.split(/[\\/]/);
    return parts[parts.length - 1] ?? "";
}
export function extensionMatches(filename, extensions) {
    const name = basename(filename).toLowerCase();
    return extensions.some((ext) => name.endsWith("." + ext));
}
/** Padrões de URL para as permissões de host e filtros do webRequest. */
export function hostPatterns(rules) {
    return rules.flatMap((r) => [`*://${r.domain}/*`, `*://*.${r.domain}/*`]);
}
const skip = (reason) => ({ capture: false, reason });
/** Decide se um download pode ser repassado. Qualquer dúvida → fica no Firefox. */
export function decide(item, settings, record, ownExtensionId) {
    if (!settings.autoCapture)
        return skip("captura automática desativada");
    if (item.byExtensionId === ownExtensionId)
        return skip("download iniciado pela própria extensão");
    if (item.incognito)
        return skip("navegação privada não participa da captura");
    if (item.state === "complete")
        return skip("download já terminou no Firefox");
    let url;
    try {
        url = new URL(item.url);
    }
    catch {
        return skip("endereço inválido");
    }
    if (url.protocol !== "http:" && url.protocol !== "https:")
        return skip("apenas http/https (blob:, data: e outros ficam no Firefox)");
    if (url.username || url.password)
        return skip("endereço com credenciais embutidas");
    const rule = settings.scope === "all"
        ? { domain: url.hostname, allowCookies: settings.allowCookiesAll }
        : findRule(url.hostname, settings.rules);
    if (!rule)
        return skip("domínio fora das regras");
    if (!settings.anyFileType && !extensionMatches(item.filename, settings.extensions)) {
        return skip("tipo de arquivo fora das regras");
    }
    if (!record)
        return skip("sem informação suficiente sobre a requisição");
    if (record.incognito)
        return skip("navegação privada não participa da captura");
    if (record.method !== "GET")
        return skip(`requisição ${record.method} não é reproduzível`);
    if (record.hasAuthorization)
        return skip("requisição com autenticação especial");
    if (record.hasCookie && !rule.allowCookies)
        return skip("requisição com cookies (pode depender da sessão)");
    return { capture: true, rule, url: item.url };
}
