//! Diálogo "Novo download". Cancelar não cria nenhuma tarefa.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use lyra_downloads_core::settings::Settings;
use lyra_downloads_ipc::{AddDownload, AddResult, Op, Source};
use serde::Deserialize;

use crate::backend;
use crate::format;
use crate::i18n::{tr, trf};

#[derive(Default, Clone)]
pub struct Prefill {
    pub url: Option<String>,
    pub suggested_filename: Option<String>,
    /// Chave de idempotência vinda do navegador (envio manual).
    pub idempotency_key: Option<String>,
    pub from_browser: bool,
}

#[derive(Deserialize)]
struct ProbeResult {
    filename: String,
    size: Option<u64>,
    accepts_ranges: Option<bool>,
}

const PROFILES: [u8; 4] = [1, 4, 8, 16];

pub fn present(
    parent: &adw::ApplicationWindow,
    settings: &Settings,
    prefill: Prefill,
    on_added: impl Fn(String) + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title(tr("Novo download"))
        .content_width(560)
        .follows_content_size(true)
        .build();

    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    let cancel = gtk::Button::with_label(&tr("Cancelar"));
    let start = gtk::Button::builder()
        .label(tr("Baixar"))
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&start);

    let page = adw::PreferencesPage::new();

    // --- Endereço ------------------------------------------------------
    let url_group = adw::PreferencesGroup::new();
    let batch_row = adw::SwitchRow::builder()
        .title(tr("Vários links"))
        .subtitle(tr("Um endereço por linha, com as mesmas opções"))
        .build();
    let url_row = adw::EntryRow::builder()
        .title(tr("Endereço (http:// ou https://)"))
        .build();
    url_row.set_input_purpose(gtk::InputPurpose::Url);
    let batch_view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .monospace(true)
        .build();
    batch_view.update_property(&[gtk::accessible::Property::Label(&tr(
        "Endereços, um por linha",
    ))]);
    let batch_scroller = gtk::ScrolledWindow::builder()
        .child(&batch_view)
        .min_content_height(120)
        .css_classes(["card"])
        .visible(false)
        .build();
    let name_row = adw::EntryRow::builder()
        .title(tr("Nome do arquivo"))
        .build();
    let probe_label = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .margin_top(6)
        .visible(false)
        .build();
    url_group.add(&batch_row);
    url_group.add(&url_row);
    url_group.add(&batch_scroller);
    url_group.add(&name_row);
    url_group.add(&probe_label);

    // --- Opções --------------------------------------------------------
    let opt_group = adw::PreferencesGroup::builder().title(tr("Opções")).build();
    let folder: Rc<RefCell<PathBuf>> =
        Rc::new(RefCell::new(settings.default_destination_dir.clone()));
    let folder_row = adw::ActionRow::builder()
        .title(tr("Pasta"))
        .subtitle(folder.borrow().to_string_lossy().to_string())
        .subtitle_selectable(true)
        .build();
    let folder_btn = gtk::Button::builder()
        .label(tr("Escolher…"))
        .valign(gtk::Align::Center)
        .build();
    folder_row.add_suffix(&folder_btn);

    let conn_model = gtk::StringList::new(&["1", "4", "8", "16"]);
    let conn_row = adw::ComboRow::builder()
        .title(tr("Conexões"))
        .subtitle(tr("Máximo solicitado ao servidor; ele pode aceitar menos"))
        .model(&conn_model)
        .build();
    let default_idx = PROFILES
        .iter()
        .position(|p| *p == settings.default_connections.as_u8())
        .unwrap_or(1);
    conn_row.set_selected(default_idx as u32);

    let sha_row = adw::EntryRow::builder()
        .title(tr("SHA-256 esperado (opcional)"))
        .build();
    let sha_hint = gtk::Label::builder()
        .label(tr("A conferência só indica que o arquivo corresponde ao hash informado; a confiança depende de onde o hash veio."))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .margin_top(6)
        .build();
    opt_group.add(&folder_row);
    opt_group.add(&conn_row);
    opt_group.add(&sha_row);
    opt_group.add(&sha_hint);

    let error_label = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["error"])
        .visible(false)
        .build();
    let error_group = adw::PreferencesGroup::new();
    error_group.add(&error_label);

    page.add(&url_group);
    page.add(&opt_group);
    page.add(&error_group);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    // --- Comportamento -------------------------------------------------
    let name_edited = Rc::new(Cell::new(false));
    let probe_gen = Rc::new(Cell::new(0u64));
    let validate = {
        let url_row = url_row.clone();
        let batch_row = batch_row.clone();
        let batch_view = batch_view.clone();
        let start = start.clone();
        move || {
            let ok = if batch_row.is_active() {
                let buf = batch_view.buffer();
                let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
                text.lines().any(|l| looks_like_url(l.trim()))
            } else {
                looks_like_url(url_row.text().trim())
            };
            start.set_sensitive(ok);
        }
    };
    let validate = Rc::new(validate);

    batch_row.connect_active_notify({
        let (url_row, name_row, batch_scroller, probe_label, validate) = (
            url_row.clone(),
            name_row.clone(),
            batch_scroller.clone(),
            probe_label.clone(),
            validate.clone(),
        );
        move |r| {
            let batch = r.is_active();
            url_row.set_visible(!batch);
            name_row.set_visible(!batch);
            probe_label.set_visible(false);
            batch_scroller.set_visible(batch);
            validate();
        }
    });
    batch_view.buffer().connect_changed({
        let validate = validate.clone();
        move |_| validate()
    });

    name_row.connect_changed({
        let name_edited = name_edited.clone();
        move |r| {
            if r.has_focus() || r.focus_child().is_some() {
                name_edited.set(true);
            }
        }
    });

    url_row.connect_changed({
        let (validate, probe_gen, name_row, name_edited, probe_label) =
            (validate.clone(), probe_gen.clone(), name_row.clone(), name_edited.clone(), probe_label.clone());
        move |r| {
            validate();
            let url = r.text().trim().to_string();
            let generation = probe_gen.get() + 1;
            probe_gen.set(generation);
            if !looks_like_url(&url) {
                probe_label.set_visible(false);
                return;
            }
            let (probe_gen, name_row, name_edited, probe_label) =
                (probe_gen.clone(), name_row.clone(), name_edited.clone(), probe_label.clone());
            glib::timeout_add_local_once(Duration::from_millis(600), move || {
                if probe_gen.get() != generation {
                    return;
                }
                probe_label.set_label(&tr("Consultando o servidor…"));
                probe_label.set_visible(true);
                glib::spawn_future_local(async move {
                    let res = backend::call::<ProbeResult>(Op::Probe { url }).await;
                    if probe_gen.get() != generation {
                        return;
                    }
                    match res {
                        Ok(p) => {
                            if !name_edited.get() {
                                name_row.set_text(&p.filename);
                            }
                            let mut info = match p.size {
                                Some(s) => trf("Tamanho informado pelo servidor: {s}", &[("s", &format::bytes(s))]),
                                None => tr("O servidor não informou o tamanho."),
                            };
                            if p.accepts_ranges == Some(false) {
                                info.push(' ');
                                info.push_str(&tr("Ele não aceita transferência em partes: apenas uma conexão será usada."));
                            }
                            probe_label.set_label(&info);
                        }
                        Err(_) => probe_label.set_visible(false),
                    }
                });
            });
        }
    });

    folder_btn.connect_clicked({
        let (folder, folder_row, dialog) = (folder.clone(), folder_row.clone(), dialog.clone());
        move |_| {
            let fd = gtk::FileDialog::builder()
                .title(tr("Escolher pasta de destino"))
                .modal(true)
                .build();
            fd.set_initial_folder(Some(&gio::File::for_path(&*folder.borrow())));
            let root = dialog.root().and_downcast::<gtk::Window>();
            let (folder, folder_row) = (folder.clone(), folder_row.clone());
            fd.select_folder(root.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(f) = res {
                    if let Some(p) = f.path() {
                        folder_row.set_subtitle(&p.to_string_lossy());
                        *folder.borrow_mut() = p;
                    }
                }
            });
        }
    });

    cancel.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });

    if let Some(url) = &prefill.url {
        url_row.set_text(url);
    }
    if let Some(name) = &prefill.suggested_filename {
        name_row.set_text(name);
        name_edited.set(true);
    }

    let on_added = Rc::new(on_added);
    start.connect_clicked({
        let dialog = dialog.clone();
        let prefill = prefill.clone();
        let url_row = url_row.clone();
        move |start| {
            error_label.set_visible(false);
            let connections = PROFILES[conn_row.selected().min(3) as usize];
            let sha = sha_row.text().trim().to_string();
            if !sha.is_empty() && !(sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit())) {
                show_error(
                    &error_label,
                    &tr("O SHA-256 esperado deve ter 64 dígitos hexadecimais."),
                );
                return;
            }
            let urls: Vec<String> = if batch_row.is_active() {
                let buf = batch_view.buffer();
                buf.text(&buf.start_iter(), &buf.end_iter(), false)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            } else {
                vec![url_row.text().trim().to_string()]
            };
            let filename = (!batch_row.is_active())
                .then(|| name_row.text().trim().to_string())
                .filter(|s| !s.is_empty());
            let dir = folder.borrow().clone();
            let sha = (!sha.is_empty()).then_some(sha);
            let source = if prefill.from_browser {
                Source::Navegador
            } else {
                Source::Interface
            };
            let key = prefill.idempotency_key.clone();

            start.set_sensitive(false);
            let (dialog, error_label, start, on_added) = (
                dialog.clone(),
                error_label.clone(),
                start.clone(),
                on_added.clone(),
            );
            glib::spawn_future_local(async move {
                let mut failures = Vec::new();
                let mut added = Vec::new();
                for (i, url) in urls.iter().enumerate() {
                    let req = AddDownload {
                        url: url.clone(),
                        filename: filename.clone(),
                        destination_dir: Some(dir.clone()),
                        connections: Some(connections),
                        expected_sha256: sha.clone(),
                        source,
                        idempotency_key: if urls.len() == 1 {
                            key.clone()
                        } else {
                            key.as_ref().map(|k| format!("{k}-{i}"))
                        },
                        suggested_filename: None,
                    };
                    match backend::call::<AddResult>(Op::AddDownload(req)).await {
                        Ok(r) => added.push(r.filename),
                        Err(e) => failures.push(format!("{url}: {e}")),
                    }
                }
                if failures.is_empty() {
                    let msg = if added.len() == 1 {
                        trf("Download adicionado: {name}", &[("name", &added[0])])
                    } else {
                        trf(
                            "{n} downloads adicionados",
                            &[("n", &added.len().to_string())],
                        )
                    };
                    on_added(msg);
                    dialog.close();
                } else {
                    if !added.is_empty() {
                        on_added(trf(
                            "{n} downloads adicionados",
                            &[("n", &added.len().to_string())],
                        ));
                    }
                    show_error(&error_label, &failures.join("\n"));
                    start.set_sensitive(true);
                }
            });
        }
    });

    dialog.present(Some(parent));
    url_row.grab_focus();
}

fn show_error(label: &gtk::Label, msg: &str) {
    label.set_label(msg);
    label.set_visible(true);
}

pub fn looks_like_url(s: &str) -> bool {
    lyra_downloads_core::validate_download_url(s)
        .map(|u| u.host_str().is_some())
        .unwrap_or(false)
}
