//! Preferências: pasta padrão, conexões, simultaneidade, limite de
//! velocidade, retomada e notificações. Salvas ao fechar o diálogo.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use lyra_downloads_core::settings::Settings;
use lyra_downloads_core::ConnectionProfile;
use lyra_downloads_ipc::Op;

use crate::backend;
use crate::i18n::tr;

const PROFILES: [u8; 4] = [1, 4, 8, 16];

pub fn present(
    parent: &adw::ApplicationWindow,
    current: Settings,
    on_saved: impl Fn(Result<Settings, String>) + 'static,
) {
    let s = Rc::new(RefCell::new(current));
    let dialog = adw::PreferencesDialog::builder()
        .title(tr("Preferences"))
        .build();
    let page = adw::PreferencesPage::new();

    let g = adw::PreferencesGroup::builder()
        .title(tr("Downloads"))
        .build();
    let folder_row = adw::ActionRow::builder()
        .title(tr("Default folder"))
        .subtitle(
            s.borrow()
                .default_destination_dir
                .to_string_lossy()
                .to_string(),
        )
        .build();
    let folder_btn = gtk::Button::builder()
        .label(tr("Choose…"))
        .valign(gtk::Align::Center)
        .build();
    folder_row.add_suffix(&folder_btn);
    let conn_row = adw::ComboRow::builder()
        .title(tr("Connections per download"))
        .subtitle(tr("Default for new downloads; a maximum, not a guarantee"))
        .model(&gtk::StringList::new(&["1", "4", "8", "16"]))
        .build();
    conn_row.set_selected(
        PROFILES
            .iter()
            .position(|p| *p == s.borrow().default_connections.as_u8())
            .unwrap_or(1) as u32,
    );
    let conc_row = adw::SpinRow::builder()
        .title(tr("Simultaneous downloads"))
        .subtitle(tr(
            "Independent of the number of connections for each download",
        ))
        .adjustment(&gtk::Adjustment::new(
            s.borrow().max_concurrent_downloads as f64,
            1.0,
            10.0,
            1.0,
            1.0,
            0.0,
        ))
        .build();
    let limit_row = adw::SpinRow::builder()
        .title(tr("Total speed limit (KiB/s)"))
        .subtitle(tr("0 = unlimited"))
        .adjustment(&gtk::Adjustment::new(
            (s.borrow().global_speed_limit_bytes / 1024) as f64,
            0.0,
            10_000_000.0,
            128.0,
            1024.0,
            0.0,
        ))
        .build();
    g.add(&folder_row);
    g.add(&conn_row);
    g.add(&conc_row);
    g.add(&limit_row);

    let g2 = adw::PreferencesGroup::builder()
        .title(tr("Behavior"))
        .build();
    let resume_row = adw::SwitchRow::builder()
        .title(tr("Resume downloads automatically"))
        .subtitle(tr(
            "When reopening after restarting the computer. Downloads you paused stay paused.",
        ))
        .active(s.borrow().auto_resume_on_start)
        .build();
    let notify_row = adw::SwitchRow::builder()
        .title(tr("Notify when finished"))
        .active(s.borrow().notifications_enabled)
        .build();
    g2.add(&resume_row);
    g2.add(&notify_row);

    page.add(&g);
    page.add(&g2);
    dialog.add(&page);

    folder_btn.connect_clicked({
        let (s, folder_row, dialog) = (s.clone(), folder_row.clone(), dialog.clone());
        move |_| {
            let fd = gtk::FileDialog::builder()
                .title(tr("Choose default folder"))
                .modal(true)
                .build();
            fd.set_initial_folder(Some(&gio::File::for_path(
                &s.borrow().default_destination_dir,
            )));
            let root = dialog.root().and_downcast::<gtk::Window>();
            let (s, folder_row) = (s.clone(), folder_row.clone());
            fd.select_folder(root.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(Some(p)) = res.map(|f| f.path()) {
                    folder_row.set_subtitle(&p.to_string_lossy());
                    s.borrow_mut().default_destination_dir = p;
                }
            });
        }
    });

    let on_saved = Rc::new(on_saved);
    dialog.connect_closed(move |_| {
        let mut new = s.borrow().clone();
        new.default_connections =
            ConnectionProfile::from_u8(PROFILES[conn_row.selected().min(3) as usize])
                .unwrap_or_default();
        new.max_concurrent_downloads = conc_row.value() as u32;
        new.global_speed_limit_bytes = limit_row.value() as u64 * 1024;
        new.auto_resume_on_start = resume_row.is_active();
        new.notifications_enabled = notify_row.is_active();
        let on_saved = on_saved.clone();
        glib::spawn_future_local(async move {
            let res = backend::call::<serde_json::Value>(Op::SetSettings {
                settings: new.clone(),
            })
            .await;
            on_saved(res.map(|_| new));
        });
    });
    dialog.present(Some(parent));
}
