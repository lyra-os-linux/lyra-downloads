//! Servidor IPC em socket Unix. Aceita só conexões do mesmo UID
//! (SO_PEERCRED), uma requisição JSON por linha.

use std::path::Path;

use lyra_downloads_ipc::{ErrorCode, Request, Response, MAX_MESSAGE_BYTES, PROTOCOL_VERSION};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::service::{self, Shared};

pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    // Seguro remover: o chamador já detém o lock de instância única.
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub async fn serve(listener: UnixListener, shared: Shared) {
    let my_uid = unsafe { libc::getuid() };
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("accept: {e}");
                continue;
            }
        };
        match stream.peer_cred() {
            Ok(cred) if cred.uid() == my_uid => {}
            _ => {
                tracing::warn!("conexão recusada: usuário diferente");
                continue;
            }
        }
        let shared = shared.clone();
        tokio::spawn(async move {
            if let Err(e) = connection(stream, shared).await {
                tracing::debug!("conexão encerrada: {e}");
            }
        });
    }
}

async fn connection(stream: UnixStream, shared: Shared) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    loop {
        let mut line = String::new();
        let n = (&mut reader)
            .take(MAX_MESSAGE_BYTES as u64 + 1)
            .read_line(&mut line)
            .await?;
        if n == 0 {
            return Ok(());
        }
        if n > MAX_MESSAGE_BYTES {
            let resp = Response::err("", ErrorCode::InvalidRequest, "Mensagem grande demais.");
            send(&mut write, &resp).await?;
            return Ok(());
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Err(e) => Response::err(
                "",
                ErrorCode::InvalidRequest,
                format!("Requisição inválida: {e}"),
            ),
            Ok(req) if req.v != PROTOCOL_VERSION => Response::err(
                &req.request_id,
                ErrorCode::UnsupportedVersion,
                format!("Versão de protocolo não suportada: {}", req.v),
            ),
            Ok(req) => {
                let id = req.request_id.clone();
                match service::handle(&shared, req.op).await {
                    Ok(v) => Response::ok(&id, v),
                    Err((code, msg)) => Response::err(&id, code, msg),
                }
            }
        };
        send(&mut write, &resp).await?;
    }
}

async fn send(w: &mut tokio::net::unix::OwnedWriteHalf, resp: &Response) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(resp).unwrap_or_default();
    bytes.push(b'\n');
    w.write_all(&bytes).await
}
