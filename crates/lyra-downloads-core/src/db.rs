//! Persistência SQLite.
//!
//! Autoridade dos dados (documentado também em `docs/ARCHITECTURE.md`):
//! - O banco (`tasks`) guarda **identidade, intenção e histórico**: qual
//!   tarefa existe, para onde vai, quais parâmetros o usuário escolheu, e
//!   o resultado final.
//! - O motor aria2 informa **estado operacional** (bytes, velocidade,
//!   conexões ativas) enquanto a tarefa está ativa — isso não é persistido
//!   campo a campo a cada tick, só snapshots relevantes (ex.: ao pausar).
//! - A sessão do aria2 (`session.aria2`) e os arquivos `.aria2` guardam os
//!   dados de retomada binários do próprio motor.
//!
//! Reconciliação na inicialização do backend: para cada `Task` com estado
//! ativo no banco, o backend consulta o aria2 pelo `aria2_gid` guardado; se
//! o GID não existir mais (sessão não recuperou, por exemplo), a tarefa é
//! marcada como pausada pelo sistema (nunca recriada como uma nova tarefa
//! duplicada) até o usuário decidir retomar ou tentar novamente.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::error::{CoreError, CoreResult};
use crate::settings::Settings;
use crate::task::{ConnectionProfile, HashVerification, PauseReason, Task, TaskState};

const SUPPORTED_SCHEMA_VERSION: i64 = 1;

pub struct Repository {
    conn: Mutex<Connection>,
}

impl Repository {
    pub fn open(path: &Path) -> CoreResult<Self> {
        let conn = Connection::open(path).map_err(CoreError::Database)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        let repo = Self {
            conn: Mutex::new(conn),
        };
        repo.migrate()?;
        Ok(repo)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> CoreResult<Self> {
        let conn = Connection::open_in_memory().map_err(CoreError::Database)?;
        let repo = Self {
            conn: Mutex::new(conn),
        };
        repo.migrate()?;
        Ok(repo)
    }

    fn migrate(&self) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

        if version > SUPPORTED_SCHEMA_VERSION {
            return Err(CoreError::IncompatibleSchema {
                found: version,
                supported: SUPPORTED_SCHEMA_VERSION,
            });
        }

        if version < 1 {
            conn.execute_batch(
                r#"
                BEGIN;
                CREATE TABLE tasks (
                    id                  TEXT PRIMARY KEY,
                    aria2_gid           TEXT,
                    url                 TEXT NOT NULL,
                    filename            TEXT NOT NULL,
                    destination_dir     TEXT NOT NULL,
                    connections         INTEGER NOT NULL,
                    expected_sha256     TEXT,
                    state               TEXT NOT NULL,
                    pause_reason        TEXT,
                    hash_verification   TEXT NOT NULL DEFAULT 'nao_verificado',
                    total_bytes         INTEGER,
                    downloaded_bytes    INTEGER NOT NULL DEFAULT 0,
                    error_message       TEXT,
                    queue_position      INTEGER NOT NULL DEFAULT 0,
                    created_at          TEXT NOT NULL,
                    updated_at          TEXT NOT NULL
                );
                CREATE INDEX idx_tasks_state ON tasks(state);
                CREATE INDEX idx_tasks_gid ON tasks(aria2_gid);

                CREATE TABLE request_keys (
                    key        TEXT PRIMARY KEY,
                    task_id    TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );

                CREATE TABLE settings (
                    key   TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                PRAGMA user_version = 1;
                COMMIT;
                "#,
            )?;
            tracing::info!("banco de dados inicializado (schema v1)");
        }

        Ok(())
    }

    // ---- tasks -----------------------------------------------------

    pub fn insert_task(&self, task: &Task) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        insert_task_stmt(&conn, task)
    }

