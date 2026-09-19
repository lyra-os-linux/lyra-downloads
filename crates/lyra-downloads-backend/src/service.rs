//! Dono único da fila e do motor.
//!
//! Regras de autoridade (ver docs/ARCHITECTURE.md):
//! - Banco: identidade, intenção (pausado pelo usuário, conexões, destino)
//!   e histórico. É a única fonte que decide quais tarefas existem.
//! - aria2: estado operacional (ativo/aguardando/pausado/erro/completo,
//!   bytes, velocidade, conexões). Lido a cada ~1 s.
//! - Sessão do aria2 + arquivos `.aria2`: dados binários de retomada.
//!
//! Uma tarefa nunca é inserida duas vezes: o motor só recebe tarefas que já
//! existem no banco, com GID determinístico derivado do `Task::id`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use lyra_downloads_aria2::{self as aria2, Aria2Options, Aria2Process, Aria2Status, SpawnConfig};
use lyra_downloads_core::db::Repository;
use lyra_downloads_core::settings::Settings;
use lyra_downloads_core::{
    self as core, sanitize, ConnectionProfile, HashVerification, NewTask, PauseReason, Task,
    TaskState,
};
use lyra_downloads_ipc::{
    AddDownload, AddResult, ErrorCode, Health, MoveDirection, Op, Snapshot, TaskView,
};
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex};
use uuid::Uuid;

use crate::hash;
use crate::notify::Notifier;
use crate::probe;

pub type Shared = Arc<Mutex<Service>>;
pub type OpResult = Result<Value, (ErrorCode, String)>;

const MAX_ENGINE_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(600);
const PERSIST_PROGRESS_EVERY: Duration = Duration::from_secs(5);

#[derive(Default, Clone, Copy)]
struct Live {
    speed: u64,
    connections: u64,
}

pub struct Service {
    repo: Arc<Repository>,
    engine: Option<Aria2Process>,
    engine_problem: Option<String>,
    engine_failures: u32,
    restarts: VecDeque<Instant>,
    /// Após (re)iniciar o motor, aplica a política de retomada uma vez.
    needs_reconcile: bool,
    live: HashMap<Uuid, Live>,
    last_persist: HashMap<Uuid, Instant>,
    settings: Settings,
    session_dir: PathBuf,
    notifier: Notifier,
    shutdown_tx: watch::Sender<bool>,
    shutting_down: bool,
}

fn err<T>(code: ErrorCode, msg: impl Into<String>) -> Result<T, (ErrorCode, String)> {
    Err((code, msg.into()))
}

fn internal(e: impl std::fmt::Display) -> (ErrorCode, String) {
    (ErrorCode::Internal, e.to_string())
}

impl Service {
    pub fn new(
        repo: Arc<Repository>,
        session_dir: PathBuf,
        notifier: Notifier,
        shutdown_tx: watch::Sender<bool>,
    ) -> Self {
        let settings = repo.load_settings().unwrap_or_default();
        Self {
            repo,
            engine: None,
            engine_problem: None,
            engine_failures: 0,
            restarts: VecDeque::new(),
            needs_reconcile: false,
            live: HashMap::new(),
            last_persist: HashMap::new(),
            settings,
            session_dir,
            notifier,
            shutdown_tx,
            shutting_down: false,
        }
    }

    fn engine_client(&self) -> Option<aria2::Aria2Client> {
        self.engine.as_ref().map(|e| e.client.clone())
    }

    /// Inicia o motor respeitando o limite de reinícios (evita loops).
    pub async fn ensure_engine(&mut self) {
        if self.shutting_down {
            return;
        }
        if let Some(engine) = self.engine.as_mut() {
            if engine.is_alive() {
                return;
            }
            tracing::warn!("aria2c encerrou inesperadamente");
            self.engine = None;
        }
        let now = Instant::now();
        while self
            .restarts
            .front()
            .is_some_and(|t| now.duration_since(*t) > RESTART_WINDOW)
        {
            self.restarts.pop_front();
        }
        if self.restarts.len() >= MAX_ENGINE_RESTARTS {
            if self.engine_problem.is_none() {
                self.engine_problem = Some(
                    "O motor aria2 falhou várias vezes seguidas. Os downloads estão parados; \
                     use \"Tentar novamente\" ou reinicie o Lyra Downloads."
                        .into(),
                );
            }
            return;
        }
        self.restarts.push_back(now);

        let cfg = SpawnConfig {
            session_dir: self.session_dir.clone(),
            default_download_dir: self.settings.default_destination_dir.clone(),
            max_concurrent_downloads: self.settings.max_concurrent_downloads,
            global_speed_limit_bytes: self.settings.global_speed_limit_bytes,
        };
        match Aria2Process::spawn(&cfg).await {
            Ok(p) => {
                self.engine = Some(p);
                self.engine_problem = None;
                self.engine_failures = 0;
                self.needs_reconcile = true;
            }
            Err(e) => {
                tracing::error!("não foi possível iniciar o aria2c: {e}");
                self.engine_problem = Some(e.to_string());
            }
        }
    }

