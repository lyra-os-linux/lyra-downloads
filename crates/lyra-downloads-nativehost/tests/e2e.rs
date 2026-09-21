//! Simula o Firefox: executa o binário real do host em um grupo de processos
//! próprio, com diretórios XDG isolados. O host inicia o backend sob demanda;
//! depois o grupo inteiro do host é morto (como o Firefox pode fazer) e o
//! download aceito precisa continuar até o fim.
//!
//! Requer os binários do workspace compilados (`cargo test --workspace` ou
//! `cargo build --workspace` antes) e aria2c instalado; caso contrário o
//! teste é pulado com aviso (e NÃO conta como aprovado).

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lyra_downloads_nativehost::{read_frame, write_frame, EXTENSION_ID};
use serde_json::{json, Value};

fn bin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lyra-downloads-nativehost"))
        .parent()
        .unwrap()
        .to_path_buf()
}

struct Iso {
    _tmp: tempfile::TempDir,
    home: PathBuf,
}

impl Iso {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        for d in ["run", "data", "config", "Downloads"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home.join("run"), std::fs::Permissions::from_mode(0o700)).unwrap();
        Iso { _tmp: tmp, home }
    }

    fn cmd(&self, program: &Path) -> Command {
        let mut c = Command::new(program);
        c.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("XDG_RUNTIME_DIR", self.home.join("run"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_DOWNLOAD_DIR", self.home.join("Downloads"));
        c
    }

    fn socket(&self) -> PathBuf {
        self.home.join("run/lyra-downloads/backend.sock")
    }
}

fn exchange(child_in: &mut impl Write, child_out: &mut impl Read, msg: Value) -> Value {
    write_frame(child_in, &msg).unwrap();
    serde_json::from_slice(&read_frame(child_out).unwrap()).unwrap()
}

fn backend_call(iso: &Iso, op: Value) -> Value {
    use std::io::{BufRead, BufReader};
    let mut s = std::os::unix::net::UnixStream::connect(iso.socket()).unwrap();
    let mut req = json!({"v":1,"request_id":"t"});
    req.as_object_mut()
        .unwrap()
        .extend(op.as_object().unwrap().clone());
    s.write_all(format!("{req}\n").as_bytes()).unwrap();
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn host_spawns_backend_and_download_survives_host_group_kill() {
    if lyra_downloads_aria2_available().is_none()
        || !bin_dir().join("lyra-downloads-backend").exists()
    {
        eprintln!(
            "aria2c ou lyra-downloads-backend ausente: teste pulado (NÃO conta como aprovado)"
        );
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let srv = rt.block_on(lyra_downloads_testserver::start()).unwrap();
    let iso = Iso::new();

    // Host em grupo de processos próprio, como um filho do Firefox.
    let mut host = iso
        .cmd(&bin_dir().join("lyra-downloads-nativehost"))
        .arg("/usr/lib64/mozilla/native-messaging-hosts/org.lyraos.downloads.json")
        .arg(EXTENSION_ID)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let pgid = host.id() as i32;
    let mut hin = host.stdin.take().unwrap();
    let mut hout = host.stdout.take().unwrap();

    let health = exchange(
        &mut hin,
        &mut hout,
        json!({"v":1,"request_id":"h1","op":"health"}),
    );
    assert_eq!(health["ok"], true, "{health}");
    assert_eq!(health["result"]["backend"]["engine_running"], true);

    let url = srv.url("/slow/6291456/via-firefox.iso");
    let resp = exchange(
        &mut hin,
        &mut hout,
        json!({"v":1,"request_id":"r1","op":"handoff","url":url}),
    );
    assert_eq!(resp["result"]["status"], "accepted", "{resp}");
    let task_id = resp["result"]["task"]["task_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Reenvio (ex.: resposta perdida) não duplica.
    let again = exchange(
        &mut hin,
        &mut hout,
        json!({"v":1,"request_id":"r1","op":"handoff","url":url}),
    );
    assert_eq!(again["result"]["task"]["task_id"], task_id.as_str());
    assert_eq!(again["result"]["task"]["duplicate"], true);

    // "Firefox" mata o grupo inteiro do host.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = host.wait();

    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let snap = backend_call(&iso, json!({"op":"list_tasks"}));
        let tasks = snap["result"]["tasks"].as_array().unwrap().clone();
        assert_eq!(tasks.len(), 1, "exatamente uma tarefa");
        if tasks[0]["state"] == "concluido" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "download não terminou: {}",
            tasks[0]
        );
        std::thread::sleep(Duration::from_millis(300));
    }
    let filename = resp["result"]["task"]["filename"].as_str().unwrap();
    assert_ne!(filename, "via-firefox.iso", "Firefox's path stays reserved");
    let file = iso.home.join("Downloads").join(filename);
    assert_eq!(
        std::fs::read(file).unwrap(),
        lyra_downloads_testserver::content(6291456)
    );

    // Host com ID de extensão desconhecido é recusado.
    let out = iso
        .cmd(&bin_dir().join("lyra-downloads-nativehost"))
        .arg("/m.json")
        .arg("outra@extensao")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());

    let _ = backend_call(&iso, json!({"op":"shutdown"}));
}

fn lyra_downloads_aria2_available() -> Option<()> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .any(|d| d.join("aria2c").is_file())
        .then_some(())
}

