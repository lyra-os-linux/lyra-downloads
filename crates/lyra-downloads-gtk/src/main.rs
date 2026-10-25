#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod desktop;
mod i18n;
mod launch;

use tauri::{Emitter, Manager};

pub const APP_ID: &str = "org.lyraos.Downloads";

fn main() {
    i18n::init();
    let filter = tracing_subscriber::EnvFilter::try_from_env("LYRA_DOWNLOADS_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    // O plugin de instância única cuida da unicidade; o prgname mantém o
    // app_id/WM_CLASS que liga a janela ao org.lyraos.Downloads.desktop.
    glib::set_prgname(Some(APP_ID));
    glib::set_application_name("Lyra Downloads");

    let first = launch::Request::parse(std::env::args());
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // Segunda execução (por exemplo, o Firefox): reapresenta a janela
            // e repassa o pedido de download, uma vez por requisição.
            let request = launch::Request::parse(argv.into_iter());
            if let Some(request) = app.state::<launch::Pending>().accept(request) {
                let _ = app.emit("add-url", request);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(launch::Pending::new(first))
        .setup(|app| {
            desktop::watch_interface(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend::backend_call,
            backend::quit_service,
            desktop::open_file,
            desktop::open_folder,
            desktop::interface_style,
            i18n::i18n_catalog,
            launch::app_info,
            launch::take_launch_request,
            launch::app_log,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Lyra Downloads");
}