    /// Retorna a tarefa já associada a uma chave de idempotência, se houver.
    pub fn task_for_request_key(&self, key: &str) -> CoreResult<Option<Uuid>> {
        let conn = self.conn.lock().unwrap();
        let id: Option<String> = conn
            .query_row(
                "SELECT task_id FROM request_keys WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.and_then(|s| Uuid::parse_str(&s).ok()))
    }

    /// Insere a tarefa e (opcionalmente) a chave de idempotência na mesma
    /// transação: ou ambas ficam gravadas, ou nenhuma.
    pub fn insert_task_with_key(&self, task: &Task, key: Option<&str>) -> CoreResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        insert_task_stmt(&tx, task)?;
        if let Some(key) = key {
            tx.execute(
                "INSERT INTO request_keys (key, task_id, created_at) VALUES (?1, ?2, ?3)",
                params![key, task.id.to_string(), chrono::Utc::now().to_rfc3339()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn update_task(&self, task: &Task) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            r#"UPDATE tasks SET
                aria2_gid = ?2, url = ?3, filename = ?4, destination_dir = ?5,
                connections = ?6, expected_sha256 = ?7, state = ?8,
                pause_reason = ?9, hash_verification = ?10, total_bytes = ?11,
                downloaded_bytes = ?12, error_message = ?13, queue_position = ?14,
                updated_at = ?15
            WHERE id = ?1"#,
            params![
                task.id.to_string(),
                task.aria2_gid,
                task.url,
                task.filename,
                task.destination_dir.to_string_lossy(),
                task.connections.as_u8(),
                task.expected_sha256,
                task.state.as_db_str(),
                task.pause_reason.map(PauseReason::as_db_str),
                task.hash_verification.as_db_str(),
                task.total_bytes,
                task.downloaded_bytes as i64,
                task.error_message,
                task.queue_position,
                task.updated_at.to_rfc3339(),
            ],
        )?;
        if changed == 0 {
            return Err(CoreError::TaskNotFound(task.id));
        }
        Ok(())
    }

    pub fn get_task(&self, id: Uuid) -> CoreResult<Option<Task>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT * FROM tasks WHERE id = ?1",
            params![id.to_string()],
            row_to_task,
        )
        .optional()
        .map_err(CoreError::Database)
    }

    pub fn get_task_by_gid(&self, gid: &str) -> CoreResult<Option<Task>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT * FROM tasks WHERE aria2_gid = ?1",
            params![gid],
            row_to_task,
        )
        .optional()
        .map_err(CoreError::Database)
    }

    pub fn list_tasks(&self) -> CoreResult<Vec<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT * FROM tasks ORDER BY queue_position ASC, created_at ASC")?;
        let rows = stmt.query_map([], row_to_task)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(CoreError::Database)
    }

    /// Remove a tarefa do histórico (apenas metadados; não mexe no arquivo em disco).
    pub fn delete_task(&self, id: Uuid) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM tasks WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn next_queue_position(&self) -> CoreResult<i64> {
        let conn = self.conn.lock().unwrap();
        let max: Option<i64> =
            conn.query_row("SELECT MAX(queue_position) FROM tasks", [], |r| r.get(0))?;
        Ok(max.unwrap_or(0) + 1)
    }

    /// Reordena as tarefas aguardando conforme a ordem dada (índice na lista
    /// vira `queue_position`). IDs que não existem são ignorados.
    pub fn reorder_waiting(&self, ordered_ids: &[Uuid]) -> CoreResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for (position, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE tasks SET queue_position = ?2 WHERE id = ?1",
                params![id.to_string(), position as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- settings ----------------------------------------------------

    pub fn load_settings(&self) -> CoreResult<Settings> {
        let conn = self.conn.lock().unwrap();
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'settings'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match raw {
            Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
            None => Ok(Settings::default()),
        }
    }

    pub fn save_settings(&self, settings: &Settings) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        let json = serde_json::to_string(settings).expect("Settings sempre serializa");
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![json],
        )?;
        Ok(())
    }
}

