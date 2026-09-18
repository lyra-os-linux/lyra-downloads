//! Sanitização de nomes de arquivo e resolução segura de destino.
//!
//! Regras: sem separadores de caminho, sem componentes `.`/`..`, sem bytes
//! de controle, sem nomes vazios após limpeza. A resolução de destino nunca
//! escapa do diretório escolhido pelo usuário (proteção contra path
//! traversal) e nunca sobrescreve um arquivo existente silenciosamente.

use std::path::{Component, Path, PathBuf};

const FALLBACK_NAME: &str = "download";
const MAX_NAME_LEN: usize = 200;

/// Limpa um nome de arquivo candidato (vindo da URL ou de `Content-Disposition`).
pub fn sanitize_filename(raw: &str) -> String {
    // Considera só o último componente, ignorando qualquer caminho embutido.
    let candidate = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();

    let mut cleaned: String = candidate
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();

    // Remove espaços/pontos nas pontas (problemáticos em alguns sistemas de
    // arquivo e usados para truques de nome tipo "..").
    cleaned = cleaned
        .trim_matches(|c: char| c == ' ' || c == '.')
        .to_string();

    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        cleaned = FALLBACK_NAME.to_string();
    }

    if cleaned.len() > MAX_NAME_LEN {
        cleaned = cleaned.chars().take(MAX_NAME_LEN).collect();
    }

    cleaned
}

/// Extrai um nome de arquivo sugerido a partir de um cabeçalho
/// `Content-Disposition`, se presente e utilizável.
pub fn filename_from_content_disposition(header: &str) -> Option<String> {
    // Aceita as formas `filename="x.zip"` e `filename*=UTF-8''x.zip`.
    for part in header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("filename*=") {
            let value = value.trim();
            if let Some(rest) = value
                .to_ascii_uppercase()
                .starts_with("UTF-8''")
                .then(|| &value[7..])
            {
                if let Ok(decoded) = percent_decode(rest) {
                    return Some(sanitize_filename(&decoded));
                }
            }
        } else if let Some(value) = part.strip_prefix("filename=") {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(sanitize_filename(value));
            }
        }
    }
    None
}

fn percent_decode(input: &str) -> Result<String, ()> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).map_err(|_| ())?;
            let byte = u8::from_str_radix(hex, 16).map_err(|_| ())?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

/// Deriva um nome de arquivo a partir do último segmento de path de uma URL,
/// já sanitizado. Usado quando não há `Content-Disposition` utilizável.
pub fn filename_from_url(url: &url::Url) -> String {
    let last_segment = url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|s| !s.is_empty()));

    match last_segment {
        Some(segment) => {
            let decoded = percent_decode(segment).unwrap_or_else(|_| segment.to_string());
            sanitize_filename(&decoded)
        }
        None => FALLBACK_NAME.to_string(),
    }
}

/// Verifica se `dir` é um diretório absoluto e não contém componentes `..`
/// (proteção básica contra path traversal vindo de preferências corrompidas
/// ou entrada externa).
pub fn is_safe_destination_dir(dir: &Path) -> bool {
    dir.is_absolute() && !dir.components().any(|c| matches!(c, Component::ParentDir))
}

/// Resolve um caminho final de destino dentro de `dir` para `filename`,
/// evitando sobrescrever um arquivo já existente: se `nome.ext` existir,
/// tenta `nome (1).ext`, `nome (2).ext`, etc. Garante que o resultado
/// permanece dentro de `dir`.
pub fn resolve_non_colliding_path(dir: &Path, filename: &str) -> PathBuf {
    resolve_non_colliding_path_with(dir, filename, &std::collections::HashSet::new())
}

/// Como [`resolve_non_colliding_path`], mas também considera ocupados os
/// caminhos em `reserved` (destinos de outras tarefas ainda não concluídas)
/// e nomes com arquivo de controle `.aria2` remanescente.
pub fn resolve_non_colliding_path_with(
    dir: &Path,
    filename: &str,
    reserved: &std::collections::HashSet<PathBuf>,
) -> PathBuf {
    let taken = |p: &Path| {
        p.exists() || reserved.contains(p) || {
            let mut ctl = p.as_os_str().to_owned();
            ctl.push(".aria2");
            Path::new(&ctl).exists()
        }
    };
    let safe_name = sanitize_filename(filename);
    let path = Path::new(&safe_name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| FALLBACK_NAME.to_string());
    let ext = path.extension().map(|e| e.to_string_lossy().to_string());

    let candidate = dir.join(&safe_name);
    if !taken(&candidate) {
        return candidate;
    }

    for n in 1u32..10_000 {
        let name = match &ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = dir.join(&name);
        if !taken(&candidate) {
            return candidate;
        }
    }

    // Extremamente improvável (10 mil colisões); usa um sufixo aleatório
    // baseado em um novo UUID para garantir progresso.
    let name = match &ext {
        Some(ext) => format!("{stem}-{}.{ext}", uuid::Uuid::new_v4()),
        None => format!("{stem}-{}", uuid::Uuid::new_v4()),
    };
    dir.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_path_separators() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a/b\\c"), "c");
    }

    #[test]
    fn falls_back_on_empty() {
        assert_eq!(sanitize_filename(""), FALLBACK_NAME);
        assert_eq!(sanitize_filename("..."), FALLBACK_NAME);
    }

    #[test]
    fn content_disposition_basic() {
        assert_eq!(
            filename_from_content_disposition("attachment; filename=\"foo.iso\""),
            Some("foo.iso".to_string())
        );
    }

    #[test]
    fn content_disposition_utf8_star() {
        assert_eq!(
            filename_from_content_disposition(
                "attachment; filename*=UTF-8''arquivo%20com%20espa%C3%A7o.iso"
            ),
            Some("arquivo com espaço.iso".to_string())
        );
    }

    #[test]
    fn collision_avoidance() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        let resolved = resolve_non_colliding_path(dir.path(), "a.txt");
        assert_eq!(resolved.file_name().unwrap().to_str().unwrap(), "a (1).txt");
    }

    #[test]
    fn collision_considers_reserved_and_control_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.iso.aria2"), b"x").unwrap();
        let mut reserved = std::collections::HashSet::new();
        reserved.insert(dir.path().join("b (1).iso"));
        let r = resolve_non_colliding_path_with(dir.path(), "b.iso", &reserved);
        assert_eq!(r.file_name().unwrap().to_str().unwrap(), "b (2).iso");
    }

    #[test]
    fn rejects_relative_and_traversal_dirs() {
        assert!(!is_safe_destination_dir(Path::new("relative")));
        assert!(!is_safe_destination_dir(Path::new("/home/user/../etc")));
        assert!(is_safe_destination_dir(Path::new("/home/user/Downloads")));
    }
}
