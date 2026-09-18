use std::process::ExitCode;

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter =
        EnvFilter::try_from_env("LYRA_DOWNLOADS_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    // Quando iniciado desanexado, stderr vai para /dev/null; o log fica em
    // $XDG_DATA_HOME/lyra-downloads/backend.log (URLs sempre redigidas).
    let log_path = lyra_downloads_core::paths::data_dir().map(|d| d.join("backend.log"));
    let file = log_path.ok().and_then(|p| {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(p)
            .ok()
    });
    match file {
        Some(f) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(f))
            .init(),
        None => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init(),
    }
}

fn main() -> ExitCode {
    init_logging();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("lyra-downloads-backend: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = rt.block_on(async {
        let paths = lyra_downloads_backend::Paths::from_xdg()?;
        lyra_downloads_backend::run(paths, true).await
    });
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("backend falhou: {e}");
            eprintln!("lyra-downloads-backend: {e}");
            ExitCode::FAILURE
        }
    }
}
