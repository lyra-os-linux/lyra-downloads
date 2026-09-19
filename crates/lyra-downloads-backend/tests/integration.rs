//! Testes de ponta a ponta: backend real (em processo), aria2c real e
//! servidor HTTP local. Não acessam a Internet. Pulados (com aviso) se o
//! aria2c não estiver instalado.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lyra_downloads_backend::{run, Paths};
use lyra_downloads_core::{HashVerification, TaskState};
use lyra_downloads_ipc::client::BackendClient;
use lyra_downloads_ipc::{AddDownload, AddResult, ErrorCode, Op, Snapshot, Source, TaskView};
use uuid::Uuid;

struct Env {
    _tmp: tempfile::TempDir,
    paths_root: PathBuf,
    downloads: PathBuf,
    socket: PathBuf,
}

fn aria2_available() -> bool {
    lyra_downloads_aria2::process::find_aria2c().is_some()
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let downloads = root.join("Downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::create_dir_all(root.join("state/aria2")).unwrap();
    Env {
        socket: root.join("backend.sock"),
        paths_root: root,
        downloads,
        _tmp: tmp,
    }
}

fn paths(e: &Env) -> Paths {
    Paths {
        database: e.paths_root.join("state/db.sqlite"),
        session_dir: e.paths_root.join("state/aria2"),
        socket: e.socket.clone(),
        lock: e.paths_root.join("backend.lock"),
    }
}

/// Inicia o backend em uma thread com runtime próprio.
fn start_backend(e: &Env) -> std::thread::JoinHandle<()> {
    let p = paths(e);
    let h = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async { run(p, false).await.unwrap() });
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while BackendClient::connect(&e.socket).is_err() {
        assert!(Instant::now() < deadline, "backend não subiu");
        std::thread::sleep(Duration::from_millis(50));
    }
    h
}

fn client(e: &Env) -> BackendClient {
    BackendClient::connect(&e.socket).unwrap()
}

fn add(c: &mut BackendClient, url: &str, dir: &Path, conns: u8, sha: Option<&str>) -> AddResult {
    c.call(Op::AddDownload(AddDownload {
        url: url.into(),
        filename: None,
        destination_dir: Some(dir.to_path_buf()),
        connections: Some(conns),
        expected_sha256: sha.map(str::to_string),
        source: Source::Interface,
        idempotency_key: None,
        suggested_filename: None,
        reserve_browser_filename: false,
    }))
    .unwrap()
}

fn task(c: &mut BackendClient, id: Uuid) -> TaskView {
    let s: Snapshot = c.call(Op::ListTasks).unwrap();
    s.tasks
        .into_iter()
        .find(|t| t.task.id == id)
        .expect("tarefa existe")
}

