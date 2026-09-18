//! Native messaging host do Firefox.
//!
//! Enquadramento oficial: 4 bytes de tamanho (u32, ordem de bytes nativa)
//! seguidos de JSON UTF-8. stdout é exclusivo do protocolo; diagnósticos vão
//! para stderr, sem URLs completas nem segredos.
//!
//! O host não é dono de nada: encaminha operações permitidas ao backend
//! (que ele inicia desanexado, se preciso) e pode abrir a interface. Se o
//! Firefox encerrar o grupo de processos do host, backend e interface
//! sobrevivem, pois são iniciados com `setsid`.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// ID técnico de DESENVOLVIMENTO da extensão (TLD `.invalid` reservado —
/// deixa claro que não é um ID de publicação). Substituir antes de uma
/// distribuição pública; ver docs/FIREFOX.md.
pub const EXTENSION_ID: &str = "lyra-downloads-dev@lyraos.invalid";
pub const HOST_NAME: &str = "org.lyraos.downloads";
pub const PROTOCOL_VERSION: u32 = 1;
/// Limite para mensagens recebidas (o Firefox aceita até 1 MiB na direção
/// host → navegador; usamos o mesmo teto nas duas direções).
pub const MAX_MESSAGE: usize = 1024 * 1024;

#[derive(Debug)]
pub enum FrameError {
    /// EOF limpo entre mensagens: o navegador fechou a conexão.
    Eof,
    /// EOF no meio de uma mensagem.
    Truncated,
    TooLarge(usize),
    Io(io::Error),
}

pub fn read_frame(r: &mut impl Read) -> Result<Vec<u8>, FrameError> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Err(FrameError::Eof),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(FrameError::Io(e)),
        }
    }
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(FrameError::TooLarge(len));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            FrameError::Truncated
        } else {
            FrameError::Io(e)
        }
    })?;
    Ok(buf)
}

pub fn write_frame(w: &mut impl Write, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "resposta grande demais",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HostOp {
    Health,
    OpenApp,
    /// Envio manual: abre o diálogo "Novo download" do aplicativo. Se o
    /// usuário cancelar, nenhuma tarefa é criada.
    SendDownload {
        url: String,
        #[serde(default)]
        suggested_filename: Option<String>,
    },
    /// Captura automática: pede ao backend que aceite a tarefa de forma
    /// durável; só então o navegador pode cancelar a própria transferência.
    Handoff {
        url: String,
        #[serde(default)]
        suggested_filename: Option<String>,
    },
    /// Desiste de um repasse (timeout do lado do navegador).
    CancelHandoff,
}

#[derive(Debug, Deserialize)]
pub struct HostRequest {
    pub v: u32,
    pub request_id: String,
    #[serde(flatten)]
    pub op: HostOp,
}

#[derive(Debug, Serialize)]
pub struct HostError {
    pub code: &'static str,
    pub message: String,
}

pub fn ok(request_id: &str, result: Value) -> Value {
    json!({ "v": PROTOCOL_VERSION, "request_id": request_id, "ok": true, "result": result })
}

pub fn error(request_id: Option<&str>, code: &'static str, message: impl Into<String>) -> Value {
    json!({
        "v": PROTOCOL_VERSION,
        "request_id": request_id,
        "ok": false,
        "error": HostError { code, message: message.into() },
    })
}

/// Ações com efeitos externos, abstraídas para testes.
pub trait Effects {
    fn backend(&mut self, op: lyra_downloads_ipc::Op) -> Result<Value, (String, String)>;
    fn open_app(&mut self, args: &[String]) -> Result<(), String>;
}

fn valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Processa uma mensagem já desenquadrada e devolve a resposta.
pub fn handle_message(raw: &[u8], fx: &mut impl Effects) -> Value {
    let value: Value = match serde_json::from_slice(raw) {
        Ok(v) => v,
        Err(_) => return error(None, "invalid_request", "Mensagem não é JSON válido."),
    };
    let request_id = value
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let req: HostRequest = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => {
            return error(
                request_id.as_deref(),
                "invalid_request",
                format!("Mensagem inválida: {e}"),
            )
        }
    };
    let rid = req.request_id.as_str();
    if !valid_request_id(rid) {
        return error(None, "invalid_request", "request_id inválido.");
    }
    if req.v != PROTOCOL_VERSION {
        return error(
            Some(rid),
            "unsupported_version",
            format!("Versão {} não suportada.", req.v),
        );
    }

    let check_url = |url: &str| -> Result<String, Value> {
        match lyra_downloads_core::validate_download_url(url) {
            Ok(u)
                if u.host_str().is_some() && u.username().is_empty() && u.password().is_none() =>
            {
                Ok(u.as_str().to_string())
            }
            _ => Err(error(
                Some(rid),
                "invalid_url",
                "Só endereços http:// ou https:// sem credenciais embutidas são aceitos.",
            )),
        }
    };

    match req.op {
        HostOp::Health => match fx.backend(lyra_downloads_ipc::Op::Health) {
            Ok(h) => ok(
                rid,
                json!({ "host_version": env!("CARGO_PKG_VERSION"), "backend": h }),
            ),
            Err((code, msg)) => error(Some(rid), map_code(&code), msg),
        },
        HostOp::OpenApp => match fx.open_app(&[]) {
            Ok(()) => ok(rid, json!({ "status": "app_opened" })),
            Err(e) => error(Some(rid), "app_unavailable", e),
        },
        HostOp::SendDownload {
            url,
            suggested_filename,
        } => {
            let url = match check_url(&url) {
                Ok(u) => u,
                Err(e) => return e,
            };
            let mut args = vec![
                "--add-url".to_string(),
                url,
                "--request-id".into(),
                rid.to_string(),
            ];
            if let Some(name) = suggested_filename.filter(|n| !n.is_empty()) {
                args.push("--suggested-name".into());
                args.push(lyra_downloads_core::sanitize::sanitize_filename(&name));
            }
            match fx.open_app(&args) {
                Ok(()) => ok(rid, json!({ "status": "dialog_opened" })),
                Err(e) => error(Some(rid), "app_unavailable", e),
            }
        }
        HostOp::Handoff {
            url,
            suggested_filename,
        } => {
            let url = match check_url(&url) {
                Ok(u) => u,
                Err(e) => return e,
            };
            let op = lyra_downloads_ipc::Op::AddDownload(lyra_downloads_ipc::AddDownload {
                url,
                filename: None,
                destination_dir: None,
                connections: None,
                expected_sha256: None,
                source: lyra_downloads_ipc::Source::Navegador,
                idempotency_key: Some(format!("firefox:{rid}")),
                suggested_filename,
            });
            match fx.backend(op) {
                Ok(r) => ok(rid, json!({ "status": "accepted", "task": r })),
                Err((code, msg)) => error(Some(rid), map_code(&code), msg),
            }
        }
        HostOp::CancelHandoff => {
            match fx.backend(lyra_downloads_ipc::Op::CancelByRequestKey {
                key: format!("firefox:{rid}"),
            }) {
                Ok(r) => ok(rid, r),
                Err((code, msg)) => error(Some(rid), map_code(&code), msg),
            }
        }
    }
}

