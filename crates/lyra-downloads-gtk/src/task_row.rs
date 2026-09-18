//! Linha de uma transferência na lista. Os widgets são criados uma vez e
//! atualizados in-place a cada ciclo (sem piscar, preservando foco).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use lyra_downloads_core::{HashVerification, TaskState};
use lyra_downloads_ipc::TaskView;
use uuid::Uuid;

use crate::format;
use crate::i18n::{tr, trf};

pub struct TaskRow {
    pub row: gtk::ListBoxRow,
    pub id: Uuid,
    icon: gtk::Image,
    name: gtk::Label,
    status: gtk::Label,
    badge: gtk::Label,
    progress: gtk::ProgressBar,
    pause_btn: gtk::Button,
    resume_btn: gtk::Button,
    cancel_btn: gtk::Button,
    retry_btn: gtk::Button,
    folder_btn: gtk::Button,
    menu_btn: gtk::MenuButton,
    pub current: Rc<RefCell<TaskView>>,
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tooltip));
    b.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    b.add_css_class("flat");
    b.add_css_class("circular");
    b.set_valign(gtk::Align::Center);
    b
}

pub fn state_label(s: TaskState) -> String {
    match s {
        TaskState::Aguardando => tr("Aguardando"),
        TaskState::Baixando => tr("Baixando"),
        TaskState::Pausado => tr("Pausado"),
        TaskState::Verificando => tr("Verificando SHA-256"),
        TaskState::Concluido => tr("Concluído"),
        TaskState::Cancelado => tr("Cancelado"),
        TaskState::Erro => tr("Erro"),
    }
}

fn state_icon(s: TaskState) -> &'static str {
    match s {
        TaskState::Aguardando => "content-loading-symbolic",
        TaskState::Baixando => "folder-download-symbolic",
        TaskState::Pausado => "media-playback-pause-symbolic",
        TaskState::Verificando => "system-search-symbolic",
        TaskState::Concluido => "object-select-symbolic",
        TaskState::Cancelado => "process-stop-symbolic",
        TaskState::Erro => "dialog-error-symbolic",
    }
}

impl TaskRow {
    pub fn new(view: &TaskView) -> Self {
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);
        outer.set_margin_top(10);
        outer.set_margin_bottom(10);
        outer.set_margin_start(12);
        outer.set_margin_end(8);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let icon = gtk::Image::new();
        icon.set_pixel_size(24);
        icon.set_valign(gtk::Align::Center);
        icon.add_css_class("dim-label");