    /// Permite nova tentativa manual de iniciar o motor (zera o limitador).
    fn reset_engine_backoff(&mut self) {
        if self.engine.is_none() {
            self.restarts.clear();
            self.engine_problem = None;
        }
    }

    fn reserved_paths(&self) -> HashSet<PathBuf> {
        self.repo
            .list_tasks()
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.final_path())
            .collect()
    }

    fn save(&mut self, task: &mut Task) -> Result<(), (ErrorCode, String)> {
        task.updated_at = Utc::now();
        self.last_persist.insert(task.id, Instant::now());
        self.repo.update_task(task).map_err(internal)
    }

    fn get(&self, id: Uuid) -> Result<Task, (ErrorCode, String)> {
        match self.repo.get_task(id).map_err(internal)? {
            Some(t) => Ok(t),
            None => err(ErrorCode::NotFound, "Tarefa não encontrada."),
        }
    }

    /// Envia uma tarefa (já persistida) ao motor. Sem motor, a tarefa
    /// permanece no banco e será enviada quando o motor estiver disponível.
    async fn submit(&mut self, task: &mut Task, paused: bool) -> Result<(), (ErrorCode, String)> {
        let Some(client) = self.engine_client() else {
            return Ok(());
        };
        let mut opts: Aria2Options = aria2::connection_options(task.connections.as_u8());
        opts.insert(
            "dir".into(),
            task.destination_dir.to_string_lossy().to_string(),
        );
        opts.insert("out".into(), task.filename.clone());
        opts.insert("pause".into(), paused.to_string());

        let gid = core::deterministic_gid(task.id);
        // Libera um resultado antigo com o mesmo GID (tentar novamente).
        let _ = client.remove_download_result(&gid).await;
        let mut with_gid = opts.clone();
        with_gid.insert("gid".into(), gid);

        let assigned = match client.add_uri(&task.url, &with_gid).await {
            Ok(g) => g,
            Err(aria2::Aria2Error::Rpc { message, .. }) => {
                // GID em uso dentro do processo atual (o aria2 responde com
                // "No URI to download."): deixa o aria2 escolher outro e
                // atualiza o mapeamento.
                tracing::info!(
                    "aria2 recusou GID determinístico ({message}); usando GID atribuído"
                );
                client.add_uri(&task.url, &opts).await.map_err(|e| {
                    (
                        ErrorCode::EngineUnavailable,
                        format!("O motor recusou o download: {e}"),
                    )
                })?
            }
            Err(e) => {
                return err(
                    ErrorCode::EngineUnavailable,
                    format!("Motor indisponível: {e}"),
                );
            }
        };
        task.aria2_gid = Some(assigned);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Operações do protocolo
    // ------------------------------------------------------------------

    pub async fn add(&mut self, req: AddDownload) -> OpResult {
        if let Some(key) = req.idempotency_key.as_deref() {
            if self.repo.request_key_revoked(key).map_err(internal)? {
                return err(
                    ErrorCode::InvalidState,
                    "Este repasse foi cancelado pelo navegador.",
                );
            }
            if let Some(existing) = self.repo.task_for_request_key(key).map_err(internal)? {
                let t = self.get(existing)?;
                return Ok(json!(AddResult {
                    task_id: t.id,
                    duplicate: true,
                    filename: t.filename
                }));
            }
        }

        let url = match core::validate_download_url(req.url.trim()) {
            Ok(u) => u,
            Err(_) => {
                return err(
                    ErrorCode::InvalidUrl,
                    "Informe um endereço http:// ou https:// válido.",
                )
            }
        };
        if url.host_str().is_none() {
            return err(
                ErrorCode::InvalidUrl,
                "O endereço não tem um servidor válido.",
            );
        }

        let dir = req
            .destination_dir
            .clone()
            .unwrap_or_else(|| self.settings.default_destination_dir.clone());
        check_destination(&dir)?;

        let connections = match req.connections {
            None => self.settings.default_connections,
            Some(n) => match ConnectionProfile::from_u8(n) {
                Some(p) => p,
                None => {
                    return err(
                        ErrorCode::InvalidRequest,
                        "Número de conexões inválido (use 1, 4, 8 ou 16).",
                    )
                }
            },
        };

        let expected_sha256 = match req.expected_sha256.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(h) => match hash::normalize_expected(h) {
                Some(h) => Some(h),
                None => {
                    return err(
                        ErrorCode::InvalidRequest,
                        "O SHA-256 esperado deve ter 64 dígitos hexadecimais.",
                    )
                }
            },
        };

        let filename = req
            .filename
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(sanitize::sanitize_filename)
            .or_else(|| {
                req.suggested_filename
                    .as_deref()
                    .map(sanitize::sanitize_filename)
            })
            .unwrap_or_else(|| sanitize::filename_from_url(&url));

        let mut reserved = self.reserved_paths();
        if req.reserve_browser_filename {
            // Firefox pause() can remove its placeholder. A subsequent
            // cancel() still unlinks that path after the native handoff.
            // Reserve it even while absent, before starting aria2, so the
            // browser's cleanup cannot unlink our open/completed file.
            reserved.insert(dir.join(&filename));
        }
        let new = NewTask {
            url: url.as_str().to_string(),
            filename,
            destination_dir: dir,
            connections,
            expected_sha256,
        };
        let mut task = core::build_task(new, &reserved).map_err(|e| match e {
            core::CoreError::InvalidDestination(_) => (
                ErrorCode::InvalidDestination,
                "Pasta de destino inválida.".to_string(),
            ),
            other => internal(other),
        })?;
        task.queue_position = self.repo.next_queue_position().map_err(internal)?;

        // Aceite durável: a tarefa (e a chave de idempotência) é gravada
        // antes de qualquer interação com o motor.
        self.repo
            .insert_task_with_key(&task, req.idempotency_key.as_deref())
            .map_err(internal)?;
        tracing::info!(id = %task.id, url = %core::redact_url(&task.url), "tarefa criada");

        if let Err((_, msg)) = self.submit(&mut task, false).await {
            tracing::warn!("tarefa {} aguardando motor: {msg}", task.id);
        }
        self.save(&mut task)?;
        Ok(json!(AddResult {
            task_id: task.id,
            duplicate: false,
            filename: task.filename
        }))
    }

    pub async fn pause(&mut self, id: Uuid) -> OpResult {
        let mut t = self.get(id)?;
        match t.state {
            TaskState::Aguardando | TaskState::Baixando => {}
            TaskState::Pausado => {
                t.pause_reason = Some(PauseReason::Usuario);
                self.save(&mut t)?;
                return Ok(Value::Null);
            }
            _ => {
                return err(
                    ErrorCode::InvalidState,
                    "Esta tarefa não pode ser pausada agora.",
                )
            }
        }
        if let (Some(client), Some(gid)) = (self.engine_client(), t.aria2_gid.clone()) {
            if let Err(e) = client.pause(&gid).await {
                tracing::debug!("pause {gid}: {e}");
            }
        }
        t.state = TaskState::Pausado;
        t.pause_reason = Some(PauseReason::Usuario);
        self.live.remove(&id);
        self.save(&mut t)?;
        Ok(Value::Null)
    }

    pub async fn resume(&mut self, id: Uuid) -> OpResult {
        let mut t = self.get(id)?;
        if t.state != TaskState::Pausado {
            return err(
                ErrorCode::InvalidState,
                "Apenas downloads pausados podem ser retomados.",
            );
        }
        self.reset_engine_backoff();
        self.ensure_engine().await;
        self.resume_in_engine(&mut t).await?;
        t.state = TaskState::Aguardando;
        t.pause_reason = None;
        self.save(&mut t)?;
        Ok(Value::Null)
    }

    async fn resume_in_engine(&mut self, t: &mut Task) -> Result<(), (ErrorCode, String)> {
        let Some(client) = self.engine_client() else {
            return Ok(());
        };
        if let Some(gid) = t.aria2_gid.clone() {
            match client.tell_status(&gid).await {
                Ok(s) if s.status == "paused" => {
                    return client.unpause(&gid).await.map_err(|e| {
                        (
                            ErrorCode::EngineUnavailable,
                            format!("Não foi possível retomar: {e}"),
                        )
                    });
                }
                Ok(s) if s.status == "active" || s.status == "waiting" => return Ok(()),
                _ => {}
            }
        }
        self.submit(t, false).await
    }

    pub async fn cancel(&mut self, id: Uuid) -> OpResult {
        let mut t = self.get(id)?;
        if t.state.is_terminal() {
            return err(ErrorCode::InvalidState, "Esta tarefa já terminou.");
        }
        if let (Some(client), Some(gid)) = (self.engine_client(), t.aria2_gid.clone()) {
            let _ = client.force_remove(&gid).await;
        }
        t.state = TaskState::Cancelado;
        t.pause_reason = None;
        self.live.remove(&id);
        self.save(&mut t)?;
        Ok(Value::Null)
    }

    pub async fn cancel_by_request_key(&mut self, key: &str) -> OpResult {
        // O lock do serviço serializa add/cancel. Gravar antes de responder
        // impede que uma tentativa atrasada crie a tarefa após o cancelamento.
        self.repo.revoke_request_key(key).map_err(internal)?;
        let Some(id) = self.repo.task_for_request_key(key).map_err(internal)? else {
            return Ok(
                json!({ "found": false, "cancelled": false, "revoked": true, "completed": false }),
            );
        };
        let Some(mut t) = self.repo.get_task(id).map_err(internal)? else {
            return Ok(
                json!({ "found": false, "cancelled": false, "revoked": true, "completed": false }),
            );
        };
        if matches!(t.state, TaskState::Concluido | TaskState::Verificando) {
            return Ok(
                json!({ "found": true, "cancelled": false, "revoked": true, "completed": true }),
            );
        }
        if let Some(gid) = t.aria2_gid.as_deref() {
            let Some(client) = self.engine_client() else {
                return err(
                    ErrorCode::EngineUnavailable,
                    "Não foi possível confirmar o cancelamento no motor.",
                );
            };
            // forceRemove pode falhar se a tarefa já terminou ou desapareceu.
            // Falhas de comunicação nunca são convertidas em confirmação.
            let _ = client.force_remove(gid).await;
            // Consulta um GID específico: listas separadas de ativos/em fila
            // não são atômicas e podem perder uma tarefa que mudou de fila.
            match client.tell_status(gid).await {
                Ok(status) if status.status == "complete" => {
                    return Ok(
                        json!({ "found": true, "cancelled": false, "revoked": true, "completed": true }),
                    );
                }
                Ok(status) if matches!(status.status.as_str(), "removed" | "error") => {}
                Err(aria2::Aria2Error::Rpc { code: 1, message })
                    if message == format!("GID {gid} is not found") => {}
                _ => {
                    return err(
                        ErrorCode::EngineUnavailable,
                        "O motor ainda não confirmou o cancelamento.",
                    )
                }
            }
        }
        t.state = TaskState::Cancelado;
        t.pause_reason = None;
        self.live.remove(&id);
        t.error_message = Some(
            "O navegador não recebeu a confirmação a tempo; o download continuou no Firefox."
                .into(),
        );
        self.save(&mut t)?;
        Ok(json!({ "found": true, "cancelled": true, "revoked": true, "completed": false }))
    }

    pub async fn retry(&mut self, id: Uuid) -> OpResult {
        let mut t = self.get(id)?;
        if !matches!(t.state, TaskState::Erro | TaskState::Cancelado) {
            return err(
                ErrorCode::InvalidState,
                "Só é possível tentar novamente downloads com erro ou cancelados.",
            );
        }
        check_destination(&t.destination_dir)?;
        self.reset_engine_backoff();
        self.ensure_engine().await;
        t.error_message = None;
        t.state = TaskState::Aguardando;
        t.pause_reason = None;
        t.hash_verification = HashVerification::NaoVerificado;
        self.submit(&mut t, false).await?;
        self.save(&mut t)?;
        Ok(Value::Null)
    }

    /// Nova tarefa com outro link. Recebe um nome novo (sem colisão), então
    /// nunca reaproveita parciais baixados de uma URL diferente.
    pub async fn retry_with_new_url(&mut self, id: Uuid, url: String) -> OpResult {
        let old = self.get(id)?;
        let req = AddDownload {
            url,
            filename: Some(old.filename.clone()),
            destination_dir: Some(old.destination_dir.clone()),
            connections: Some(old.connections.as_u8()),
            expected_sha256: old.expected_sha256.clone(),
            source: lyra_downloads_ipc::Source::Interface,
            idempotency_key: None,
            suggested_filename: None,
            reserve_browser_filename: false,
        };
        self.add(req).await
    }

    pub async fn remove_from_history(&mut self, id: Uuid) -> OpResult {
        let t = self.get(id)?;
        if !t.state.is_terminal() {
            return err(
                ErrorCode::InvalidState,
                "Cancele o download antes de removê-lo da lista.",
            );
        }
        if let (Some(client), Some(gid)) = (self.engine_client(), t.aria2_gid.clone()) {
            let _ = client.remove_download_result(&gid).await;
        }
        self.repo.delete_task(id).map_err(internal)?;
        self.live.remove(&id);
        Ok(Value::Null)
    }

    pub async fn delete_file(&mut self, id: Uuid) -> OpResult {
        let t = self.get(id)?;
        if !t.state.is_terminal() {
            return err(
                ErrorCode::InvalidState,
                "Cancele o download antes de excluir o arquivo.",
            );
        }
        let name = sanitize::sanitize_filename(&t.filename);
        if name != t.filename || !sanitize::is_safe_destination_dir(&t.destination_dir) {
            return err(
                ErrorCode::InvalidState,
                "Caminho do arquivo inválido; nada foi excluído.",
            );
        }
        let path = t.destination_dir.join(&name);
        let mut control = path.as_os_str().to_owned();
        control.push(".aria2");
        for p in [path.clone(), PathBuf::from(control)] {
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_file() || m.file_type().is_symlink() => {
                    std::fs::remove_file(&p).map_err(|e| {
                        (
                            ErrorCode::Internal,
                            format!("Não foi possível excluir {}: {e}", p.display()),
                        )
                    })?;
                }
                Ok(_) => {
                    return err(
                        ErrorCode::InvalidState,
                        "O destino não é um arquivo comum; nada foi excluído.",
                    )
                }
                Err(_) => {}
            }
        }
        if let (Some(client), Some(gid)) = (self.engine_client(), t.aria2_gid.clone()) {
            let _ = client.remove_download_result(&gid).await;
        }
        self.repo.delete_task(id).map_err(internal)?;
        Ok(Value::Null)
    }

    pub async fn move_task(&mut self, id: Uuid, dir: MoveDirection) -> OpResult {
        let mut waiting: Vec<Task> = self
            .repo
            .list_tasks()
            .map_err(internal)?
            .into_iter()
            .filter(|t| t.state == TaskState::Aguardando)
            .collect();
        let Some(idx) = waiting.iter().position(|t| t.id == id) else {
            return err(
                ErrorCode::InvalidState,
                "Só é possível reordenar downloads aguardando.",
            );
        };
        let item = waiting.remove(idx);
        let new_idx = match dir {
            MoveDirection::Up => idx.saturating_sub(1),
            MoveDirection::Down => (idx + 1).min(waiting.len()),
            MoveDirection::Top => 0,
            MoveDirection::Bottom => waiting.len(),
        };
        waiting.insert(new_idx, item);
        let ids: Vec<Uuid> = waiting.iter().map(|t| t.id).collect();
        self.repo.reorder_waiting(&ids).map_err(internal)?;
        self.mirror_queue_order(&waiting).await;
        Ok(Value::Null)
    }

    async fn mirror_queue_order(&self, ordered: &[Task]) {
        let Some(client) = self.engine_client() else {
            return;
        };
        let Ok(engine_waiting) = client.tell_waiting().await else {
            return;
        };
        let in_engine: HashSet<String> = engine_waiting
            .into_iter()
            .filter(|s| s.status == "waiting")
            .map(|s| s.gid)
            .collect();
        let mut pos = 0i64;
        for t in ordered {
            if let Some(gid) = t.aria2_gid.as_deref().filter(|g| in_engine.contains(*g)) {
                let _ = client.change_position(gid, pos, "POS_SET").await;
                pos += 1;
            }
        }
    }

    pub async fn change_connections(&mut self, id: Uuid, n: u8) -> OpResult {
        let Some(profile) = ConnectionProfile::from_u8(n) else {
            return err(
                ErrorCode::InvalidRequest,
                "Número de conexões inválido (use 1, 4, 8 ou 16).",
            );
        };
        let mut t = self.get(id)?;
        if t.state.is_terminal() || t.state == TaskState::Verificando {
            return err(
                ErrorCode::InvalidState,
                "Não é possível alterar conexões de uma tarefa encerrada.",
            );
        }
        t.connections = profile;
        let mut restarted = false;
        if let (Some(client), Some(gid)) = (self.engine_client(), t.aria2_gid.clone()) {
            let opts = aria2::connection_options(profile.as_u8());
            let status = client.tell_status(&gid).await.ok().map(|s| s.status);
            if status.as_deref() == Some("active") {
                // Alterar conexões de uma transferência ativa exige pausá-la:
                // ela recomeça do ponto parcial (arquivo .aria2 preservado).
                let _ = client.force_pause(&gid).await;
                for _ in 0..50 {
                    if client
                        .tell_status(&gid)
                        .await
                        .is_ok_and(|s| s.status == "paused")
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                client.change_option(&gid, &opts).await.map_err(|e| {
                    (
                        ErrorCode::EngineUnavailable,
                        format!("O motor não aceitou a alteração: {e}"),
                    )
                })?;
                let _ = client.unpause(&gid).await;
                restarted = true;
            } else if status.is_some() {
                client.change_option(&gid, &opts).await.map_err(|e| {
                    (
                        ErrorCode::EngineUnavailable,
                        format!("O motor não aceitou a alteração: {e}"),
                    )
                })?;
            }
        }
        self.save(&mut t)?;
        Ok(json!({ "restarted": restarted }))
    }

    pub async fn pause_all(&mut self, reason: PauseReason) -> OpResult {
        if let Some(client) = self.engine_client() {
            let _ = client.pause_all().await;
        }
        for mut t in self.repo.list_tasks().map_err(internal)? {
            if matches!(t.state, TaskState::Aguardando | TaskState::Baixando) {
                t.state = TaskState::Pausado;
                t.pause_reason = Some(reason);
                self.live.remove(&t.id);
                self.save(&mut t)?;
            }
        }
        Ok(Value::Null)
    }

    pub async fn resume_all(&mut self) -> OpResult {
        self.reset_engine_backoff();
        self.ensure_engine().await;
        for mut t in self.repo.list_tasks().map_err(internal)? {
            if t.state == TaskState::Pausado {
                if let Err((_, m)) = self.resume_in_engine(&mut t).await {
                    tracing::warn!("retomar {}: {m}", t.id);
                    continue;
                }
                t.state = TaskState::Aguardando;
                t.pause_reason = None;
                self.save(&mut t)?;
            }
        }
        Ok(Value::Null)
    }

    pub async fn set_settings(&mut self, s: Settings) -> OpResult {
        if !(1..=10).contains(&s.max_concurrent_downloads) {
            return err(
                ErrorCode::InvalidRequest,
                "Downloads simultâneos deve estar entre 1 e 10.",
            );
        }
        check_destination(&s.default_destination_dir)?;
        self.repo.save_settings(&s).map_err(internal)?;
        if let Some(client) = self.engine_client() {
            let mut g = Aria2Options::new();
            g.insert(
                "max-concurrent-downloads".into(),
                s.max_concurrent_downloads.to_string(),
            );
            g.insert(
                "max-overall-download-limit".into(),
                s.global_speed_limit_bytes.to_string(),
            );
            if let Err(e) = client.change_global_option(&g).await {
                tracing::warn!("changeGlobalOption: {e}");
            }
        }
        self.settings = s;
        Ok(Value::Null)
    }

    pub fn health(&self) -> Health {
        Health {
            backend_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: lyra_downloads_ipc::PROTOCOL_VERSION,
            engine_running: self.engine.is_some(),
            engine_version: self.engine.as_ref().map(|e| e.version.clone()),
            engine_problem: self.engine_problem.clone(),
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot, (ErrorCode, String)> {
        let tasks: Vec<TaskView> = self
            .repo
            .list_tasks()
            .map_err(internal)?
            .into_iter()
            .map(|task| {
                let live = self.live.get(&task.id).copied().unwrap_or_default();
                TaskView {
                    task,
                    download_speed: live.speed,
                    active_connections: live.connections,
                }
            })
            .collect();
        let total_speed = tasks.iter().map(|t| t.download_speed).sum();
        Ok(Snapshot {
            tasks,
            total_speed,
            engine_running: self.engine.is_some(),
            engine_problem: self.engine_problem.clone(),
        })
    }

    /// Pausa tudo (motivo: sistema, para retomar conforme preferência),
    /// salva a sessão, encerra o aria2 e sinaliza o encerramento do backend.
    pub async fn shutdown(&mut self) {
        if self.shutting_down {
            return;
        }
        let _ = self.pause_all(PauseReason::Sistema).await;
        self.shutting_down = true;
        if let Some(engine) = self.engine.take() {
            engine.shutdown().await;
        }
        let _ = self.shutdown_tx.send(true);
    }

    // ------------------------------------------------------------------
    // Sincronização periódica com o motor
    // ------------------------------------------------------------------

    async fn engine_statuses(&mut self) -> Option<HashMap<String, Aria2Status>> {
        let client = self.engine_client()?;
        let result = async {
            let mut all = client.tell_active().await?;
            all.extend(client.tell_waiting().await?);
            all.extend(client.tell_stopped().await?);
            Ok::<_, aria2::Aria2Error>(all)
        }
        .await;
        match result {
            Ok(list) => {
                self.engine_failures = 0;
                Some(list.into_iter().map(|s| (s.gid.clone(), s)).collect())
            }
            Err(e) => {
                self.engine_failures += 1;
                tracing::warn!("falha ao consultar aria2 ({}): {e}", self.engine_failures);
                if self.engine_failures >= 3 {
                    if let Some(engine) = self.engine.take() {
                        engine.shutdown().await;
                    }
                    self.engine_problem =
                        Some("O motor aria2 parou de responder; reiniciando.".into());
                }
                None
            }
        }
    }
}

fn check_destination(dir: &Path) -> Result<(), (ErrorCode, String)> {
    if !sanitize::is_safe_destination_dir(dir) {
        return err(
            ErrorCode::InvalidDestination,
            "A pasta de destino precisa ser um caminho absoluto.",
        );
    }
    if !dir.is_dir() {
        return err(
            ErrorCode::InvalidDestination,
            format!("A pasta de destino não existe: {}", dir.display()),
        );
    }
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).map_err(|_| {
        (
            ErrorCode::InvalidDestination,
            "Caminho de destino inválido.".to_string(),
        )
    })?;
    if unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) } != 0 {
        return err(
            ErrorCode::InvalidDestination,
            format!("Sem permissão de escrita em {}", dir.display()),
        );
    }
    Ok(())
}

