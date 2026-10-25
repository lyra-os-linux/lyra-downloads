//! Chamadas ao serviço de downloads. Cada chamada abre uma conexão curta ao
//! socket local numa thread de bloqueio; a interface recebe JSON pronto.

use lyra_downloads_core::i18n::{tr, trf};
use lyra_downloads_ipc::client::{BackendClient, ClientError};
use lyra_downloads_ipc::Op;
use serde_json::Value;

/// Executa uma operação do protocolo (o JSON já validado como `Op`).
#[tauri::command]
pub async fn backend_call(op: Value) -> Result<Value, String> {
    let op: Op = serde_json::from_value(op).map_err(|error| {
        trf(
            "Invalid response from the download service: {detail}",
            &[("detail", &error.to_string())],
        )
    })?;
    call(op).await
}

pub async fn call(op: Op) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut client = BackendClient::connect_or_spawn()?;
        client.call::<Value>(op)
    })
    .await
    .map_err(|_| tr("Internal error while contacting the download service."))?
    .map_err(describe)
}

fn describe(error: ClientError) -> String {
    match error {
        ClientError::Unavailable(detail) => trf(
            "Download service unavailable: {detail}",
            &[("detail", &detail)],
        ),
        ClientError::Io(detail) => trf(
            "Could not communicate with the download service: {detail}",
            &[("detail", &detail.to_string())],
        ),
        ClientError::Protocol(detail) => trf(
            "Invalid response from the download service: {detail}",
            &[("detail", &detail)],
        ),
        ClientError::Backend { message, .. } => message,
    }
}

/// "Pausar tudo e encerrar": só conecta a um serviço já em execução (nunca
/// inicia um para encerrá-lo) e fecha o aplicativo em seguida.
#[tauri::command]
pub async fn quit_service(app: tauri::AppHandle) -> Result<(), String> {
    let _ = tauri::async_runtime::spawn_blocking(|| -> Result<(), ClientError> {
        let socket = lyra_downloads_core::paths::backend_socket_path()
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        BackendClient::connect(&socket)?.call::<Value>(Op::Shutdown)?;
        Ok(())
    })
    .await;
    app.exit(0);
    Ok(())
}
