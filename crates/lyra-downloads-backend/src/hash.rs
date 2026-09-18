use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Calcula o SHA-256 de um arquivo em streaming (blocos de 1 MiB).
/// Deve ser chamado fora de threads de interface/async (spawn_blocking).
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Normaliza um SHA-256 informado pelo usuário: aceita maiúsculas/minúsculas
/// e espaços nas pontas; rejeita tudo que não for 64 dígitos hex.
pub fn normalize_expected(input: &str) -> Option<String> {
    let s = input.trim().to_ascii_lowercase();
    (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("abc");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn normalize() {
        let h = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        assert!(normalize_expected(h).is_some());
        assert!(normalize_expected("abc").is_none());
        assert!(normalize_expected(&"z".repeat(64)).is_none());
    }
}
