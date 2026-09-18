// Regras da captura automática (lógica pura, testável sem navegador).

export interface DomainRule {
  domain: string;
  /** Capturar mesmo quando o navegador enviou cookies. Desligado por padrão:
   * sem isso não há como saber se o download depende da sessão. */
  allowCookies: boolean;
}

export interface Settings {
  /** Usar o Lyra Downloads para os downloads do Firefox (desligado por padrão). */
  autoCapture: boolean;
  /** "all": qualquer site; "listed": só os domínios em `rules`. */
  scope: "all" | "listed";
  rules: DomainRule[];
  /** Qualquer tipo de arquivo, ou só os de `extensions`. */
  anyFileType: boolean;
  extensions: string[];
  /** No escopo "all": capturar mesmo quando o navegador enviou cookies. */
  allowCookiesAll: boolean;
  /** Trazer a janela do Lyra Downloads para a frente ao receber um download. */
  showApp: boolean;
}

export const DEFAULT_EXTENSIONS = [
  "iso", "img", "zip", "7z", "rar", "tar", "tar.gz", "tgz", "tar.xz", "tar.zst",
  "gz", "xz", "zst", "bz2", "rpm", "deb", "appimage", "flatpak", "dmg", "exe", "msi",
];

export const DEFAULT_SETTINGS: Settings = {
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
export function requiredOrigins(s: Settings): string[] {
  return s.scope === "all" ? ALL_SITES_ORIGINS : hostPatterns(s.rules);
}

export function normalizeDomain(input: string): string | null {
  const d = input.trim().toLowerCase().replace(/^\*\./, "").replace(/\.$/, "");
  if (!/^[a-z0-9-]+(\.[a-z0-9-]+)+$/.test(d)) return null;
  return d;
}

export function normalizeExtension(input: string): string | null {
  const e = input.trim().toLowerCase().replace(/^\.+/, "");
  return /^[a-z0-9]+(\.[a-z0-9]+)?$/.test(e) ? e : null;
}

/** O host é o domínio da regra ou um subdomínio dele. */
export function hostMatches(host: string, domain: string): boolean {
  const h = host.toLowerCase();
  return h === domain || h.endsWith("." + domain);
}

export function findRule(host: string, rules: DomainRule[]): DomainRule | undefined {
  return rules.find((r) => hostMatches(host, r.domain));
}

/** Nome-base do caminho final escolhido pelo Firefox (já considera
 * Content-Disposition e tipo MIME, não só o sufixo da URL). */
export function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? "";
}

export function extensionMatches(filename: string, extensions: string[]): boolean {
  const name = basename(filename).toLowerCase();
  return extensions.some((ext) => name.endsWith("." + ext));
}

/** Padrões de URL para as permissões de host e filtros do webRequest. */
export function hostPatterns(rules: DomainRule[]): string[] {
  return rules.flatMap((r) => [`*://${r.domain}/*`, `*://*.${r.domain}/*`]);
}

export interface RequestRecord {
  url: string;
  method: string;
  hasCookie: boolean;
  hasAuthorization: boolean;
  incognito: boolean;
  time: number;
}

export interface DownloadInfo {
  id: number;
  url: string;
  filename: string;
  incognito: boolean;
  state: string;
  byExtensionId?: string;
}

export type Decision =
  | { capture: true; rule: DomainRule; url: string }
  | { capture: false; reason: string };

const skip = (reason: string): Decision => ({ capture: false, reason });

/** Decide se um download pode ser repassado. Qualquer dúvida → fica no Firefox. */
export function decide(
  item: DownloadInfo,
  settings: Settings,
  record: RequestRecord | undefined,
  ownExtensionId: string,
): Decision {
  if (!settings.autoCapture) return skip("captura automática desativada");
  if (item.byExtensionId === ownExtensionId) return skip("download iniciado pela própria extensão");
  if (item.incognito) return skip("navegação privada não participa da captura");
  if (item.state === "complete") return skip("download já terminou no Firefox");
  let url: URL;
  try {
    url = new URL(item.url);
  } catch {
    return skip("endereço inválido");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return skip("apenas http/https (blob:, data: e outros ficam no Firefox)");
  if (url.username || url.password) return skip("endereço com credenciais embutidas");
  const rule: DomainRule | undefined =
    settings.scope === "all"
      ? { domain: url.hostname, allowCookies: settings.allowCookiesAll }
      : findRule(url.hostname, settings.rules);
  if (!rule) return skip("domínio fora das regras");
  if (!settings.anyFileType && !extensionMatches(item.filename, settings.extensions)) {
    return skip("tipo de arquivo fora das regras");
  }
  if (!record) return skip("sem informação suficiente sobre a requisição");
  if (record.incognito) return skip("navegação privada não participa da captura");
  if (record.method !== "GET") return skip(`requisição ${record.method} não é reproduzível`);
  if (record.hasAuthorization) return skip("requisição com autenticação especial");
  if (record.hasCookie && !rule.allowCookies) return skip("requisição com cookies (pode depender da sessão)");
  return { capture: true, rule, url: item.url };
}
