//! Diretórios XDG usados pelo Lyra Downloads.
//!
//! - `config`: preferências do usuário (não usado para segredos).
//! - `data`: banco SQLite e sessão persistente do aria2 (`XDG_DATA_HOME`).
//! - `runtime`: socket Unix do backend, arquivo de lock de instância única e
//!   segredo RPC do aria2 (`XDG_RUNTIME_DIR`, com permissões 0700).

use std::fs;
use std::io;
use std::path::PathBuf;

const APP_DIR: &str = "lyra-downloads";

fn ensure_dir(path: &PathBuf) -> io::Result<()> {
    fs::create_dir_all(path)
}

/// `$XDG_CONFIG_HOME/lyra-downloads`
pub fn config_dir() -> io::Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_CONFIG_HOME indisponível"))?;
    let dir = base.join(APP_DIR);
    ensure_dir(&dir)?;
    Ok(dir)
}

/// `$XDG_DATA_HOME/lyra-downloads`
pub fn data_dir() -> io::Result<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_DATA_HOME indisponível"))?;
    let dir = base.join(APP_DIR);
    ensure_dir(&dir)?;
    Ok(dir)
}

/// `$XDG_RUNTIME_DIR/lyra-downloads` (cai para `$XDG_DATA_HOME/lyra-downloads/run`
/// se `XDG_RUNTIME_DIR` não estiver definido — não deveria acontecer em uma
/// sessão de usuário normal, mas evita pânico em ambientes de teste/CI).
pub fn runtime_dir() -> io::Result<PathBuf> {
    let dir = match dirs::runtime_dir() {
        Some(base) => base.join(APP_DIR),
        None => data_dir()?.join("run"),
    };
    ensure_dir(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

/// Caminho do banco SQLite principal.
pub fn database_path() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("lyra-downloads.db"))
}

/// Diretório de sessão do aria2 (arquivo `session.aria2` + configuração dedicada).
pub fn aria2_session_dir() -> io::Result<PathBuf> {
    let dir = data_dir()?.join("aria2");
    ensure_dir(&dir)?;
    Ok(dir)
}

/// Caminho do socket Unix do backend.
pub fn backend_socket_path() -> io::Result<PathBuf> {
    Ok(runtime_dir()?.join("backend.sock"))
}

/// Caminho do arquivo de lock de instância única do backend.
pub fn backend_lock_path() -> io::Result<PathBuf> {
    Ok(runtime_dir()?.join("backend.lock"))
}

/// Caminho do arquivo com o segredo RPC do aria2 desta instância (permissões 0600).
pub fn aria2_rpc_secret_path() -> io::Result<PathBuf> {
    Ok(runtime_dir()?.join("aria2-rpc-secret"))
}

/// Pasta padrão de downloads do usuário (`XDG_DOWNLOAD_DIR`), com fallback
/// para `$HOME/Downloads` e depois para `$HOME` se nada estiver configurado.
pub fn default_downloads_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(|| PathBuf::from("."))
}