/// Complemento com o status HTTP real, quando o aria2 o informa. Descreve
/// causas possíveis sem afirmar uma delas (um 403 pode ter vários motivos).
fn http_status_hint(msg: &str) -> Option<String> {
    let idx = msg.find("status=")?;
    let code: String = msg[idx + 7..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let extra = match code.as_str() {
        "" => return None,
        "401" | "403" => " Pode ser um link expirado, um recurso que exige login ou um bloqueio para este acesso.",
        "404" | "410" => " O arquivo pode ter sido removido ou o link estar incorreto.",
        "429" => " O servidor pediu para diminuir o ritmo de requisições; tente mais tarde ou com menos conexões.",
        c if c.starts_with('5') => " Erro no servidor; costuma ser temporário.",
        _ => "",
    };
    Some(format!(" (HTTP {code}).{extra}"))
}

/// Um ciclo de sincronização: garante o motor, reconcilia após (re)início,
/// traduz estados do aria2 para o domínio e agenda verificações de hash.
pub async fn tick(shared: &Shared) {
    let mut svc = shared.lock().await;
    svc.ensure_engine().await;
    let Some(statuses) = svc.engine_statuses().await else {
        return;
    };

    let reconcile = std::mem::take(&mut svc.needs_reconcile);
    let auto_resume = svc.settings.auto_resume_on_start;
    let client = match svc.engine_client() {
        Some(c) => c,
        None => return,
    };
    let tasks = match svc.repo.list_tasks() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("falha ao ler tarefas: {e}");
            return;
        }
    };

    let known: HashSet<&str> = tasks
        .iter()
        .filter_map(|t| t.aria2_gid.as_deref())
        .collect();
    if reconcile {
        // Downloads na sessão do motor que não pertencem a nenhuma tarefa
        // (não deveria ocorrer): ficam pausados, nunca apagados.
        for (gid, s) in &statuses {
            if !known.contains(gid.as_str()) && (s.status == "active" || s.status == "waiting") {
                tracing::warn!("download desconhecido {gid} na sessão do motor; pausando");
                let _ = client.force_pause(gid).await;
            }
        }
    }

    let mut to_verify = Vec::new();
    let mut finished = Vec::new();

    for mut t in tasks {
        if t.state.is_terminal() || t.state == TaskState::Verificando {
            if t.state == TaskState::Verificando && reconcile {
                to_verify.push(t.clone()); // verificação interrompida: refaz
            }
            continue;
        }
        let status = t.aria2_gid.as_deref().and_then(|g| statuses.get(g));

        // Reconciliação: tarefa ativa no banco mas ausente no motor. Reenvia
        // com o mesmo GID/destino; o aria2 retoma pelo arquivo .aria2.
        let status = match status {
            Some(s) => s.clone(),
            None => {
                let should_run = match t.state {
                    TaskState::Pausado => {
                        auto_resume && reconcile && t.pause_reason == Some(PauseReason::Sistema)
                    }
                    _ => !reconcile || auto_resume,
                };
                if let Err((_, m)) = svc.submit(&mut t, !should_run).await {
                    tracing::warn!("reenvio de {} falhou: {m}", t.id);
                    continue;
                }
                if should_run {
                    t.state = TaskState::Aguardando;
                    t.pause_reason = None;
                } else if t.state != TaskState::Pausado {
                    t.state = TaskState::Pausado;
                    t.pause_reason = Some(PauseReason::Sistema);
                }
                let _ = svc.save(&mut t);
                continue;
            }
        };
        let gid = status.gid.clone();

        if reconcile {
            // Motor iniciado com --pause=true: tudo restaurado da sessão está
            // pausado; decide o que deve voltar a rodar.
            let wants_run = match t.state {
                TaskState::Pausado => auto_resume && t.pause_reason == Some(PauseReason::Sistema),
                _ => auto_resume,
            };
            if wants_run && status.status == "paused" {
                let _ = client.unpause(&gid).await;
                t.state = TaskState::Aguardando;
                t.pause_reason = None;
                let _ = svc.save(&mut t);
                continue;
            }
            if !wants_run && t.state != TaskState::Pausado {
                let _ = client.force_pause(&gid).await;
                t.state = TaskState::Pausado;
                t.pause_reason = Some(PauseReason::Sistema);
                let _ = svc.save(&mut t);
                continue;
            }
        }

        let prev_state = t.state;
        let prev_total = t.total_bytes;
        match status.status.as_str() {
            "active" => {
                t.state = TaskState::Baixando;
                t.pause_reason = None;
                t.total_bytes = (status.total_length > 0).then_some(status.total_length);
                t.downloaded_bytes = status.completed_length;
                svc.live.insert(
                    t.id,
                    Live {
                        speed: status.download_speed,
                        connections: status.connections,
                    },
                );
            }
            "waiting" => {
                t.state = TaskState::Aguardando;
                svc.live.remove(&t.id);
            }
            "paused" => {
                if t.state != TaskState::Pausado {
                    t.state = TaskState::Pausado;
                    t.pause_reason.get_or_insert(PauseReason::Sistema);
                }
                if status.total_length > 0 {
                    t.total_bytes = Some(status.total_length);
                }
                t.downloaded_bytes = status.completed_length;
                svc.live.remove(&t.id);
            }
            "complete" => {
                let len = status.completed_length.max(status.total_length);
                t.total_bytes = Some(len);
                t.downloaded_bytes = len;
                svc.live.remove(&t.id);
                let _ = client.remove_download_result(&gid).await;
                if t.expected_sha256.is_some() {
                    t.state = TaskState::Verificando;
                    to_verify.push(t.clone());
                } else {
                    t.state = TaskState::Concluido;
                    finished.push(t.clone());
                }
            }
            "error" => {
                let code = status.error_code_num().unwrap_or(1);
                let desc = aria2::errors::describe(code);
                let hint = status
                    .error_message
                    .as_deref()
                    .and_then(http_status_hint)
                    .unwrap_or_default();
                t.state = TaskState::Erro;
                let base = desc.message.trim_end_matches('.');
                t.error_message = Some(if hint.is_empty() {
                    desc.message.to_string()
                } else {
                    format!("{base}{hint}")
                });
                t.downloaded_bytes = status.completed_length;
                svc.live.remove(&t.id);
                tracing::warn!(id = %t.id, code, "download com erro");
            }
            "removed" => {
                t.state = TaskState::Cancelado;
                svc.live.remove(&t.id);
            }
            _ => {}
        }

        let changed = t.state != prev_state || t.total_bytes != prev_total;
        let stale = svc
            .last_persist
            .get(&t.id)
            .is_none_or(|i| i.elapsed() >= PERSIST_PROGRESS_EVERY);
        if changed || stale {
            let _ = svc.save(&mut t);
        }
    }

    let notifier = svc.notifier.clone();
    let notify = svc.settings.notifications_enabled;
    drop(svc);

    for t in finished {
        if notify {
            notifier
                .download_finished("Download concluído", &t.filename, &t.final_path())
                .await;
        }
    }
    for t in to_verify {
        spawn_verification(shared.clone(), t);
    }
}

