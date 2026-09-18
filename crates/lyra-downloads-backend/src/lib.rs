pub mod hash;
pub mod notify;
pub mod probe;
pub mod server;
pub mod service;

use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use lyra_downloads_core::db::Repository;
use tokio::sync::{watch, Mutex};

/// Lock exclusivo de instância única (flock). Mantido aberto enquanto o
/// backend vive; o kernel libera se o processo morrer.
pub struct InstanceLock {
    _file: File,
}

pub fn acquire_instance_lock(path: &Path) -> std::io::Result<Option<InstanceLock>> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path)?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Ok(None);
        }
        return Err(e);
    }
    Ok(Some(InstanceLock { _file: file }))
}

pub struct Paths {
    pub database: std::path::PathBuf,
    pub session_dir: std::path::PathBuf,
    pub socket: std::path::PathBuf,
    pub lock: std::path::PathBuf,
}

impl Paths {
    pub fn from_xdg() -> std::io::Result<Self> {
        use lyra_downloads_core::paths;
        Ok(Self {
            database: paths::database_path()?,
            session_dir: paths::aria2_session_dir()?,
            socket: paths::backend_socket_path()?,
            lock: paths::backend_lock_path()?,
        })
    }
}

/// Executa o backend até receber `Shutdown` (ou SIGTERM/SIGINT).
/// Retorna `Ok(false)` se outra instância já estava ativa.
pub async fn run(paths: Paths, notifications: bool) -> anyhow_like::Result<bool> {
    let Some(_lock) = acquire_instance_lock(&paths.lock)? else {
        tracing::info!("outra instância do backend já está ativa");
        return Ok(false);
    };
    let repo = Arc::new(Repository::open(&paths.database)?);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let notifier = if notifications {
        notify::Notifier::new().await
    } else {
        notify::Notifier::disabled()
    };
    let svc = service::Service::new(repo, paths.session_dir.clone(), notifier, shutdown_tx);
    let shared: service::Shared = Arc::new(Mutex::new(svc));

    // Primeiro ciclo antes de aceitar clientes: motor iniciado e reconciliado.
    service::tick(&shared).await;

    let listener = server::bind(&paths.socket)?;
    let server_task = tokio::spawn(server::serve(listener, shared.clone()));

    let ticker_shared = shared.clone();
    let ticker = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            service::tick(&ticker_shared).await;
        }
    });

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    tokio::select! {
        _ = shutdown_rx.changed() => {}
        _ = sigterm.recv() => { shared.lock().await.shutdown().await; }
        _ = sigint.recv() => { shared.lock().await.shutdown().await; }
    }

    ticker.abort();
    server_task.abort();
    let _ = std::fs::remove_file(&paths.socket);
    tracing::info!("backend encerrado");
    Ok(true)
}

/// Erro genérico simples para o ponto de entrada.
pub mod anyhow_like {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}
