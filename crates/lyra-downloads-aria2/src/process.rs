//! Supervisão do processo `aria2c` exclusivo do Lyra Downloads.
//!
//! Nunca reutiliza nem encerra instâncias de aria2 de terceiros: esta
//! instância tem porta, segredo, sessão e log próprios, e só o PID que ela
//! mesma iniciou é sinalizado.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use rand::RngCore;
use tokio::process::{Child, Command};

use crate::client::Aria2Client;

/// Min split size padrão (documentado): 1 MiB. O padrão do aria2 é 20 MiB,
/// o que impede arquivos médios de usarem várias conexões.
pub const DEFAULT_MIN_SPLIT_SIZE: &str = "1M";

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error(
        "aria2c não foi encontrado. Instale o pacote \"aria2\" (no openSUSE: sudo zypper install aria2) \
         ou garanta que o executável esteja no PATH."
    )]
    NotFound,
    #[error("falha ao iniciar aria2c: {0}")]
    Io(#[from] io::Error),
    #[error("aria2c iniciou mas não respondeu ao RPC a tempo")]
    NotReady,
}

pub fn find_aria2c() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LYRA_DOWNLOADS_ARIA2C") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("aria2c"))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub fn pick_free_loopback_port() -> io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

pub fn generate_secret() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub struct SpawnConfig {
    pub session_dir: PathBuf,
    pub default_download_dir: PathBuf,
    pub max_concurrent_downloads: u32,
    pub global_speed_limit_bytes: u64,
}

pub struct Aria2Process {
    child: Child,
    pub port: u16,
    pub client: Aria2Client,
    pub version: String,
}

impl Aria2Process {
    pub async fn spawn(cfg: &SpawnConfig) -> Result<Self, SpawnError> {
        let binary = find_aria2c().ok_or(SpawnError::NotFound)?;
        let pid_file = cfg.session_dir.join("aria2.pid");
        reap_stale_instance(&pid_file, &cfg.session_dir.join("aria2.conf")).await;
        let secret = generate_secret();
        let session_file = cfg.session_dir.join("session.aria2");
        let conf_file = cfg.session_dir.join("aria2.conf");
        let log_file = cfg.session_dir.join("aria2.log");

        // O segredo vai num arquivo de configuração privado (0600), não na
        // linha de comando, para não aparecer em `ps`.
        write_private(&conf_file, &format!("rpc-secret={secret}\n"))?;
        if !session_file.exists() {
            write_private(&session_file, "")?;
        }

        // Tentamos algumas portas: entre escolher a porta livre e o aria2
        // fazer bind há uma pequena janela de corrida.
        let mut last_err = SpawnError::NotReady;
        for _ in 0..3 {
            let port = pick_free_loopback_port()?;
            let mut cmd = Command::new(&binary);
            cmd.arg(format!("--conf-path={}", conf_file.display()))
                .arg("--enable-rpc=true")
                .arg("--rpc-listen-all=false")
                .arg("--rpc-allow-origin-all=false")
                .arg(format!("--rpc-listen-port={port}"))
                .arg(format!("--dir={}", cfg.default_download_dir.display()))
                .arg(format!("--input-file={}", session_file.display()))
                .arg(format!("--save-session={}", session_file.display()))
                .arg("--save-session-interval=30")
                .arg("--force-save=false")
                // Tudo que for restaurado da sessão começa pausado; o backend
                // decide o que retomar (preferência + intenção do usuário).
                .arg("--pause=true")
                .arg("--continue=true")
                .arg("--allow-overwrite=false")
                .arg("--auto-file-renaming=false")
                .arg("--check-certificate=true")
                .arg("--max-tries=5")
                .arg("--retry-wait=5")
                .arg("--connect-timeout=30")
                .arg("--timeout=60")
                .arg(format!("--min-split-size={DEFAULT_MIN_SPLIT_SIZE}"))
                .arg(format!(
                    "--max-concurrent-downloads={}",
                    cfg.max_concurrent_downloads.max(1)
                ))
                .arg(format!(
                    "--max-overall-download-limit={}",
                    cfg.global_speed_limit_bytes
                ))
                .arg(format!("--log={}", log_file.display()))
                .arg("--log-level=notice")
                .arg("--console-log-level=error")
                .arg("--summary-interval=0")
                .arg("--show-console-readout=false")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(false);

            let mut child = cmd.spawn()?;
            let client = Aria2Client::new(port, &secret);
            match wait_ready(&client, &mut child).await {
                Ok(version) => {
                    tracing::info!(port, version, pid = child.id(), "aria2c pronto");
                    if let Some(pid) = child.id() {
                        let _ = write_private(&pid_file, &pid.to_string());
                    }
                    return Ok(Self {
                        child,
                        port,
                        client,
                        version,
                    });
                }
                Err(e) => {
                    tracing::warn!("aria2c não ficou pronto na porta {port}: {e}");
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Retorna `true` se o processo ainda está vivo.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Salva a sessão, pede shutdown gracioso e espera; força só o próprio PID.
    pub async fn shutdown(mut self) {
        let _ = self.client.save_session().await;
        let _ = self.client.shutdown().await;
        match tokio::time::timeout(Duration::from_secs(10), self.child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                tracing::warn!("aria2c não encerrou em 10s; enviando SIGTERM ao próprio PID");
                if let Some(pid) = self.child.id() {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
            }
        }
    }
}

/// Se um backend anterior caiu, o aria2c dele pode continuar vivo usando a
/// mesma sessão. Só encerra o PID registrado se `/proc/<pid>` confirmar que
/// é um `aria2c` iniciado com o nosso `--conf-path` privado — nunca
/// instâncias de aria2 de terceiros.
async fn reap_stale_instance(pid_file: &Path, conf_file: &Path) {
    let Ok(raw) = std::fs::read_to_string(pid_file) else {
        return;
    };
    let Ok(pid) = raw.trim().parse::<i32>() else {
        return;
    };
    if !is_our_aria2(pid, conf_file) {
        let _ = std::fs::remove_file(pid_file);
        return;
    }
    tracing::warn!(
        pid,
        "aria2c remanescente de uma execução anterior; encerrando"
    );
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    for _ in 0..50 {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = std::fs::remove_file(pid_file);
}

fn is_our_aria2(pid: i32, conf_file: &Path) -> bool {
    let exe_ok = std::fs::read_link(format!("/proc/{pid}/exe"))
        .map(|p| p.file_name().is_some_and(|n| n == "aria2c"))
        .unwrap_or(false);
    let expected = format!("--conf-path={}", conf_file.display());
    let cmd_ok = std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|c| c.split(|b| *b == 0).any(|arg| arg == expected.as_bytes()))
        .unwrap_or(false);
    exe_ok && cmd_ok
}

async fn wait_ready(client: &Aria2Client, child: &mut Child) -> Result<String, SpawnError> {
    for _ in 0..50 {
        if let Ok(Some(_)) = child.try_wait() {
            return Err(SpawnError::NotReady);
        }
        if let Ok(v) = client.get_version().await {
            return Ok(v);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(SpawnError::NotReady)
}

fn write_private(path: &Path, content: &str) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(content.as_bytes())?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}
