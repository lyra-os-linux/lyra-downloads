mod add_dialog;
mod backend;
mod format;
mod i18n;
mod prefs;
mod task_row;
mod window;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "org.lyraos.Downloads";

/// Argumentos aceitos (também repassados à instância primária pelo GApplication):
/// `lyra-downloads [--add-url URL [--suggested-name NOME] [--request-id ID]]`
#[derive(Default)]
struct Args {
    add_url: Option<String>,
    suggested_name: Option<String>,
    request_id: Option<String>,
}

fn parse_args(argv: &[std::ffi::OsString]) -> Args {
    let mut a = Args::default();
    let mut it = argv.iter().skip(1).map(|s| s.to_string_lossy().to_string());
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--add-url" => a.add_url = it.next(),
            "--suggested-name" => a.suggested_name = it.next(),
            "--request-id" => a.request_id = it.next(),
            _ => {}
        }
    }
    a
}

/// Diagnóstico de desenvolvimento: renderiza a janela em PNG após alguns
/// segundos (útil onde o compositor não permite capturas de tela).
fn render_later(w: gtk::Widget, path: String) {
    glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
        let paintable = gtk::WidgetPaintable::new(Some(&w));
        let snap = gtk::Snapshot::new();
        paintable.snapshot(&snap, w.width() as f64, w.height() as f64);
        let rendered = snap
            .to_node()
            .zip(w.native().and_then(|n| n.renderer()))
            .map(|(node, r)| r.render_texture(&node, None));
        match rendered.map(|t| t.save_to_png(&path)) {
            Some(Ok(())) => eprintln!("janela renderizada em {path}"),
            other => eprintln!("falha ao renderizar janela: {other:?}"),
        }
    });
}

fn main() -> glib::ExitCode {
    i18n::init();
    let filter = tracing_subscriber::EnvFilter::try_from_env("LYRA_DOWNLOADS_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let ui: Rc<RefCell<Option<Rc<window::Ui>>>> = Rc::new(RefCell::new(None));
    let ensure_window = {
        let ui = ui.clone();
        move |app: &adw::Application| -> Rc<window::Ui> {
            let existing = ui.borrow().clone();
            match existing {
                Some(u) if u.window.is_visible() || u.window.application().is_some() => {
                    u.window.present();
                    u
                }
                _ => {
                    let u = window::build(app);
                    if let Ok(path) = std::env::var("LYRA_DOWNLOADS_RENDER_TO") {
                        render_later(u.window.clone().upcast(), path);
                    }
                    *ui.borrow_mut() = Some(u.clone());
                    u
                }
            }
        }
    };

    app.connect_startup(|_| gtk::Window::set_default_icon_name(APP_ID));
    app.connect_activate({
        let ensure_window = ensure_window.clone();
        move |app| {
            ensure_window(app);
        }
    });
    // IDs de requisições do navegador já apresentadas: um reenvio não abre
    // um segundo diálogo (o backend também é idempotente pela mesma chave).
    let seen_requests: Rc<RefCell<std::collections::HashSet<String>>> = Rc::default();
    app.connect_command_line(move |app, cmd| {
        let args = parse_args(&cmd.arguments());
        let u = ensure_window(app);
        if let Some(id) = &args.request_id {
            if !seen_requests.borrow_mut().insert(id.clone()) {
                return glib::ExitCode::SUCCESS;
            }
        }
        if let Some(url) = args.add_url {
            u.open_add_dialog(add_dialog::Prefill {
                url: Some(url),
                suggested_filename: args.suggested_name,
                idempotency_key: args.request_id.map(|id| format!("firefox:{id}")),
                from_browser: true,
            });
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}