fn map_code(code: &str) -> &'static str {
    match code {
        "unavailable" => "app_unavailable",
        "invalid_url" => "invalid_url",
        "invalid_destination" => "invalid_destination",
        "invalid_request" => "invalid_request",
        _ => "backend_error",
    }
}

/// Laço principal: lê mensagens até EOF. Mensagens inválidas recebem erro
/// estruturado e o laço continua; mensagens grandes demais ou truncadas
/// encerram a conexão (o fluxo de bytes deixou de ser confiável).
pub fn run(
    input: &mut impl Read,
    output: &mut impl Write,
    fx: &mut impl Effects,
) -> io::Result<()> {
    loop {
        match read_frame(input) {
            Ok(raw) => {
                let resp = handle_message(&raw, fx);
                write_frame(output, &resp)?;
            }
            Err(FrameError::Eof) => return Ok(()),
            Err(FrameError::TooLarge(n)) => {
                let _ = write_frame(
                    output,
                    &error(
                        None,
                        "too_large",
                        format!("Mensagem de {n} bytes excede o limite."),
                    ),
                );
                return Ok(());
            }
            Err(FrameError::Truncated) => {
                eprintln!("lyra-downloads-nativehost: mensagem truncada; encerrando");
                return Ok(());
            }
            Err(FrameError::Io(e)) => return Err(e),
        }
    }
}

