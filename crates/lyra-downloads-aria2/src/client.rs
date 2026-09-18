//! Cliente JSON-RPC do aria2 (HTTP em loopback, com `rpc-secret`).
//!
//! Assinaturas conferidas empiricamente contra aria2 1.37.0:
//! - `aria2.addUri(secret, [uris], options)` — `uris` é um array.
//! - Valores numéricos em `tellStatus` chegam como strings.
//! - `addUri` com um `gid` já em uso falha com a mensagem enganosa
//!   "No URI to download." — o chamador deve tentar de novo sem `gid`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum Aria2Error {
    #[error("falha de comunicação com o aria2: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("aria2 respondeu com erro {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("resposta inesperada do aria2: {0}")]
    Protocol(String),
}

pub type Aria2Result<T> = Result<T, Aria2Error>;

/// Opções por download, sempre como strings (formato exigido pelo aria2).
pub type Aria2Options = BTreeMap<String, String>;

#[derive(Debug, Clone)]
pub struct Aria2Client {
    http: reqwest::Client,
    endpoint: String,
    token: String,
    next_id: std::sync::Arc<AtomicU64>,
}

impl Aria2Client {
    pub fn new(port: u16, secret: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .no_proxy()
            .build()
            .expect("cliente HTTP local sempre constrói");
        Self {
            http,
            endpoint: format!("http://127.0.0.1:{port}/jsonrpc"),
            token: format!("token:{secret}"),
            next_id: std::sync::Arc::new(AtomicU64::new(1)),
        }
    }

    async fn call(&self, method: &str, mut params: Vec<Value>) -> Aria2Result<Value> {
        params.insert(0, Value::String(self.token.clone()));
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body =
            json!({ "jsonrpc": "2.0", "id": id.to_string(), "method": method, "params": params });
        let response: Value = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = response.get("error") {
            return Err(Aria2Error::Rpc {
                code: err.get("code").and_then(Value::as_i64).unwrap_or(-1),
                message: err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| Aria2Error::Protocol(format!("sem campo result em {method}")))
    }

    pub async fn get_version(&self) -> Aria2Result<String> {
        let v = self.call("aria2.getVersion", vec![]).await?;
        Ok(v.get("version")
            .and_then(Value::as_str)
            .unwrap_or("desconhecida")
            .to_string())
    }

    /// Adiciona um download. Retorna o GID atribuído.
    pub async fn add_uri(&self, uri: &str, options: &Aria2Options) -> Aria2Result<String> {
        let v = self
            .call("aria2.addUri", vec![json!([uri]), json!(options)])
            .await?;
        v.as_str()
            .map(str::to_string)
            .ok_or_else(|| Aria2Error::Protocol("addUri não retornou GID".into()))
    }

    pub async fn tell_status(&self, gid: &str) -> Aria2Result<Aria2Status> {
        let v = self
            .call("aria2.tellStatus", vec![json!(gid), json!(STATUS_KEYS)])
            .await?;
        serde_json::from_value(v).map_err(|e| Aria2Error::Protocol(e.to_string()))
    }

    pub async fn tell_active(&self) -> Aria2Result<Vec<Aria2Status>> {
        let v = self
            .call("aria2.tellActive", vec![json!(STATUS_KEYS)])
            .await?;
        serde_json::from_value(v).map_err(|e| Aria2Error::Protocol(e.to_string()))
    }

    pub async fn tell_waiting(&self) -> Aria2Result<Vec<Aria2Status>> {
        let v = self
            .call(
                "aria2.tellWaiting",
                vec![json!(0), json!(10_000), json!(STATUS_KEYS)],
            )
            .await?;
        serde_json::from_value(v).map_err(|e| Aria2Error::Protocol(e.to_string()))
    }

    pub async fn tell_stopped(&self) -> Aria2Result<Vec<Aria2Status>> {
        let v = self
            .call(
                "aria2.tellStopped",
                vec![json!(0), json!(10_000), json!(STATUS_KEYS)],
            )
            .await?;
        serde_json::from_value(v).map_err(|e| Aria2Error::Protocol(e.to_string()))
    }

    pub async fn pause(&self, gid: &str) -> Aria2Result<()> {
        self.call("aria2.pause", vec![json!(gid)]).await.map(|_| ())
    }

    pub async fn force_pause(&self, gid: &str) -> Aria2Result<()> {
        self.call("aria2.forcePause", vec![json!(gid)])
            .await
            .map(|_| ())
    }

    pub async fn unpause(&self, gid: &str) -> Aria2Result<()> {
        self.call("aria2.unpause", vec![json!(gid)])
            .await
            .map(|_| ())
    }

    pub async fn force_remove(&self, gid: &str) -> Aria2Result<()> {
        self.call("aria2.forceRemove", vec![json!(gid)])
            .await
            .map(|_| ())
    }

    pub async fn remove_download_result(&self, gid: &str) -> Aria2Result<()> {
        self.call("aria2.removeDownloadResult", vec![json!(gid)])
            .await
            .map(|_| ())
    }

    /// `how` = "POS_SET" | "POS_CUR" | "POS_END".
    pub async fn change_position(&self, gid: &str, pos: i64, how: &str) -> Aria2Result<i64> {
        let v = self
            .call(
                "aria2.changePosition",
                vec![json!(gid), json!(pos), json!(how)],
            )
            .await?;
        v.as_i64()
            .ok_or_else(|| Aria2Error::Protocol("changePosition sem posição".into()))
    }

    pub async fn change_option(&self, gid: &str, options: &Aria2Options) -> Aria2Result<()> {
        self.call("aria2.changeOption", vec![json!(gid), json!(options)])
            .await
            .map(|_| ())
    }

    pub async fn change_global_option(&self, options: &Aria2Options) -> Aria2Result<()> {
        self.call("aria2.changeGlobalOption", vec![json!(options)])
            .await
            .map(|_| ())
    }

    pub async fn pause_all(&self) -> Aria2Result<()> {
        self.call("aria2.forcePauseAll", vec![]).await.map(|_| ())
    }

    pub async fn save_session(&self) -> Aria2Result<()> {
        self.call("aria2.saveSession", vec![]).await.map(|_| ())
    }

    pub async fn shutdown(&self) -> Aria2Result<()> {
        self.call("aria2.shutdown", vec![]).await.map(|_| ())
    }
}

const STATUS_KEYS: &[&str] = &[
    "gid",
    "status",
    "totalLength",
    "completedLength",
    "downloadSpeed",
    "connections",
    "errorCode",
    "errorMessage",
    "dir",
    "files",
];

fn de_u64_str<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let s: Option<String> = Option::deserialize(d)?;
    Ok(s.and_then(|s| s.parse().ok()).unwrap_or(0))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Aria2File {
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Aria2Status {
    pub gid: String,
    pub status: String,
    #[serde(default, deserialize_with = "de_u64_str")]
    pub total_length: u64,
    #[serde(default, deserialize_with = "de_u64_str")]
    pub completed_length: u64,
    #[serde(default, deserialize_with = "de_u64_str")]
    pub download_speed: u64,
    #[serde(default, deserialize_with = "de_u64_str")]
    pub connections: u64,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub files: Vec<Aria2File>,
}

impl Aria2Status {
    pub fn error_code_num(&self) -> Option<u32> {
        self.error_code.as_deref().and_then(|c| c.parse().ok())
    }
}
