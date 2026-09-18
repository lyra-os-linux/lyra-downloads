//! Protocolo IPC local entre o backend e seus clientes (interface GTK e
//! native host do Firefox). Transporte: socket Unix em
//! `$XDG_RUNTIME_DIR/lyra-downloads/backend.sock` (diretório 0700, socket
//! 0600), uma mensagem JSON por linha, uma resposta por requisição.

pub mod client;

use std::path::PathBuf;

use lyra_downloads_core::settings::Settings;
use lyra_downloads_core::Task;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Interface,
    Navegador,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddDownload {
    pub url: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub destination_dir: Option<PathBuf>,
    #[serde(default)]
    pub connections: Option<u8>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    pub source: Source,
    /// Chave de idempotência: reenvios com a mesma chave retornam a mesma
    /// tarefa em vez de criar outra. Downloads intencionais posteriores da
    /// mesma URL usam chaves diferentes e não são bloqueados.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Nome sugerido pelo navegador/servidor (Content-Disposition), usado
    /// quando `filename` não é informado.
    #[serde(default)]
    pub suggested_filename: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveDirection {
    Up,
    Down,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Health,
    ListTasks,
    /// Consulta leve (HEAD) para sugerir nome/tamanho; não cria tarefa.
    Probe {
        url: String,
    },
    AddDownload(AddDownload),
    Pause {
        id: Uuid,
    },
    Resume {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
    Retry {
        id: Uuid,
    },
    /// Cria uma nova tarefa com um novo link a partir de uma tarefa com erro
    /// (não reaproveita parciais de uma URL diferente).
    RetryWithNewUrl {
        id: Uuid,
        url: String,
    },
    RemoveFromHistory {
        id: Uuid,
    },
    /// Exclui o arquivo (e parciais/.aria2) do disco. Ação explícita.
    DeleteFile {
        id: Uuid,
    },
    Move {
        id: Uuid,
        direction: MoveDirection,
    },
    ChangeConnections {
        id: Uuid,
        connections: u8,
    },
    /// Cancela apenas a tarefa criada com esta chave de idempotência (repasse
    /// do navegador abandonado por timeout). Não afeta nenhuma outra tarefa.
    CancelByRequestKey {
        key: String,
    },
    PauseAll,
    ResumeAll,
    GetSettings,
    /// Tenta iniciar o motor de novo após falhas (zera o limitador de reinícios).
    RestartEngine,
    SetSettings {
        settings: Settings,
    },
    /// Pausa tudo, salva a sessão e encerra backend e aria2.
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub request_id: String,
    #[serde(flatten)]
    pub op: Op,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    UnsupportedVersion,
    NotFound,
    InvalidUrl,
    InvalidDestination,
    InvalidState,
    EngineUnavailable,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub request_id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Response {
    pub fn ok(request_id: &str, result: impl Serialize) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            request_id: request_id.to_string(),
            ok: true,
            result: Some(serde_json::to_value(result).unwrap_or(serde_json::Value::Null)),
            error: None,
        }
    }

    pub fn err(request_id: &str, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            request_id: request_id.to_string(),
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub backend_version: String,
    pub protocol: u32,
    pub engine_running: bool,
    pub engine_version: Option<String>,
    /// Diagnóstico legível quando o motor não está disponível.
    pub engine_problem: Option<String>,
}

/// Tarefa com dados operacionais ao vivo (não persistidos).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub task: Task,
    pub download_speed: u64,
    pub active_connections: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub tasks: Vec<TaskView>,
    pub total_speed: u64,
    pub engine_running: bool,
    pub engine_problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddResult {
    pub task_id: Uuid,
    /// `true` se a chave de idempotência já existia (nenhuma tarefa nova criada).
    pub duplicate: bool,
    pub filename: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip_flattened() {
        let r = Request {
            v: 1,
            request_id: "abc".into(),
            op: Op::Pause { id: Uuid::nil() },
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"op\":\"pause\""));
        let back: Request = serde_json::from_str(&s).unwrap();
        assert!(matches!(back.op, Op::Pause { .. }));
    }

    #[test]
    fn unknown_op_is_rejected() {
        let s = r#"{"v":1,"request_id":"x","op":"aria2_raw","method":"aria2.shutdown"}"#;
        assert!(serde_json::from_str::<Request>(s).is_err());
    }
}
