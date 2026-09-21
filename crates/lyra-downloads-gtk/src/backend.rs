//! Chamadas ao backend fora da thread principal do GTK. Cada chamada abre
//! uma conexão curta ao socket local (barato) em uma thread do pool do GIO.

use crate::i18n::{tr, trf};
use lyra_downloads_ipc::client::{BackendClient, ClientError};
use lyra_downloads_ipc::Op;
use serde::de::DeserializeOwned;

pub async fn call<T: DeserializeOwned + Send + 'static>(op: Op) -> Result<T, String> {
    gtk::gio::spawn_blocking(move || {
        let mut c = BackendClient::connect_or_spawn()?;
        c.call::<T>(op)
    })
    .await
    .map_err(|_| tr("Internal error while contacting the download service."))?
    .map_err(|e: ClientError| match e {
        ClientError::Unavailable(detail) => trf(
            "Download service unavailable: {detail}",
            &[("detail", &detail)],
        ),
        ClientError::Io(detail) => trf(
            "Could not communicate with the download service: {detail}",
            &[("detail", &detail.to_string())],
        ),
        ClientError::Protocol(detail) => trf(
            "Invalid response from the download service: {detail}",
            &[("detail", &detail)],
        ),
        ClientError::Backend { message, .. } => message,
    })
}

/// Dispara uma operação sem resultado e reporta erro via callback na thread GTK.
pub fn fire(op: Op, on_error: impl Fn(String) + 'static) {
    gtk::glib::spawn_future_local(async move {
        if let Err(e) = call::<serde_json::Value>(op).await {
            on_error(e);
        }
    });
}
