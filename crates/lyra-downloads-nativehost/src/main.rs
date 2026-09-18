use std::process::ExitCode;

use lyra_downloads_ipc::client::{
    find_sibling_executable, spawn_detached, BackendClient, ClientError,
};
use lyra_downloads_nativehost::{caller_allowed, error, run, write_frame, Effects};
use serde_json::Value;

struct Real;

impl Effects for Real {
    fn backend(&mut self, op: lyra_downloads_ipc::Op) -> Result<Value, (String, String)> {
        let mut client = BackendClient::connect_or_spawn()
            .map_err(|e| ("unavailable".to_string(), e.to_string()))?;
        client.call::<Value>(op).map_err(|e| match e {
            ClientError::Backend { code, message } => (
                serde_json::to_value(code)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                message,
            ),
            other => ("unavailable".to_string(), other.to_string()),
        })
    }

    fn open_app(&mut self, args: &[String]) -> Result<(), String> {
        let exe = find_sibling_executable("lyra-downloads")
            .ok_or_else(|| "O aplicativo Lyra Downloads não está instalado.".to_string())?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        spawn_detached(&exe, &args)
            .map_err(|e| format!("Não foi possível abrir o Lyra Downloads: {e}"))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();

    if !caller_allowed(&args) {
        eprintln!("lyra-downloads-nativehost: chamador não autorizado");
        let _ = write_frame(
            &mut output,
            &error(None, "not_allowed", "Extensão não autorizada."),
        );
        return ExitCode::FAILURE;
    }
    match run(&mut input, &mut output, &mut Real) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lyra-downloads-nativehost: {e}");
            ExitCode::FAILURE
        }
    }
}
