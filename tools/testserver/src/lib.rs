//! Servidor HTTP de teste local (apenas para testes; não empacotado).
//!
//! Rotas:
//! - `/file/<n>[/nome]`     — n bytes determinísticos, suporta Range/206.
//! - `/noranges/<n>`        — ignora Range, sempre 200 completo.
//! - `/nolength/<n>`        — sem Content-Length (encerra a conexão no fim).
//! - `/slow/<n>`            — como /file, mas envia em blocos lentos.
//! - `/cd/<n>`              — com `Content-Disposition: attachment; filename="nome real.bin"`.
//! - `/redirect/<rota...>`  — 302 para `/<rota...>`.
//! - `/status/<código>`     — responde só com o código (403, 404, 429, 500...).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub fn byte_at(i: u64) -> u8 {
    ((i.wrapping_mul(31).wrapping_add(7)) % 251) as u8
}

pub fn content(n: u64) -> Vec<u8> {
    (0..n).map(byte_at).collect()
}

#[derive(Clone, Default)]
pub struct Stats {
    pub requests: Arc<AtomicUsize>,
    pub range_requests: Arc<AtomicUsize>,
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub stats: Stats,
}

impl TestServer {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

pub async fn start() -> std::io::Result<TestServer> {
    start_on("127.0.0.1:0").await
}

pub async fn start_on(bind: &str) -> std::io::Result<TestServer> {
    let listener = TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    let stats = Stats::default();
    let s = stats.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                continue;
            };
            let s = s.clone();
            tokio::spawn(async move {
                let _ = handle(sock, s).await;
            });
        }
    });
    Ok(TestServer { addr, stats })
}

async fn handle(mut sock: TcpStream, stats: Stats) -> std::io::Result<()> {
    loop {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut tmp).await?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.len() > 64 * 1024 {
                return Ok(());
            }
        }
        stats.requests.fetch_add(1, Ordering::SeqCst);
        let req = String::from_utf8_lossy(&buf).to_string();
        let mut lines = req.lines();
        let first = lines.next().unwrap_or("");
        let mut parts = first.split_whitespace();
        let method = parts.next().unwrap_or("GET").to_string();
        let target = parts.next().unwrap_or("/").to_string();
        let path = target.split('?').next().unwrap_or("/").to_string();
        let range = lines
            .clone()
            .find(|l| l.to_ascii_lowercase().starts_with("range:"))
            .map(|l| l[6..].trim().to_string());
        if range.is_some() {
            stats.range_requests.fetch_add(1, Ordering::SeqCst);
        }
        let head = method == "HEAD";
        let keep = respond(&mut sock, &path, range.as_deref(), head).await?;
        if !keep {
            return Ok(());
        }
    }
}

fn parse_range(r: &str, len: u64) -> Option<(u64, u64)> {
    let spec = r.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let start: u64 = a.parse().ok()?;
    let end: u64 = if b.is_empty() {
        len - 1
    } else {
        b.parse::<u64>().ok()?.min(len - 1)
    };
    (start <= end && start < len).then_some((start, end))
}

async fn respond(
    sock: &mut TcpStream,
    path: &str,
    range: Option<&str>,
    head: bool,
) -> std::io::Result<bool> {
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let num = |i: usize| segs.get(i).and_then(|s| s.parse::<u64>().ok());
    match segs.first().copied() {
        Some("status") => {
            let code = num(1).unwrap_or(500);
            let h = format!("HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            sock.write_all(h.as_bytes()).await?;
            Ok(false)
        }
        Some("redirect") => {
            let rest = segs[1..].join("/");
            let h = format!("HTTP/1.1 302 Found\r\nLocation: /{rest}\r\nContent-Length: 0\r\n\r\n");
            sock.write_all(h.as_bytes()).await?;
            Ok(true)
        }
        Some(kind @ ("file" | "noranges" | "nolength" | "slow" | "cd")) => {
            let n = num(1).unwrap_or(0);
            let ranges = kind != "noranges" && kind != "nolength";
            let mut extra = String::new();
            if kind == "cd" {
                extra.push_str("Content-Disposition: attachment; filename=\"nome real.bin\"\r\n");
            }
            let (status, start, end) = match (ranges, range.and_then(|r| parse_range(r, n))) {
                (true, Some((s, e))) => {
                    extra.push_str(&format!("Content-Range: bytes {s}-{e}/{n}\r\n"));
                    ("206 Partial Content", s, e + 1)
                }
                _ => ("200 OK", 0, n),
            };
            if ranges {
                extra.push_str("Accept-Ranges: bytes\r\n");
            }
            let len_hdr = if kind == "nolength" {
                "Connection: close\r\n".to_string()
            } else {
                format!("Content-Length: {}\r\n", end - start)
            };
            let h = format!("HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\n{len_hdr}{extra}\r\n");
            sock.write_all(h.as_bytes()).await?;
            if !head {
                let chunk = if kind == "slow" {
                    32 * 1024
                } else {
                    256 * 1024
                };
                let mut i = start;
                while i < end {
                    let j = (i + chunk).min(end);
                    let data: Vec<u8> = (i..j).map(byte_at).collect();
                    sock.write_all(&data).await?;
                    i = j;
                    if kind == "slow" {
                        tokio::time::sleep(Duration::from_millis(40)).await;
                    }
                }
            }
            Ok(kind != "nolength")
        }
        _ => {
            sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await?;
            Ok(true)
        }
    }
}
