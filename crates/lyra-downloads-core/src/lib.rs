pub mod db;
pub mod error;
pub mod paths;
pub mod sanitize;
pub mod settings;
pub mod task;

pub use error::{CoreError, CoreResult};
pub use task::{ConnectionProfile, HashVerification, NewTask, PauseReason, Task, TaskState};

use chrono::Utc;
use uuid::Uuid;

/// Valida que a URL é http(s) e a normaliza (`url::Url` já preserva query
/// strings e assinaturas corretamente ao fazer round-trip por `as_str`).
pub fn validate_download_url(raw: &str) -> CoreResult<url::Url> {
    let parsed =
        url::Url::parse(raw).map_err(|_| CoreError::UnsupportedUrlScheme(raw.to_string()))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(CoreError::UnsupportedUrlScheme(other.to_string())),
    }
}

/// Versão de uma URL segura para logs e diagnósticos: remove credenciais,
/// query string e fragmento (onde costumam ficar tokens e assinaturas).
pub fn redact_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut u) => {
            let had_query = u.query().is_some();
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.set_query(None);
            u.set_fragment(None);
            let mut s = u.to_string();
            if had_query {
                s.push_str("?[redigido]");
            }
            s
        }
        Err(_) => "[url inválida]".to_string(),
    }
}

/// Constrói uma `Task` completa a partir dos dados de criação, validando o
/// destino e resolvendo colisão de nome de arquivo. Não toca no disco além
/// de checar existência (o arquivo só é criado pelo aria2).
pub fn build_task(
    new_task: NewTask,
    reserved: &std::collections::HashSet<std::path::PathBuf>,
) -> CoreResult<Task> {
    validate_download_url(&new_task.url)?;

    if !sanitize::is_safe_destination_dir(&new_task.destination_dir) {
        return Err(CoreError::InvalidDestination(
            new_task.destination_dir.to_string_lossy().to_string(),
        ));
    }

    let resolved_path = sanitize::resolve_non_colliding_path_with(
        &new_task.destination_dir,
        &new_task.filename,
        reserved,
    );
    let final_filename = resolved_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| CoreError::InvalidFilename(new_task.filename.clone()))?;

    let now = Utc::now();
    Ok(Task {
        id: Uuid::new_v4(),
        aria2_gid: None,
        url: new_task.url,
        filename: final_filename,
        destination_dir: new_task.destination_dir,
        connections: new_task.connections,
        expected_sha256: new_task.expected_sha256,
        state: TaskState::Aguardando,
        pause_reason: None,
        hash_verification: HashVerification::NaoVerificado,
        total_bytes: None,
        downloaded_bytes: 0,
        error_message: None,
        queue_position: 0, // preenchido pelo chamador via Repository::next_queue_position
        created_at: now,
        updated_at: now,
    })
}

/// GID determinístico de 16 caracteres hex derivado do UUID da tarefa, usado
/// ao pedir ao aria2 que use esse GID (`addUri` aceita a opção `gid`). Isso
/// simplifica a reconciliação após reinício: o mesmo `Task::id` sempre
/// solicita o mesmo GID. Se o aria2 rejeitar (colisão dentro do processo em
/// execução), o chamador deve deixar o aria2 escolher um GID livre e
/// atualizar `Task::aria2_gid` com o valor retornado.
pub fn deterministic_gid(id: Uuid) -> String {
    let hex = id.simple().to_string();
    hex[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_and_query() {
        let r = redact_url("https://user:pw@example.org/a.iso?token=SECRET#x");
        assert!(!r.contains("SECRET"));
        assert!(!r.contains("pw"));
        assert_eq!(r, "https://example.org/a.iso?[redigido]");
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(validate_download_url("ftp://example.org/f").is_err());
        assert!(validate_download_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn accepts_https_with_query_and_signature() {
        let url = validate_download_url("https://example.org/f.zip?token=abc&Signature=xyz%2F123")
            .unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.org/f.zip?token=abc&Signature=xyz%2F123"
        );
    }

    #[test]
    fn deterministic_gid_is_stable_and_16_hex_chars() {
        let id = Uuid::new_v4();
        let gid1 = deterministic_gid(id);
        let gid2 = deterministic_gid(id);
        assert_eq!(gid1, gid2);
        assert_eq!(gid1.len(), 16);
        assert!(gid1.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