fn insert_task_stmt(conn: &Connection, task: &Task) -> CoreResult<()> {
    conn.execute(
        r#"INSERT INTO tasks (
            id, aria2_gid, url, filename, destination_dir, connections,
            expected_sha256, state, pause_reason, hash_verification,
            total_bytes, downloaded_bytes, error_message, queue_position,
            created_at, updated_at
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)"#,
        params![
            task.id.to_string(),
            task.aria2_gid,
            task.url,
            task.filename,
            task.destination_dir.to_string_lossy(),
            task.connections.as_u8(),
            task.expected_sha256,
            task.state.as_db_str(),
            task.pause_reason.map(PauseReason::as_db_str),
            task.hash_verification.as_db_str(),
            task.total_bytes,
            task.downloaded_bytes as i64,
            task.error_message,
            task.queue_position,
            task.created_at.to_rfc3339(),
            task.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn row_to_task(row: &Row) -> rusqlite::Result<Task> {
    let id: String = row.get("id")?;
    let destination_dir: String = row.get("destination_dir")?;
    let connections: u8 = row.get("connections")?;
    let state: String = row.get("state")?;
    let pause_reason: Option<String> = row.get("pause_reason")?;
    let hash_verification: String = row.get("hash_verification")?;
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    let downloaded_bytes: i64 = row.get("downloaded_bytes")?;

    Ok(Task {
        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
        aria2_gid: row.get("aria2_gid")?,
        url: row.get("url")?,
        filename: row.get("filename")?,
        destination_dir: PathBuf::from(destination_dir),
        connections: ConnectionProfile::from_u8(connections).unwrap_or_default(),
        expected_sha256: row.get("expected_sha256")?,
        state: TaskState::from_db_str(&state).unwrap_or(TaskState::Erro),
        pause_reason: pause_reason.and_then(|s| PauseReason::from_db_str(&s)),
        hash_verification: HashVerification::from_db_str(&hash_verification).unwrap_or_default(),
        total_bytes: row.get("total_bytes")?,
        downloaded_bytes: downloaded_bytes.max(0) as u64,
        error_message: row.get("error_message")?,
        queue_position: row.get("queue_position")?,
        created_at: parse_rfc3339(&created_at),
        updated_at: parse_rfc3339(&updated_at),
    })
}

fn parse_rfc3339(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task() -> Task {
        let now = chrono::Utc::now();
        Task {
            id: Uuid::new_v4(),
            aria2_gid: None,
            url: "https://example.org/file.iso".into(),
            filename: "file.iso".into(),
            destination_dir: PathBuf::from("/home/user/Downloads"),
            connections: ConnectionProfile::Four,
            expected_sha256: None,
            state: TaskState::Aguardando,
            pause_reason: None,
            hash_verification: HashVerification::NaoVerificado,
            total_bytes: None,
            downloaded_bytes: 0,
            error_message: None,
            queue_position: 1,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let repo = Repository::open_in_memory().unwrap();
        let task = sample_task();
        repo.insert_task(&task).unwrap();
        let fetched = repo.get_task(task.id).unwrap().unwrap();
        assert_eq!(fetched.url, task.url);
        assert_eq!(fetched.state, TaskState::Aguardando);
        assert_eq!(fetched.connections, ConnectionProfile::Four);
    }

    #[test]
    fn request_key_is_idempotent_and_atomic() {
        let repo = Repository::open_in_memory().unwrap();
        let t1 = sample_task();
        repo.insert_task_with_key(&t1, Some("req-1")).unwrap();
        assert_eq!(repo.task_for_request_key("req-1").unwrap(), Some(t1.id));
        // A mesma chave de novo falha e não deixa uma segunda tarefa gravada.
        let t2 = sample_task();
        assert!(repo.insert_task_with_key(&t2, Some("req-1")).is_err());
        assert!(repo.get_task(t2.id).unwrap().is_none());
        // Mesma URL com outra chave é permitida (download intencional repetido).
        let t3 = sample_task();
        repo.insert_task_with_key(&t3, Some("req-2")).unwrap();
        assert_eq!(repo.list_tasks().unwrap().len(), 2);
    }

    #[test]
    fn update_nonexistent_fails() {
        let repo = Repository::open_in_memory().unwrap();
        let task = sample_task();
        let err = repo.update_task(&task).unwrap_err();
        assert!(matches!(err, CoreError::TaskNotFound(_)));
    }

    #[test]
    fn delete_removes_metadata_only() {
        let repo = Repository::open_in_memory().unwrap();
        let task = sample_task();
        repo.insert_task(&task).unwrap();
        repo.delete_task(task.id).unwrap();
        assert!(repo.get_task(task.id).unwrap().is_none());
    }

    #[test]
    fn settings_roundtrip() {
        let repo = Repository::open_in_memory().unwrap();
        let settings = Settings {
            max_concurrent_downloads: 5,
            ..Default::default()
        };
        repo.save_settings(&settings).unwrap();
        let loaded = repo.load_settings().unwrap();
        assert_eq!(loaded.max_concurrent_downloads, 5);
    }

    #[test]
    fn reorder_waiting_updates_positions() {
        let repo = Repository::open_in_memory().unwrap();
        let mut t1 = sample_task();
        t1.queue_position = 1;
        let mut t2 = sample_task();
        t2.queue_position = 2;
        repo.insert_task(&t1).unwrap();
        repo.insert_task(&t2).unwrap();
        repo.reorder_waiting(&[t2.id, t1.id]).unwrap();
        let list = repo.list_tasks().unwrap();
        assert_eq!(list[0].id, t2.id);
        assert_eq!(list[1].id, t1.id);
    }
}
