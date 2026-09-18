use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Perfil de conexões paralelas por tarefa, mapeado 1:1 para
/// `max-connection-per-server` e `split` do aria2 (o valor pedido é um
/// máximo aceito pelo servidor de origem, nunca uma garantia).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionProfile {
    One = 1,
    Four = 4,
    Eight = 8,
    Sixteen = 16,
}

impl ConnectionProfile {
    pub const DEFAULT: ConnectionProfile = ConnectionProfile::Four;

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::One),
            4 => Some(Self::Four),
            8 => Some(Self::Eight),
            16 => Some(Self::Sixteen),
            _ => None,
        }
    }
}

impl Default for ConnectionProfile {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TaskState {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Aguardando => "aguardando",
            Self::Baixando => "baixando",
            Self::Pausado => "pausado",
            Self::Verificando => "verificando",
            Self::Concluido => "concluido",
            Self::Cancelado => "cancelado",
            Self::Erro => "erro",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        Some(match s {
            "aguardando" => Self::Aguardando,
            "baixando" => Self::Baixando,
            "pausado" => Self::Pausado,
            "verificando" => Self::Verificando,
            "concluido" => Self::Concluido,
            "cancelado" => Self::Cancelado,
            "erro" => Self::Erro,
            _ => return None,
        })
    }
}

impl HashVerification {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::NaoVerificado => "nao_verificado",
            Self::Confere => "confere",
            Self::NaoConfere => "nao_confere",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        Some(match s {
            "nao_verificado" => Self::NaoVerificado,
            "confere" => Self::Confere,
            "nao_confere" => Self::NaoConfere,
            _ => return None,
        })
    }
}

impl PauseReason {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Usuario => "usuario",
            Self::Sistema => "sistema",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        Some(match s {
            "usuario" => Self::Usuario,
            "sistema" => Self::Sistema,
            _ => return None,
        })
    }
}

/// Estados possíveis de uma tarefa. Nomeados em português para corresponder
/// diretamente ao vocabulário exigido na interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Aguardando na fila (ainda não enviada ao aria2, ou limite de
    /// downloads simultâneos atingido).
    Aguardando,
    Baixando,
    Pausado,
    /// Download concluído, verificação de SHA-256 em andamento.
    Verificando,
    Concluido,
    Cancelado,
    Erro,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Concluido | Self::Cancelado | Self::Erro)
    }
}

/// Resultado da verificação de SHA-256 esperado (quando fornecido pelo
/// usuário). Nunca implica autenticidade por si só — apenas que os bytes
/// baixados correspondem ao hash informado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashVerification {
    /// Nenhum hash esperado foi informado, ou a verificação foi pulada.
    #[default]
    NaoVerificado,
    Confere,
    NaoConfere,
}

/// Por que uma tarefa está pausada — necessário para respeitar a intenção
/// do usuário na retomada automática após reinício do backend/motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    /// O usuário pausou explicitamente; não deve ser retomada automaticamente.
    Usuario,
    /// Pausada pelo sistema (ex.: limite de downloads simultâneos, motor
    /// reiniciado); pode ser retomada automaticamente conforme preferência.
    Sistema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    /// GID atual no aria2, se a tarefa já foi submetida ao motor.
    pub aria2_gid: Option<String>,
    pub url: String,
    pub filename: String,
    pub destination_dir: PathBuf,
    pub connections: ConnectionProfile,
    pub expected_sha256: Option<String>,
    pub state: TaskState,
    pub pause_reason: Option<PauseReason>,
    pub hash_verification: HashVerification,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub error_message: Option<String>,
    /// Posição relativa entre as tarefas aguardando (menor = mais próxima do início).
    pub queue_position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn final_path(&self) -> PathBuf {
        self.destination_dir.join(&self.filename)
    }

    pub fn progress_fraction(&self) -> Option<f64> {
        self.total_bytes.and_then(|total| {
            if total == 0 {
                None
            } else {
                Some(self.downloaded_bytes as f64 / total as f64)
            }
        })
    }
}

/// Dados necessários para criar uma nova tarefa (antes de ganhar um `id`
/// próprio e ser persistida).
#[derive(Debug, Clone)]
pub struct NewTask {
    pub url: String,
    pub filename: String,
    pub destination_dir: PathBuf,
    pub connections: ConnectionProfile,
    pub expected_sha256: Option<String>,
}