        let texts = gtk::Box::new(gtk::Orientation::Vertical, 2);
        texts.set_hexpand(true);
        let name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["heading"])
            .build();
        let status = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .css_classes(["dim-label", "caption", "numeric"])
            .build();
        let badge = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["caption"])
            .visible(false)
            .build();
        texts.append(&name);
        texts.append(&status);
        texts.append(&badge);

        let pause_btn = icon_button("media-playback-pause-symbolic", &tr("Pausar"));
        let resume_btn = icon_button("media-playback-start-symbolic", &tr("Retomar"));
        let cancel_btn = icon_button("process-stop-symbolic", &tr("Cancelar transferência"));
        let retry_btn = icon_button("view-refresh-symbolic", &tr("Tentar novamente"));
        let folder_btn = icon_button("folder-open-symbolic", &tr("Abrir pasta"));
        let menu_btn = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(tr("Mais ações"))
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        menu_btn.update_property(&[gtk::accessible::Property::Label(&tr("Mais ações"))]);

        top.append(&icon);
        top.append(&texts);
        for b in [
            &pause_btn,
            &resume_btn,
            &retry_btn,
            &cancel_btn,
            &folder_btn,
        ] {
            top.append(b);
        }
        top.append(&menu_btn);

        let progress = gtk::ProgressBar::new();
        progress.set_margin_start(36);
        progress.set_margin_end(4);

        outer.append(&top);
        outer.append(&progress);

        let row = gtk::ListBoxRow::builder()
            .child(&outer)
            .activatable(true)
            .build();

        let r = Self {
            row,
            id: view.task.id,
            icon,
            name,
            status,
            badge,
            progress,
            pause_btn,
            resume_btn,
            cancel_btn,
            retry_btn,
            folder_btn,
            menu_btn,
            current: Rc::new(RefCell::new(view.clone())),
        };
        r.update(view);
        r
    }

    pub fn buttons(&self) -> RowButtons<'_> {
        RowButtons {
            pause: &self.pause_btn,
            resume: &self.resume_btn,
            cancel: &self.cancel_btn,
            retry: &self.retry_btn,
            folder: &self.folder_btn,
            menu: &self.menu_btn,
        }
    }

    pub fn update(&self, v: &TaskView) {
        *self.current.borrow_mut() = v.clone();
        let t = &v.task;
        self.name.set_label(&t.filename);
        self.name
            .set_tooltip_text(Some(&t.final_path().to_string_lossy()));
        self.icon.set_icon_name(Some(state_icon(t.state)));
        self.row
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "{}, {}",
                t.filename,
                state_label(t.state)
            ))]);

        // Linha de status com dados reais; o que é desconhecido é dito como tal.
        let mut parts = vec![state_label(t.state)];
        let size = match t.total_bytes {
            Some(total) if t.state != TaskState::Concluido => trf(
                "{a} de {b}",
                &[
                    ("a", &format::bytes(t.downloaded_bytes)),
                    ("b", &format::bytes(total)),
                ],
            ),
            Some(total) => format::bytes(total),
            None if t.downloaded_bytes > 0 => trf(
                "{a} (tamanho desconhecido)",
                &[("a", &format::bytes(t.downloaded_bytes))],
            ),
            None => String::new(),
        };
        if !size.is_empty() {
            parts.push(size);
        }
        if t.state == TaskState::Baixando {
            parts.push(format::speed(v.download_speed));
            parts.push(trf(
                "{n} de {max} conexões",
                &[
                    ("n", &v.active_connections.to_string()),
                    ("max", &t.connections.as_u8().to_string()),
                ],
            ));
            match format::eta(t.total_bytes, t.downloaded_bytes, v.download_speed) {
                Some(s) => parts.push(trf("faltam {t}", &[("t", &format::duration(s))])),
                None => parts.push(tr("tempo restante desconhecido")),
            }
        }
        if let Some(e) = t
            .error_message
            .as_deref()
            .filter(|_| t.state == TaskState::Erro)
        {
            parts.push(e.to_string());
        }
        self.status.set_label(&parts.join(" · "));

        // Selo de verificação (só para concluídos).
        self.badge.remove_css_class("success");
        self.badge.remove_css_class("error");
        self.badge.remove_css_class("dim-label");
        if t.state == TaskState::Concluido {
            let (text, class) = match t.hash_verification {
                HashVerification::Confere => {
                    (tr("SHA-256 confere com o valor informado"), "success")
                }
                HashVerification::NaoConfere => (
                    tr("SHA-256 não confere — arquivo mantido; verifique a origem"),
                    "error",
                ),
                HashVerification::NaoVerificado => (tr("Não verificado"), "dim-label"),
            };
            self.badge.set_label(&text);
            self.badge.add_css_class(class);
            self.badge.set_visible(true);
        } else {
            self.badge.set_visible(false);
        }

        match (t.state, t.progress_fraction()) {
            (TaskState::Concluido, _) => {
                self.progress.set_fraction(1.0);
                self.progress.set_visible(false);
            }
            (TaskState::Baixando, None) => {
                self.progress.set_visible(true);
                self.progress.pulse();
            }
            (_, Some(f)) => {
                self.progress.set_visible(true);
                self.progress.set_fraction(f.clamp(0.0, 1.0));
            }
            (_, None) => {
                self.progress.set_visible(true);
                self.progress.set_fraction(0.0);
            }
        }
        self.progress.remove_css_class("error");
        if t.state == TaskState::Erro {
            self.progress.add_css_class("error");
        }

        let s = t.state;
        self.pause_btn
            .set_visible(matches!(s, TaskState::Aguardando | TaskState::Baixando));
        self.resume_btn.set_visible(s == TaskState::Pausado);
        self.cancel_btn.set_visible(matches!(
            s,
            TaskState::Aguardando | TaskState::Baixando | TaskState::Pausado
        ));
        self.retry_btn
            .set_visible(matches!(s, TaskState::Erro | TaskState::Cancelado));
        self.folder_btn.set_visible(s == TaskState::Concluido);
    }
}

pub struct RowButtons<'a> {
    pub pause: &'a gtk::Button,
    pub resume: &'a gtk::Button,
    pub cancel: &'a gtk::Button,
    pub retry: &'a gtk::Button,
    pub folder: &'a gtk::Button,
    pub menu: &'a gtk::MenuButton,
}