fn spawn_verification(shared: Shared, task: Task) {
    tokio::spawn(async move {
        let path = task.final_path();
        let expected = task.expected_sha256.clone().unwrap_or_default();
        let result = tokio::task::spawn_blocking(move || hash::sha256_file(&path)).await;

        let mut svc = shared.lock().await;
        let Ok(Some(mut t)) = svc.repo.get_task(task.id) else {
            return;
        };
        if t.state != TaskState::Verificando {
            return;
        }
        let body;
        match result {
            Ok(Ok(actual)) if actual == expected => {
                t.hash_verification = HashVerification::Confere;
                t.state = TaskState::Concluido;
                body = format!("{} — SHA-256 confere com o valor informado", t.filename);
            }
            Ok(Ok(_)) => {
                t.hash_verification = HashVerification::NaoConfere;
                t.state = TaskState::Concluido;
                body = format!(
                    "{} — SHA-256 NÃO confere. O arquivo foi mantido; confira a origem do hash e do arquivo.",
                    t.filename
                );
            }
            Ok(Err(e)) => {
                t.hash_verification = HashVerification::NaoVerificado;
                t.state = TaskState::Erro;
                t.error_message = Some(format!(
                    "Não foi possível ler o arquivo para verificar o SHA-256: {e}"
                ));
                body = String::new();
            }
            Err(e) => {
                t.state = TaskState::Erro;
                t.error_message = Some(format!("Falha interna na verificação: {e}"));
                body = String::new();
            }
        }
        let _ = svc.save(&mut t);
        let notifier = svc.notifier.clone();
        let notify = svc.settings.notifications_enabled && t.state == TaskState::Concluido;
        drop(svc);
        if notify {
            notifier
                .download_finished("Download concluído", &body, &t.final_path())
                .await;
        }
    });
}