/// PIDs de um executável cujo ambiente aponta para este XDG_RUNTIME_DIR isolado.
fn pids_of(iso: &Iso, exe_name: &str) -> Vec<i32> {
    let marker = format!("XDG_RUNTIME_DIR={}", iso.home.join("run").display());
    let mut out = vec![];
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
        if exe
            .as_ref()
            .and_then(|p| p.file_name())
            .is_none_or(|n| n != exe_name)
        {
            continue;
        }
        let env = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
        if env.split(|b| *b == 0).any(|kv| kv == marker.as_bytes()) {
            out.push(pid);
        }
    }
    out
}

fn wait_until(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn host_health(iso: &Iso) -> Value {
    let mut host = iso
        .cmd(&bin_dir().join("lyra-downloads-nativehost"))
        .arg("/m.json")
        .arg(EXTENSION_ID)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut hin = host.stdin.take().unwrap();
    let mut hout = host.stdout.take().unwrap();
    let r = exchange(
        &mut hin,
        &mut hout,
        json!({"v":1,"request_id":"hh","op":"health"}),
    );
    drop(hin);
    let _ = host.wait();
    r
}

fn task_state(iso: &Iso) -> (usize, String, u64) {
    let snap = backend_call(iso, json!({"op":"list_tasks"}));
    let tasks = snap["result"]["tasks"].as_array().unwrap().clone();
    let t = &tasks[0];
    (
        tasks.len(),
        t["state"].as_str().unwrap_or("").to_string(),
        t["downloaded_bytes"].as_u64().unwrap_or(0),
    )
}

#[test]
fn crash_recovery_reaps_orphan_engine_and_finishes_without_duplicates() {
    if lyra_downloads_aria2_available().is_none()
        || !bin_dir().join("lyra-downloads-backend").exists()
    {
        eprintln!(
            "aria2c ou lyra-downloads-backend ausente: teste pulado (NÃO conta como aprovado)"
        );
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let srv = rt.block_on(lyra_downloads_testserver::start()).unwrap();
    let iso = Iso::new();
    let size: u64 = 16 * 1024 * 1024;

    let health = host_health(&iso);
    assert_eq!(health["ok"], true, "{health}");
    let _ = backend_call(
        &iso,
        json!({"op":"add_download","url":srv.url(&format!("/slow/{size}/crash.bin")),"connections":4,"source":"interface"}),
    );
    assert!(
        wait_until(30, || task_state(&iso).2 > 0),
        "download não começou"
    );

    // (a) O backend morre sem aviso; o aria2c dele fica órfão e vivo.
    let backend = pids_of(&iso, "lyra-downloads-backend");
    let engine = pids_of(&iso, "aria2c");
    assert_eq!((backend.len(), engine.len()), (1, 1));
    unsafe { libc::kill(backend[0], libc::SIGKILL) };
    assert!(wait_until(10, || pids_of(&iso, "lyra-downloads-backend").is_empty()));
    assert!(
        !pids_of(&iso, "aria2c").is_empty(),
        "aria2c órfão continua vivo"
    );

    // Novo backend (iniciado sob demanda) encerra o órfão verificado e segue.
    let health = host_health(&iso);
    assert_eq!(health["ok"], true, "{health}");
    assert!(
        wait_until(10, || !pids_of(&iso, "aria2c").contains(&engine[0])),
        "órfão não foi encerrado"
    );
    assert_eq!(pids_of(&iso, "aria2c").len(), 1, "exatamente um motor");
    let before = task_state(&iso).2;
    assert!(
        wait_until(30, || task_state(&iso).2 > before),
        "não retomou após a queda do backend"
    );

    // (b) Queda total (como um desligamento abrupto): backend e aria2c mortos.
    for pid in pids_of(&iso, "lyra-downloads-backend")
        .into_iter()
        .chain(pids_of(&iso, "aria2c"))
    {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert!(wait_until(10, || pids_of(&iso, "lyra-downloads-backend")
        .is_empty()
        && pids_of(&iso, "aria2c").is_empty()));
    let health = host_health(&iso);
    assert_eq!(health["ok"], true, "{health}");

    assert!(
        wait_until(120, || task_state(&iso).1 == "concluido"),
        "download não terminou após as quedas: {:?}",
        task_state(&iso)
    );
    let (count, _, _) = task_state(&iso);
    assert_eq!(count, 1, "nenhuma tarefa duplicada");
    assert_eq!(
        std::fs::read(iso.home.join("Downloads/crash.bin")).unwrap(),
        lyra_downloads_testserver::content(size),
        "bytes íntegros após as quedas"
    );
    let _ = backend_call(&iso, json!({"op":"shutdown"}));
}
