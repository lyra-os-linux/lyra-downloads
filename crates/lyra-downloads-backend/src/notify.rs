//! Notificações de conclusão via `org.freedesktop.Notifications`, com a
//! ação "Abrir pasta". Funciona mesmo com a janela fechada, porque o
//! backend continua vivo. Nunca executa o arquivo baixado.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;
use zbus::zvariant::Value;

#[derive(Clone)]
pub struct Notifier {
    conn: Option<zbus::Connection>,
    folders: Arc<Mutex<HashMap<u32, PathBuf>>>,
}

impl Notifier {
    pub async fn new() -> Self {
        let conn = zbus::Connection::session().await.ok();
        let notifier = Self {
            conn,
            folders: Arc::new(Mutex::new(HashMap::new())),
        };
        notifier.listen_actions();
        notifier
    }

    pub fn disabled() -> Self {
        Self {
            conn: None,
            folders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn listen_actions(&self) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let folders = self.folders.clone();
        tokio::spawn(async move {
            use futures_lite::StreamExt;
            let rule = match zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface("org.freedesktop.Notifications")
                .and_then(|b| b.member("ActionInvoked"))
            {
                Ok(b) => b.build(),
                Err(_) => return,
            };
            let Ok(mut stream) = zbus::MessageStream::for_match_rule(rule, &conn, None).await
            else {
                return;
            };
            while let Some(Ok(msg)) = stream.next().await {
                let Ok((id, action)) = msg.body().deserialize::<(u32, String)>() else {
                    continue;
                };
                if action != "open-folder" {
                    continue;
                }
                if let Some(dir) = folders.lock().await.get(&id).cloned() {
                    open_folder(&conn, &dir).await;
                }
            }
        });
    }

    pub async fn download_finished(&self, title: &str, body: &str, file: &Path) {
        let Some(conn) = &self.conn else { return };
        let mut hints: HashMap<&str, Value> = HashMap::new();
        hints.insert("desktop-entry", Value::from("org.lyraos.Downloads"));
        let actions = vec!["open-folder", "Abrir pasta"];
        let reply = conn
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    "Lyra Downloads",
                    0u32,
                    "org.lyraos.Downloads",
                    title,
                    body,
                    actions,
                    hints,
                    -1i32,
                ),
            )
            .await;
        match reply.and_then(|m| m.body().deserialize::<u32>()) {
            Ok(id) => {
                self.folders.lock().await.insert(id, file.to_path_buf());
            }
            Err(e) => tracing::debug!("notificação indisponível: {e}"),
        }
    }
}

/// Abre a pasta do arquivo no gerenciador de arquivos, selecionando o item
/// (`org.freedesktop.FileManager1.ShowItems`), com fallback para `gio open`.
pub async fn open_folder(conn: &zbus::Connection, file: &Path) {
    let uri = format!("file://{}", percent_encode_path(file));
    let shown = conn
        .call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(vec![uri.as_str()], ""),
        )
        .await
        .is_ok();
    if !shown {
        if let Some(dir) = file.parent() {
            let _ = std::process::Command::new("gio")
                .arg("open")
                .arg(dir)
                .spawn();
        }
    }
}

fn percent_encode_path(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
