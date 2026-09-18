use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::paths;
use crate::task::ConnectionProfile;

/// Preferências do usuário, persistidas na tabela `settings` do banco.
/// Os valores aqui são os padrões documentados na seção 3 do escopo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub default_destination_dir: PathBuf,
    pub default_connections: ConnectionProfile,
    /// Limite de downloads simultâneos (independente do número de conexões
    /// por download). Padrão inicial: 2.
    pub max_concurrent_downloads: u32,
    /// Limite de velocidade global em bytes/s. `0` = sem limite.
    pub global_speed_limit_bytes: u64,
    /// Retomar automaticamente tarefas pausadas pelo sistema (não pelo
    /// usuário) após reinício do backend/motor.
    pub auto_resume_on_start: bool,
    pub notifications_enabled: bool,
    /// Captura automática de downloads pela extensão do Firefox — desativada
    /// por padrão (ver seção 6 do escopo).
    pub browser_auto_capture_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_destination_dir: paths::default_downloads_dir(),
            default_connections: ConnectionProfile::DEFAULT,
            max_concurrent_downloads: 2,
            global_speed_limit_bytes: 0,
            auto_resume_on_start: true,
            notifications_enabled: true,
            browser_auto_capture_enabled: false,
        }
    }
}
