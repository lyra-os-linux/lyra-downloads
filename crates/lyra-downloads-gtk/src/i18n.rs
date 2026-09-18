//! gettext. As mensagens-fonte estão em português do Brasil (idioma
//! inicial); traduções para outros idiomas entram como catálogos em `po/`.

use gettextrs::{bind_textdomain_codeset, bindtextdomain, setlocale, textdomain, LocaleCategory};

const DOMAIN: &str = "lyra-downloads";
const DEFAULT_LOCALE_DIR: &str = "/usr/share/locale";

pub fn tr(message: &str) -> String {
    gettextrs::gettext(message)
}

/// Tradução com placeholders nomeados: `trf("{n} itens", &[("n", "3")])`.
pub fn trf(message: &str, values: &[(&str, &str)]) -> String {
    let mut s = tr(message);
    for (k, v) in values {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

pub fn init() {
    let _ = setlocale(LocaleCategory::LcAll, "");
    let dir = std::env::var("LYRA_DOWNLOADS_LOCALEDIR")
        .ok()
        .or_else(|| option_env!("LYRA_DOWNLOADS_LOCALEDIR").map(str::to_string))
        .unwrap_or_else(|| DEFAULT_LOCALE_DIR.to_string());
    let _ = bindtextdomain(DOMAIN, dir);
    let _ = bind_textdomain_codeset(DOMAIN, "UTF-8");
    let _ = textdomain(DOMAIN);
}