/// Despacha uma operação do protocolo.
pub async fn handle(shared: &Shared, op: Op) -> OpResult {
    match op {
        Op::Health => Ok(json!(shared.lock().await.health())),
        Op::ListTasks => shared.lock().await.snapshot().map(|s| json!(s)),
        Op::Probe { url } => {
            let url = match core::validate_download_url(url.trim()) {
                Ok(u) => u,
                Err(_) => {
                    return err(
                        ErrorCode::InvalidUrl,
                        "Informe um endereço http:// ou https:// válido.",
                    )
                }
            };
            // Sem trava: a consulta de rede não bloqueia a fila.
            Ok(json!(probe::probe(&url).await))
        }
        Op::AddDownload(req) => shared.lock().await.add(req).await,
        Op::Pause { id } => shared.lock().await.pause(id).await,
        Op::Resume { id } => shared.lock().await.resume(id).await,
        Op::Cancel { id } => shared.lock().await.cancel(id).await,
        Op::Retry { id } => shared.lock().await.retry(id).await,
        Op::RetryWithNewUrl { id, url } => shared.lock().await.retry_with_new_url(id, url).await,
        Op::RemoveFromHistory { id } => shared.lock().await.remove_from_history(id).await,
        Op::DeleteFile { id } => shared.lock().await.delete_file(id).await,
        Op::Move { id, direction } => shared.lock().await.move_task(id, direction).await,
        Op::ChangeConnections { id, connections } => {
            shared
                .lock()
                .await
                .change_connections(id, connections)
                .await
        }
        Op::CancelByRequestKey { key } => shared.lock().await.cancel_by_request_key(&key).await,
        Op::PauseAll => shared.lock().await.pause_all(PauseReason::Usuario).await,
        Op::ResumeAll => shared.lock().await.resume_all().await,
        Op::GetSettings => Ok(json!(shared.lock().await.settings)),
        Op::RestartEngine => {
            let mut svc = shared.lock().await;
            svc.reset_engine_backoff();
            svc.ensure_engine().await;
            Ok(json!(svc.health()))
        }
        Op::SetSettings { settings } => shared.lock().await.set_settings(settings).await,
        Op::Shutdown => {
            shared.lock().await.shutdown().await;
            Ok(Value::Null)
        }
    }
}
