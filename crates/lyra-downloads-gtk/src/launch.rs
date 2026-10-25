//! Linha de comando e pedidos vindos do navegador:
//! `lyra-downloads [--add-url URL [--suggested-name NOME] [--request-id ID]]`.

use std::collections::HashSet;
use std::sync::Mutex;

use serde::Serialize;

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Request {
    pub url: Option<String>,
    pub suggested_name: Option<String>,
    pub request_id: Option<String>,
}

impl Request {
    pub fn parse(argv: impl Iterator<Item = String>) -> Self {
        let mut request = Self::default();
        let mut args = argv.skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--add-url" => request.url = args.next(),
                "--suggested-name" => request.suggested_name = args.next(),
                "--request-id" => request.request_id = args.next(),
                _ => {}
            }
        }
        request
    }
}

/// Pedido da primeira execução (lido pela interface ao abrir) e IDs de
/// requisições já apresentadas: um reenvio do navegador não abre um segundo
/// diálogo (o serviço também é idempotente pela mesma chave).
pub struct Pending {
    first: Mutex<Option<Request>>,
    seen: Mutex<HashSet<String>>,
}

impl Pending {
    pub fn new(first: Request) -> Self {
        let pending = Self {
            first: Mutex::new(None),
            seen: Mutex::new(HashSet::new()),
        };
        let first = pending.accept(first);
        *pending.first.lock().unwrap() = first;
        pending
    }

    /// Devolve o pedido se ele tiver URL e ainda não tiver sido apresentado.
    pub fn accept(&self, request: Request) -> Option<Request> {
        request.url.as_ref()?;
        if let Some(id) = &request.request_id {
            if !self.seen.lock().unwrap().insert(id.clone()) {
                return None;
            }
        }
        Some(request)
    }
}

#[tauri::command]
pub fn take_launch_request(pending: tauri::State<'_, Pending>) -> Option<Request> {
    pending.first.lock().unwrap().take()
}

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
    locale: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        locale: crate::i18n::active_locale(),
    }
}

/// Erros da interface que impediriam a janela de iniciar vão para o stderr.
#[tauri::command]
pub fn app_log(message: String) {
    eprintln!("lyra-downloads (ui): {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> std::vec::IntoIter<String> {
        let mut all = vec!["lyra-downloads".to_owned()];
        all.extend(args.iter().map(|s| s.to_string()));
        all.into_iter()
    }

    #[test]
    fn parses_browser_handoff() {
        let r = Request::parse(argv(&[
            "--add-url",
            "https://x/a.iso",
            "--suggested-name",
            "a.iso",
            "--request-id",
            "42",
        ]));
        assert_eq!(r.url.as_deref(), Some("https://x/a.iso"));
        assert_eq!(r.suggested_name.as_deref(), Some("a.iso"));
        assert_eq!(r.request_id.as_deref(), Some("42"));
    }

    #[test]
    fn presents_each_browser_request_once() {
        let pending = Pending::new(Request::default());
        let request = Request::parse(argv(&["--add-url", "https://x/a", "--request-id", "7"]));
        assert!(pending.accept(request.clone()).is_some());
        assert!(pending.accept(request).is_none());
        assert!(pending
            .accept(Request::parse(argv(&["--add-url", "https://x/b"])))
            .is_some());
        assert!(pending.accept(Request::default()).is_none());
    }

    #[test]
    fn first_launch_request_is_taken_once() {
        let pending = Pending::new(Request::parse(argv(&[
            "--add-url",
            "https://x/a",
            "--request-id",
            "1",
        ])));
        assert!(pending.first.lock().unwrap().take().is_some());
        assert!(pending.first.lock().unwrap().take().is_none());
        // O mesmo pedido repassado depois por uma segunda execução é ignorado.
        assert!(pending
            .accept(Request::parse(argv(&[
                "--add-url",
                "https://x/a",
                "--request-id",
                "1"
            ])))
            .is_none());
    }
}
