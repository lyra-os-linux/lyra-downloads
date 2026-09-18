//! Consulta leve (HEAD) para sugerir nome e tamanho antes de criar a tarefa.
//! Falhas aqui nunca impedem o download: são só sugestões.

use std::time::Duration;

use lyra_downloads_core::sanitize;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub filename: String,
    pub size: Option<u64>,
    /// `Some(false)` quando o servidor declarou que não aceita ranges: só uma
    /// conexão será efetiva, independentemente do perfil escolhido.
    pub accepts_ranges: Option<bool>,
}

pub async fn probe(url: &url_like::Url) -> ProbeResult {
    let fallback = sanitize::filename_from_url(url);
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return ProbeResult {
                filename: fallback,
                size: None,
                accepts_ranges: None,
            }
        }
    };

    let resp = match client.head(url.as_str()).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => {
            return ProbeResult {
                filename: fallback,
                size: None,
                accepts_ranges: None,
            }
        }
    };

    let headers = resp.headers();
    let filename = headers
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(sanitize::filename_from_content_disposition)
        .unwrap_or_else(|| sanitize::filename_from_url(resp.url()));
    let size = headers
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let accepts_ranges = headers
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("bytes"));

    ProbeResult {
        filename,
        size,
        accepts_ranges,
    }
}

/// `reqwest::Url` e `url::Url` são o mesmo tipo; alias só para clareza.
pub mod url_like {
    pub use reqwest::Url;
}