fn wait_for(
    c: &mut BackendClient,
    id: Uuid,
    secs: u64,
    pred: impl Fn(&TaskView) -> bool,
) -> TaskView {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let t = task(c, id);
        if pred(&t) {
            return t;
        }
        assert!(
            Instant::now() < deadline,
            "tempo esgotado; estado atual: {:?} {:?}",
            t.task.state,
            t.task.error_message
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn server() -> (
    tokio::runtime::Runtime,
    lyra_downloads_testserver::TestServer,
) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let s = rt.block_on(lyra_downloads_testserver::start()).unwrap();
    (rt, s)
}

macro_rules! require_aria2 {
    () => {
        if !aria2_available() {
            eprintln!("aria2c não encontrado: teste pulado (NÃO conta como aprovado)");
            return;
        }
    };
}

#[test]
fn firefox_cleanup_cannot_delete_handoff_payload() {
    require_aria2!();
    let e = env();
    let (_rt, srv) = server();
    let backend = start_backend(&e);
    let mut c = client(&e);
    let size = 256 * 1024;
    // pause() removed Firefox's placeholder before the handoff. Existing
    // numbered files must also remain untouched by the collision resolver.
    let original = e.downloads.join("browser.bin");
    let occupied = e.downloads.join("browser (1).bin");
    std::fs::write(&occupied, b"existing user file").unwrap();
    assert!(!original.exists());
    let request = AddDownload {
        url: srv.url(&format!("/slow/{size}/server.bin")),
        filename: None,
        destination_dir: Some(e.downloads.clone()),
        connections: Some(1),
        expected_sha256: None,
        source: Source::Navegador,
        idempotency_key: Some("firefox:cleanup-regression".into()),
        suggested_filename: Some("browser.bin".into()),
        reserve_browser_filename: true,
    };
    let first: AddResult = c.call(Op::AddDownload(request.clone())).unwrap();
    let again: AddResult = c.call(Op::AddDownload(request)).unwrap();
    assert!(again.duplicate);
    assert_eq!(first.task_id, again.task_id);
    wait_for(&mut c, first.task_id, 30, |t| t.task.downloaded_bytes > 0);
    // Firefox finalize(true) may unlink the original path at any time.
    let _ = std::fs::remove_file(&original);
    let finished = wait_for(&mut c, first.task_id, 30, |t| {
        t.task.state == TaskState::Concluido
    });
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
    backend.join().unwrap();
    assert_eq!(first.filename, "browser (2).bin");
    assert_eq!(finished.task.filename, first.filename);
    assert_eq!(
        std::fs::read(finished.task.final_path()).unwrap(),
        lyra_downloads_testserver::content(size)
    );
    assert_eq!(std::fs::read(occupied).unwrap(), b"existing user file");
    assert!(!original.exists());
}

#[test]
fn download_bytes_hash_and_parallel_ranges() {
    require_aria2!();
    let e = env();
    let (_rt, srv) = server();
    let _b = start_backend(&e);
    let mut c = client(&e);

    let size = 6 * 1024 * 1024;
    let expected = lyra_downloads_testserver::content(size);
    let good = sha256(&expected);

    let r = add(
        &mut c,
        &srv.url(&format!("/file/{size}/img.iso")),
        &e.downloads,
        4,
        Some(&good.to_uppercase()),
    );
    assert_eq!(r.filename, "img.iso");
    let t = wait_for(&mut c, r.task_id, 60, |t| {
        t.task.state == TaskState::Concluido
    });
    assert_eq!(t.task.hash_verification, HashVerification::Confere);
    assert_eq!(t.task.total_bytes, Some(size));
    assert_eq!(
        std::fs::read(e.downloads.join("img.iso")).unwrap(),
        expected
    );
    // Perfil 4 com arquivo de 6 MiB e min-split-size 1 MiB: o motor abre
    // várias requisições com Range (não medimos ganho de velocidade).
    assert!(
        srv.stats
            .range_requests
            .load(std::sync::atomic::Ordering::SeqCst)
            >= 2
    );

    // Mesmo nome de novo: não sobrescreve, ganha sufixo.
    let r2 = add(
        &mut c,
        &srv.url(&format!("/file/{size}/img.iso")),
        &e.downloads,
        1,
        Some(&"0".repeat(64)),
    );
    assert_eq!(r2.filename, "img (1).iso");
    let t2 = wait_for(&mut c, r2.task_id, 60, |t| {
        t.task.state == TaskState::Concluido
    });
    assert_eq!(t2.task.hash_verification, HashVerification::NaoConfere);
    assert!(
        e.downloads.join("img (1).iso").exists(),
        "arquivo divergente é mantido"
    );

    // Sem hash: concluído, não verificado.
    let r3 = add(&mut c, &srv.url("/cd/1000"), &e.downloads, 1, None);
    let t3 = wait_for(&mut c, r3.task_id, 30, |t| {
        t.task.state == TaskState::Concluido
    });
    assert_eq!(t3.task.hash_verification, HashVerification::NaoVerificado);

    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
}

#[test]
fn unknown_length_no_ranges_and_redirect() {
    require_aria2!();
    let e = env();
    let (_rt, srv) = server();
    let _b = start_backend(&e);
    let mut c = client(&e);

    let a = add(&mut c, &srv.url("/nolength/300000"), &e.downloads, 8, None);
    let b = add(
        &mut c,
        &srv.url("/noranges/2000000"),
        &e.downloads,
        16,
        None,
    );
    let r = add(
        &mut c,
        &srv.url("/redirect/file/12345"),
        &e.downloads,
        4,
        None,
    );
    for id in [a.task_id, b.task_id, r.task_id] {
        wait_for(&mut c, id, 60, |t| t.task.state == TaskState::Concluido);
    }
    assert_eq!(
        std::fs::read(e.downloads.join("300000")).unwrap(),
        lyra_downloads_testserver::content(300000)
    );
    assert_eq!(
        std::fs::read(e.downloads.join("2000000")).unwrap(),
        lyra_downloads_testserver::content(2000000)
    );
    assert_eq!(
        std::fs::read(e.downloads.join("12345")).unwrap(),
        lyra_downloads_testserver::content(12345)
    );
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
}

#[test]
fn http_errors_are_recoverable_and_described() {
    require_aria2!();
    let e = env();
    let (_rt, srv) = server();
    let _b = start_backend(&e);
    let mut c = client(&e);

    let r404 = add(&mut c, &srv.url("/status/404"), &e.downloads, 1, None);
    let r403 = add(&mut c, &srv.url("/status/403"), &e.downloads, 1, None);
    let refused = add(&mut c, "http://127.0.0.1:9/x.bin", &e.downloads, 1, None);

    let t = wait_for(&mut c, r404.task_id, 60, |t| {
        t.task.state == TaskState::Erro
    });
    assert!(t.task.error_message.unwrap().contains("não encontrado"));
    let t = wait_for(&mut c, r403.task_id, 60, |t| {
        t.task.state == TaskState::Erro
    });
    let msg = t.task.error_message.unwrap();
    assert!(msg.contains("403"), "{msg}");
    assert!(
        msg.contains("Pode ser"),
        "403 não deve ter diagnóstico definitivo: {msg}"
    );
    wait_for(&mut c, refused.task_id, 90, |t| {
        t.task.state == TaskState::Erro
    });

    // Tentar de novo é possível (volta à fila e falha de novo, sem loop infinito).
    let _: serde_json::Value = c.call(Op::Retry { id: r404.task_id }).unwrap();
    wait_for(&mut c, r404.task_id, 60, |t| {
        t.task.state == TaskState::Erro
    });

    // Novo link: nova tarefa, sem reaproveitar parciais.
    let n: AddResult = c
        .call(Op::RetryWithNewUrl {
            id: r404.task_id,
            url: srv.url("/file/1000/x"),
        })
        .unwrap();
    assert_ne!(n.task_id, r404.task_id);
    wait_for(&mut c, n.task_id, 30, |t| {
        t.task.state == TaskState::Concluido
    });

    // Destino inválido.
    let bad = c.call::<AddResult>(Op::AddDownload(AddDownload {
        url: srv.url("/file/10"),
        filename: None,
        destination_dir: Some(PathBuf::from("/nao/existe")),
        connections: None,
        expected_sha256: None,
        source: Source::Interface,
        idempotency_key: None,
        suggested_filename: None,
        reserve_browser_filename: false,
    }));
    assert!(matches!(
        bad,
        Err(lyra_downloads_ipc::client::ClientError::Backend {
            code: ErrorCode::InvalidDestination,
            ..
        })
    ));

    // Esquema não suportado.
    let bad = c.call::<AddResult>(Op::AddDownload(AddDownload {
        url: "file:///etc/passwd".into(),
        filename: None,
        destination_dir: None,
        connections: None,
        expected_sha256: None,
        source: Source::Navegador,
        idempotency_key: None,
        suggested_filename: None,
        reserve_browser_filename: false,
    }));
    assert!(matches!(
        bad,
        Err(lyra_downloads_ipc::client::ClientError::Backend {
            code: ErrorCode::InvalidUrl,
            ..
        })
    ));
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
}

#[test]
fn idempotent_browser_requests() {
    require_aria2!();
    let e = env();
    let (_rt, srv) = server();
    let _b = start_backend(&e);
    let mut c = client(&e);

    let req = |key: &str| {
        Op::AddDownload(AddDownload {
            url: srv.url("/file/2000/same.bin"),
            filename: None,
            destination_dir: Some(e.downloads.clone()),
            connections: None,
            expected_sha256: None,
            source: Source::Navegador,
            idempotency_key: Some(key.into()),
            suggested_filename: None,
            reserve_browser_filename: false,
        })
    };
    let a: AddResult = c.call(req("ext-1")).unwrap();
    let a2: AddResult = c.call(req("ext-1")).unwrap();
    assert_eq!(a.task_id, a2.task_id);
    assert!(a2.duplicate);
    let b: AddResult = c.call(req("ext-2")).unwrap();
    assert_ne!(
        a.task_id, b.task_id,
        "download intencional posterior da mesma URL é permitido"
    );
    let s: Snapshot = c.call(Op::ListTasks).unwrap();
    assert_eq!(s.tasks.len(), 2);
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
}

#[test]
fn browser_cancellation_blocks_late_requests_and_survives_restart() {
    require_aria2!();
    let e = env();
    let (_rt, srv) = server();
    let backend = start_backend(&e);
    let mut c = client(&e);
    let request = |key: &str, route: &str| {
        Op::AddDownload(AddDownload {
            url: srv.url(route),
            filename: None,
            destination_dir: Some(e.downloads.clone()),
            connections: None,
            expected_sha256: None,
            source: Source::Navegador,
            idempotency_key: Some(key.into()),
            suggested_filename: None,
            reserve_browser_filename: false,
        })
    };
    let cancel = |key: &str| Op::CancelByRequestKey { key: key.into() };
    let revoked: serde_json::Value = c.call(cancel("firefox:late")).unwrap();
    assert_eq!(revoked["revoked"], true);
    assert_eq!(revoked["found"], false);
    assert!(c
        .call::<AddResult>(request("firefox:late", "/file/1000/late.bin"))
        .is_err());
    assert!(c.call::<Snapshot>(Op::ListTasks).unwrap().tasks.is_empty());

    let active: AddResult = c
        .call(request("firefox:active", "/slow/16777216/active.bin"))
        .unwrap();
    wait_for(&mut c, active.task_id, 30, |t| t.task.downloaded_bytes > 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match c.call::<serde_json::Value>(cancel("firefox:active")) {
            Ok(result) => {
                assert_eq!(result["revoked"], true);
                assert_eq!(result["cancelled"], true);
                assert_eq!(result["completed"], false);
                break;
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("cancelamento não foi confirmado: {e}"),
        }
    }
    assert_eq!(
        task(&mut c, active.task_id).task.state,
        TaskState::Cancelado
    );
    assert!(c
        .call::<AddResult>(request("firefox:active", "/slow/16777216/active.bin"))
        .is_err());

    let completed: AddResult = c
        .call(request("firefox:completed", "/file/1000/done.bin"))
        .unwrap();
    wait_for(&mut c, completed.task_id, 30, |t| {
        t.task.state == TaskState::Concluido
    });
    let result: serde_json::Value = c.call(cancel("firefox:completed")).unwrap();
    assert_eq!(result["completed"], true);
    assert_eq!(
        task(&mut c, completed.task_id).task.state,
        TaskState::Concluido
    );
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
    drop(c);
    backend.join().unwrap();
    let backend = start_backend(&e);
    let mut c = client(&e);
    assert!(c
        .call::<AddResult>(request("firefox:late", "/file/1000/late.bin"))
        .is_err());
    assert_eq!(
        task(&mut c, active.task_id).task.state,
        TaskState::Cancelado
    );
    let unrelated: AddResult = c
        .call(request("firefox:new", "/file/1000/late.bin"))
        .unwrap();
    wait_for(&mut c, unrelated.task_id, 30, |t| {
        t.task.state == TaskState::Concluido
    });
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
    backend.join().unwrap();
}

#[test]
fn pause_resume_and_restart_recovery_without_duplicates() {
    require_aria2!();
    let e = env();
    let (_rt, srv) = server();
    let size: u64 = 8 * 1024 * 1024;
    let b = start_backend(&e);
    let mut c = client(&e);

    let r = add(
        &mut c,
        &srv.url(&format!("/slow/{size}/big.bin")),
        &e.downloads,
        4,
        None,
    );
    wait_for(&mut c, r.task_id, 30, |t| {
        t.task.state == TaskState::Baixando && t.task.downloaded_bytes > 0
    });

    let _: serde_json::Value = c.call(Op::Pause { id: r.task_id }).unwrap();
    let paused = wait_for(&mut c, r.task_id, 10, |t| {
        t.task.state == TaskState::Pausado
    });
    assert_eq!(
        paused.task.pause_reason,
        Some(lyra_downloads_core::PauseReason::Usuario)
    );
    let _: serde_json::Value = c.call(Op::Resume { id: r.task_id }).unwrap();
    wait_for(&mut c, r.task_id, 30, |t| {
        t.task.state == TaskState::Baixando
    });

    // Encerra o backend no meio (pausa tudo pelo sistema) e reinicia.
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
    b.join().unwrap();
    assert!(
        e.downloads.join("big.bin.aria2").exists(),
        "arquivo de controle preservado"
    );

    let b2 = start_backend(&e);
    let mut c = client(&e);
    let s: Snapshot = c.call(Op::ListTasks).unwrap();
    assert_eq!(s.tasks.len(), 1, "nenhuma tarefa duplicada após reinício");
    let t = wait_for(&mut c, r.task_id, 120, |t| {
        t.task.state == TaskState::Concluido
    });
    assert_eq!(t.task.total_bytes, Some(size));
    assert_eq!(
        std::fs::read(e.downloads.join("big.bin")).unwrap(),
        lyra_downloads_testserver::content(size)
    );
    assert!(!e.downloads.join("big.bin.aria2").exists());
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
    b2.join().unwrap();
}

#[test]
fn user_pause_survives_restart() {
    require_aria2!();
    let e = env();
    let (_rt, srv) = server();
    let b = start_backend(&e);
    let mut c = client(&e);
    let r = add(
        &mut c,
        &srv.url("/slow/8388608/p.bin"),
        &e.downloads,
        1,
        None,
    );
    wait_for(&mut c, r.task_id, 30, |t| t.task.downloaded_bytes > 0);
    let _: serde_json::Value = c.call(Op::Pause { id: r.task_id }).unwrap();
    wait_for(&mut c, r.task_id, 10, |t| {
        t.task.state == TaskState::Pausado
    });
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
    b.join().unwrap();

    let b2 = start_backend(&e);
    let mut c = client(&e);
    std::thread::sleep(Duration::from_secs(3));
    let t = task(&mut c, r.task_id);
    assert_eq!(
        t.task.state,
        TaskState::Pausado,
        "pausa do usuário é respeitada"
    );
    assert_eq!(t.download_speed, 0);
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
    b2.join().unwrap();
}

#[test]
fn cancel_keeps_partial_delete_is_explicit() {
    require_aria2!();
    let e = env();
    let (_rt, srv) = server();
    let _b = start_backend(&e);
    let mut c = client(&e);
    let r = add(
        &mut c,
        &srv.url("/slow/8388608/c.bin"),
        &e.downloads,
        1,
        None,
    );
    wait_for(&mut c, r.task_id, 30, |t| t.task.downloaded_bytes > 0);
    let _: serde_json::Value = c.call(Op::Cancel { id: r.task_id }).unwrap();
    wait_for(&mut c, r.task_id, 10, |t| {
        t.task.state == TaskState::Cancelado
    });
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        e.downloads.join("c.bin").exists(),
        "cancelar não apaga o parcial"
    );

    // Remover do histórico não apaga o arquivo.
    let _: serde_json::Value = c.call(Op::RemoveFromHistory { id: r.task_id }).unwrap();
    assert!(e.downloads.join("c.bin").exists());

    // Excluir arquivo é explícito.
    let r2 = add(&mut c, &srv.url("/file/5000/d.bin"), &e.downloads, 1, None);
    wait_for(&mut c, r2.task_id, 30, |t| {
        t.task.state == TaskState::Concluido
    });
    let _: serde_json::Value = c.call(Op::DeleteFile { id: r2.task_id }).unwrap();
    assert!(!e.downloads.join("d.bin").exists());
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
}

#[test]
fn invalid_and_oversized_messages_are_rejected() {
    require_aria2!();
    use std::io::{BufRead, BufReader, Write};
    let e = env();
    let _b = start_backend(&e);
    let mut s = std::os::unix::net::UnixStream::connect(&e.socket).unwrap();
    s.write_all(b"{nao e json}\n").unwrap();
    let mut line = String::new();
    BufReader::new(s.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.contains("invalid_request"));
    s.write_all(
        b"{\"v\":1,\"request_id\":\"x\",\"op\":\"aria2_raw\",\"method\":\"aria2.shutdown\"}\n",
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(s.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(
        line.contains("invalid_request"),
        "operações fora da lista são recusadas"
    );
    s.write_all(b"{\"v\":99,\"request_id\":\"y\",\"op\":\"health\"}\n")
        .unwrap();
    let mut line = String::new();
    BufReader::new(s.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.contains("unsupported_version"));
    let mut c = client(&e);
    let _: serde_json::Value = c.call(Op::Shutdown).unwrap();
}
