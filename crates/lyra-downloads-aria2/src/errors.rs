//! Tradução dos códigos de erro do aria2 (tabela "Exit status" do manual,
//! conferida para a versão 1.37.0) para mensagens traduzidas com fallback em inglês.
//!
//! As mensagens descrevem o que o motor observou, sem afirmar causas que o
//! aria2 não consegue determinar (ex.: um 403 pode ser link expirado,
//! bloqueio por região, falta de cookie de sessão, limite do servidor...).

use gettextrs::gettext as tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Vale a pena tentar de novo com o mesmo link.
    Transitorio,
    /// Tentar de novo com o mesmo link provavelmente falha; talvez um novo link resolva.
    Link,
    /// Problema local (disco, permissão, destino).
    Local,
    Outro,
}

pub struct ErrorDescription {
    pub message: String,
    pub category: ErrorCategory,
}

pub fn describe(code: u32) -> ErrorDescription {
    use ErrorCategory::*;
    let (message, category) = match code {
        2 => (tr("Timed out while contacting the server."), Transitorio),
        3 => (
            tr("Resource not found on the server (for example, HTTP 404)."),
            Link,
        ),
        4 => (tr("The server repeatedly responded \"not found\"."), Link),
        5 => (
            tr("The speed fell below the configured minimum."),
            Transitorio,
        ),
        6 => (tr("Network problem while downloading."), Transitorio),
        8 => (
            tr("The server does not support resuming a partial download."),
            Link,
        ),
        9 => (tr("Not enough disk space at the destination."), Local),
        10 => (
            tr("The .aria2 control file does not match the download."),
            Local,
        ),
        11 => (tr("Another task is already downloading this file."), Local),
        13 => (
            tr("A file with this name already exists at the destination."),
            Local,
        ),
        14 => (tr("Could not rename the file."), Local),
        15 => (tr("Could not open the existing partial file."), Local),
        16 => (
            tr("Could not create the file at the destination (check permissions)."),
            Local,
        ),
        17 => (tr("Disk read/write error."), Local),
        18 => (tr("Could not create the destination folder."), Local),
        19 => (tr("Could not resolve the server name (DNS)."), Transitorio),
        22 => (
            tr("The server denied access or responded unexpectedly."),
            Link,
        ),
        23 => (tr("Too many redirects."), Link),
        24 => (tr("The server requires authentication."), Link),
        28 => (
            tr("Invalid option sent to the engine (internal error)."),
            Outro,
        ),
        29 => (
            tr("The server is temporarily unavailable or overloaded (for example, HTTP 429/503)."),
            Transitorio,
        ),
        32 => (tr("The engine integrity check failed."), Outro),
        _ => (tr("The download engine reported an error."), Outro),
    };
    ErrorDescription { message, category }
}
