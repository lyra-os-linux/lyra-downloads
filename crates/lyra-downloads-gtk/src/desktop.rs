//! Integração com o GNOME: abrir arquivo ou pasta e seguir o esquema de cor
//! e a cor de destaque do sistema.

use gio::prelude::*;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

fn file_uri(path: &str) -> String {
    gio::File::for_path(path).uri().to_string()
}

/// Abre o arquivo no aplicativo padrão.
#[tauri::command]
pub async fn open_file(path: String) -> Result<(), String> {
    let uri = file_uri(&path);
    tauri::async_runtime::spawn_blocking(move || {
        gio::AppInfo::launch_default_for_uri(&uri, Some(&crate::webkit_env::launch_context()))
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Mostra o item na pasta (org.freedesktop.FileManager1, como o GTK faz);
/// sem gerenciador de arquivos no barramento, abre a pasta.
#[tauri::command]
pub async fn open_folder(path: String, dir: String) -> Result<(), String> {
    let (item, folder) = (file_uri(&path), file_uri(&dir));
    tauri::async_runtime::spawn_blocking(move || {
        let shown =
            gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).and_then(|bus| {
                bus.call_sync(
                    Some("org.freedesktop.FileManager1"),
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1",
                    "ShowItems",
                    Some(&(vec![item.as_str()], "").to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    5000,
                    gio::Cancellable::NONE,
                )
            });
        if shown.is_ok() {
            return Ok(());
        }
        gio::AppInfo::launch_default_for_uri(&folder, Some(&crate::webkit_env::launch_context()))
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn interface_settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    source.lookup("org.gnome.desktop.interface", true)?;
    Some(gio::Settings::new("org.gnome.desktop.interface"))
}

fn interface_state(settings: &gio::Settings) -> Value {
    let has_accent = settings
        .settings_schema()
        .is_some_and(|schema| schema.has_key("accent-color"));
    json!({
        "color_scheme": settings.string("color-scheme").to_string(),
        "accent": if has_accent { settings.string("accent-color").to_string() } else { "blue".to_owned() },
    })
}

#[tauri::command]
pub fn interface_style() -> Value {
    interface_settings()
        .map(|settings| interface_state(&settings))
        .unwrap_or_else(|| json!({ "color_scheme": "default", "accent": "blue" }))
}

/// Avisa a interface quando o tema ou o destaque do GNOME mudam.
pub fn watch_interface(app: AppHandle) {
    let Some(settings) = interface_settings() else {
        return;
    };
    settings.connect_changed(None, move |settings, key| {
        if key == "color-scheme" || key == "accent-color" {
            let _ = app.emit("interface-changed", interface_state(settings));
        }
    });
    // GSettings só emite "changed" para chaves já lidas.
    let _ = interface_state(&settings);
    // A assinatura vive enquanto o processo viver.
    std::mem::forget(settings);
}
