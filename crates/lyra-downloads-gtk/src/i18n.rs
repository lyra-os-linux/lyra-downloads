//! A interface web usa os mesmos msgids em inglês do domínio gettext
//! `lyra-downloads`; o catálogo do locale ativo vai inteiro para o JS.

use std::collections::HashMap;
use std::path::PathBuf;

pub use lyra_downloads_core::i18n::init;

/// Locale de mensagens efetivo entre os catálogos distribuídos (po/LINGUAS).
pub fn active_locale() -> &'static str {
    let language = std::env::var("LANGUAGE").unwrap_or_default();
    let mut candidates: Vec<String> = language.split(':').map(str::to_owned).collect();
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(name) {
            candidates.push(value);
        }
    }
    candidates
        .iter()
        .filter(|value| !value.trim().is_empty())
        .find_map(|value| supported(value))
        .unwrap_or("en_US")
}

fn supported(value: &str) -> Option<&'static str> {
    let base = value.trim().split(['.', '@']).next().unwrap_or("");
    match base
        .split(['_', '-'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pt" => Some("pt_BR"),
        "es" => Some("es_ES"),
        "en" => Some("en_US"),
        _ => None,
    }
}

#[tauri::command]
pub fn i18n_catalog() -> HashMap<String, String> {
    let dir = std::env::var("LYRA_DOWNLOADS_LOCALEDIR")
        .ok()
        .or_else(|| option_env!("LYRA_DOWNLOADS_LOCALEDIR").map(str::to_owned))
        .unwrap_or_else(|| "/usr/share/locale".to_owned());
    let path = PathBuf::from(dir)
        .join(active_locale())
        .join("LC_MESSAGES/lyra-downloads.mo");
    std::fs::read(path)
        .map(|bytes| parse_mo(&bytes))
        .unwrap_or_default()
}

/// Lê um catálogo GNU `.mo` (só a forma singular; a interface não usa plurais).
fn parse_mo(bytes: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let big = match bytes.get(0..4) {
        Some([0xde, 0x12, 0x04, 0x95]) => false,
        Some([0x95, 0x04, 0x12, 0xde]) => true,
        _ => return map,
    };
    let word = |offset: usize| -> Option<u32> {
        let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        })
    };
    let (Some(count), Some(originals), Some(translations)) = (word(8), word(12), word(16)) else {
        return map;
    };
    let string = |table: u32, index: u32| -> Option<String> {
        let entry = table as usize + index as usize * 8;
        let length = word(entry)? as usize;
        let offset = word(entry + 4)? as usize;
        let raw = bytes.get(offset..offset + length)?;
        String::from_utf8(raw.split(|&b| b == 0).next()?.to_vec()).ok()
    };
    for index in 0..count {
        if let (Some(key), Some(value)) = (string(originals, index), string(translations, index)) {
            if !key.is_empty() && !value.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_environment_locales_to_shipped_catalogs() {
        assert_eq!(supported("pt_BR.UTF-8"), Some("pt_BR"));
        assert_eq!(supported("pt-PT"), Some("pt_BR"));
        assert_eq!(supported("es_MX.utf8@euro"), Some("es_ES"));
        assert_eq!(supported("en_GB"), Some("en_US"));
        assert_eq!(supported("C.UTF-8"), None);
        assert_eq!(supported("de_DE"), None);
    }

    #[test]
    fn parses_a_compiled_catalog() {
        let dir = std::env::temp_dir().join(format!("lyra-dl-mo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let po = dir.join("t.po");
        let mo = dir.join("t.mo");
        std::fs::write(&po, "msgid \"\"\nmsgstr \"Content-Type: text/plain; charset=UTF-8\\n\"\n\nmsgid \"Pause\"\nmsgstr \"Pausar\"\n").unwrap();
        let ok = std::process::Command::new("msgfmt")
            .arg("-o")
            .arg(&mo)
            .arg(&po)
            .status();
        if !ok.is_ok_and(|s| s.success()) {
            return; // msgfmt ausente: o teste de parsing não se aplica.
        }
        let map = parse_mo(&std::fs::read(&mo).unwrap());
        assert_eq!(map.get("Pause").map(String::as_str), Some("Pausar"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
