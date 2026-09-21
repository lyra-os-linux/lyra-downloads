//! Cliente síncrono do backend (usado pela interface em uma thread de
//! trabalho e pelo native host).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;

use crate::{Op, Request, Response, MAX_MESSAGE_BYTES, PROTOCOL_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("backend indisponível: {0}")]
    Unavailable(String),
    #[error("erro de comunicação com o backend: {0}")]
    Io(#[from] std::io::Error),
    #[error("resposta inválida do backend: {0}")]
    Protocol(String),
    #[error("{message}")]
    Backend {
        code: crate::ErrorCode,
        message: String,
    },
}

pub struct BackendClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    counter: u64,
}

impl BackendClient {
    pub fn connect(socket: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        let writer = stream.try_clone()?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
            counter: 0,
        })
    }

    fn connect_ready(socket: &Path) -> Result<Self, ClientError> {
        let mut client = Self::connect(socket)?;
        // A dying process can still accept a connection. Only a read-only
        // reply confirms readiness; never retry the caller's mutating request.
        client.call::<serde_json::Value>(Op::Health)?;
        Ok(client)
    }

    /// Conecta ao backend; se não responder, inicia-o desanexado
    /// (nova sessão via `setsid`) e aguarda uma resposta de saúde.
    pub fn connect_or_spawn() -> Result<Self, ClientError> {
        let socket = lyra_downloads_core::paths::backend_socket_path()?;
        if let Ok(c) = Self::connect_ready(&socket) {
            return Ok(c);
        }
        spawn_backend_detached()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match Self::connect_ready(&socket) {
                Ok(c) => return Ok(c),
                Err(e) if Instant::now() > deadline => {
                    return Err(ClientError::Unavailable(format!(
                        "o backend não respondeu após ser iniciado ({e})"
                    )))
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    pub fn request_raw(&mut self, request_id: &str, op: Op) -> Result<Response, ClientError> {
        let req = Request {
            v: PROTOCOL_VERSION,
            request_id: request_id.to_string(),
            op,
        };
        let mut line =
            serde_json::to_vec(&req).map_err(|e| ClientError::Protocol(e.to_string()))?;
        line.push(b'\n');
        self.writer.write_all(&line)?;

        let mut buf = String::new();
        let n = (&mut self.reader)
            .take(MAX_MESSAGE_BYTES as u64 * 8)
            .read_line(&mut buf)?;
        if n == 0 {
            return Err(ClientError::Unavailable(
                "conexão encerrada pelo backend".into(),
            ));
        }
        let resp: Response =
            serde_json::from_str(&buf).map_err(|e| ClientError::Protocol(e.to_string()))?;
        if resp.request_id != request_id {
            return Err(ClientError::Protocol(
                "request_id divergente na resposta".into(),
            ));
        }
        Ok(resp)
    }

    pub fn call<T: DeserializeOwned>(&mut self, op: Op) -> Result<T, ClientError> {
        self.counter += 1;
        let id = format!("c{}-{}", std::process::id(), self.counter);
        let resp = self.request_raw(&id, op)?;
        if !resp.ok {
            let e = resp.error.unwrap_or(crate::ErrorBody {
                code: crate::ErrorCode::Internal,
                message: "erro desconhecido".into(),
            });
            return Err(ClientError::Backend {
                code: e.code,
                message: e.message,
            });
        }
        serde_json::from_value(resp.result.unwrap_or(serde_json::Value::Null))
            .map_err(|e| ClientError::Protocol(e.to_string()))
    }
}

/// Localiza um executável irmão (mesmo diretório do binário atual) ou no PATH.
pub fn find_sibling_executable(name: &str) -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Inicia um processo totalmente desanexado do chamador: nova sessão
/// (`setsid`), stdio em /dev/null. Assim, se o Firefox encerrar o grupo de
/// processos do native host, o processo iniciado sobrevive.
pub fn spawn_detached(program: &Path, args: &[&str]) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    // Recolhe o filho em segundo plano para não deixar zumbi enquanto o
    // chamador viver; o processo em si continua independente.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn spawn_backend_detached() -> Result<(), ClientError> {
    let exe = find_sibling_executable("lyra-downloads-backend").ok_or_else(|| {
        ClientError::Unavailable("executável lyra-downloads-backend não encontrado".into())
    })?;
    spawn_detached(&exe, &[])?;
    Ok(())
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn accepted_connection_without_a_reply_is_not_ready() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("backend.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            assert!(matches!(request.op, Op::Health));
            // Emulate a process exiting after accept, before responding.
        });
        assert!(BackendClient::connect_ready(&path).is_err());
        server.join().unwrap();
    }

    #[test]
    fn readiness_does_not_send_or_replay_the_callers_operation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("backend.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            assert!(matches!(request.op, Op::Health));
            let reply = Response::ok(&request.request_id, serde_json::Value::Null);
            writeln!(stream, "{}", serde_json::to_string(&reply).unwrap()).unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            assert!(matches!(request.op, Op::Shutdown));
            // Lost response: the operation must not be resent.
        });
        let mut client = BackendClient::connect_ready(&path).unwrap();
        assert!(client.call::<serde_json::Value>(Op::Shutdown).is_err());
        server.join().unwrap();
    }
}