/// Valida a origem informada pelo Firefox (ID da extensão no último
/// argumento; o primeiro é o caminho do manifesto do host).
pub fn caller_allowed(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == EXTENSION_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Fake {
        backend_calls: Vec<String>,
        opened: Vec<Vec<String>>,
        accepted_keys: std::collections::HashMap<String, String>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                backend_calls: vec![],
                opened: vec![],
                accepted_keys: Default::default(),
            }
        }
    }

    impl Effects for Fake {
        fn backend(&mut self, op: lyra_downloads_ipc::Op) -> Result<Value, (String, String)> {
            self.backend_calls.push(serde_json::to_string(&op).unwrap());
            match op {
                lyra_downloads_ipc::Op::AddDownload(a) => {
                    // Simula a idempotência do backend.
                    let key = a.idempotency_key.unwrap();
                    let n = self.accepted_keys.len();
                    let dup = self.accepted_keys.contains_key(&key);
                    let id = self
                        .accepted_keys
                        .entry(key)
                        .or_insert(format!("task-{n}"))
                        .clone();
                    Ok(json!({ "task_id": id, "duplicate": dup }))
                }
                _ => Ok(json!({})),
            }
        }
        fn open_app(&mut self, args: &[String]) -> Result<(), String> {
            self.opened.push(args.to_vec());
            Ok(())
        }
    }

    fn frame(v: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        write_frame(&mut out, v).unwrap();
        out
    }

    fn responses(out: &[u8]) -> Vec<Value> {
        let mut c = Cursor::new(out.to_vec());
        let mut v = vec![];
        while let Ok(f) = read_frame(&mut c) {
            v.push(serde_json::from_slice(&f).unwrap());
        }
        v
    }

    /// Leitor que entrega 1 byte por chamada (mensagens fragmentadas).
    struct Trickle(Cursor<Vec<u8>>);
    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(1);
            self.0.read(&mut buf[..n])
        }
    }

    #[test]
    fn fragmented_input_is_reassembled() {
        let mut input = frame(&json!({"v":1,"request_id":"a1","op":"health"}));
        input.extend(frame(&json!({"v":1,"request_id":"a2","op":"open_app"})));
        let mut out = vec![];
        let mut fx = Fake::new();
        run(&mut Trickle(Cursor::new(input)), &mut out, &mut fx).unwrap();
        let r = responses(&out);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0]["request_id"], "a1");
        assert_eq!(r[1]["result"]["status"], "app_opened");
    }

    #[test]
    fn invalid_messages_get_structured_errors_and_loop_continues() {
        let mut input = Vec::new();
        let garbage = b"{isto nao e json";
        input.extend((garbage.len() as u32).to_ne_bytes());
        input.extend(garbage);
        input.extend(frame(
            &json!({"v":1,"request_id":"x","op":"aria2_raw","method":"aria2.shutdown"}),
        ));
        input.extend(frame(&json!({"v":2,"request_id":"y","op":"health"})));
        input.extend(frame(
            &json!({"v":1,"request_id":"z","op":"handoff","url":"file:///etc/passwd"}),
        ));
        input.extend(frame(
            &json!({"v":1,"request_id":"w","op":"handoff","url":"https://u:p@example.org/a"}),
        ));
        input.extend(frame(
            &json!({"v":1,"request_id":"../bad id","op":"health"}),
        ));
        let mut out = vec![];
        let mut fx = Fake::new();
        run(&mut Cursor::new(input), &mut out, &mut fx).unwrap();
        let r = responses(&out);
        let codes: Vec<&str> = r
            .iter()
            .map(|v| v["error"]["code"].as_str().unwrap())
            .collect();
        assert_eq!(
            codes,
            [
                "invalid_request",
                "invalid_request",
                "unsupported_version",
                "invalid_url",
                "invalid_url",
                "invalid_request"
            ]
        );
        assert!(
            fx.backend_calls.is_empty(),
            "nada inválido chega ao backend"
        );
    }

    #[test]
    fn oversized_and_truncated_close_cleanly() {
        let mut input = ((MAX_MESSAGE + 1) as u32).to_ne_bytes().to_vec();
        input.extend(vec![b' '; 16]);
        let mut out = vec![];
        run(&mut Cursor::new(input), &mut out, &mut Fake::new()).unwrap();
        assert_eq!(responses(&out)[0]["error"]["code"], "too_large");

        let mut input = 100u32.to_ne_bytes().to_vec();
        input.extend(b"{\"v\":1");
        let mut out = vec![];
        run(&mut Cursor::new(input), &mut out, &mut Fake::new()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn repeated_handoff_is_idempotent_by_request_id() {
        let msg = json!({"v":1,"request_id":"req-7","op":"handoff","url":"https://example.org/a.iso?sig=abc"});
        let mut input = frame(&msg);
        input.extend(frame(&msg));
        input.extend(frame(&json!({"v":1,"request_id":"req-8","op":"handoff","url":"https://example.org/a.iso?sig=abc"})));
        let mut out = vec![];
        let mut fx = Fake::new();
        run(&mut Cursor::new(input), &mut out, &mut fx).unwrap();
        let r = responses(&out);
        assert_eq!(
            r[0]["result"]["task"]["task_id"],
            r[1]["result"]["task"]["task_id"]
        );
        assert_eq!(r[1]["result"]["task"]["duplicate"], true);
        assert_ne!(
            r[0]["result"]["task"]["task_id"],
            r[2]["result"]["task"]["task_id"]
        );
        // A URL assinada chega intacta ao backend.
        assert!(fx.backend_calls[0].contains("sig=abc"));
    }

    #[test]
    fn send_download_opens_dialog_with_separate_args() {
        let mut out = vec![];
        let mut fx = Fake::new();
        let msg = json!({"v":1,"request_id":"m1","op":"send_download","url":"https://example.org/x y.iso","suggested_filename":"../x.iso"});
        run(&mut Cursor::new(frame(&msg)), &mut out, &mut fx).unwrap();
        assert_eq!(responses(&out)[0]["result"]["status"], "dialog_opened");
        let args = &fx.opened[0];
        assert_eq!(args[0], "--add-url");
        assert_eq!(args[1], "https://example.org/x%20y.iso");
        assert_eq!(args[3], "m1");
        assert_eq!(args[5], "x.iso", "nome sugerido é sanitizado");
        assert!(
            fx.backend_calls.is_empty(),
            "envio manual não cria tarefa sem confirmação do usuário"
        );
    }

    #[test]
    fn caller_validation() {
        let args = vec![
            "host".into(),
            "/path/manifest.json".into(),
            EXTENSION_ID.into(),
        ];
        assert!(caller_allowed(&args));
        assert!(!caller_allowed(&[
            "host".into(),
            "/m.json".into(),
            "other@ext".into()
        ]));
    }
}
