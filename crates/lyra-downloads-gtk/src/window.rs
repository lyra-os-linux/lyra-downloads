use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use lyra_downloads_core::settings::Settings;
use lyra_downloads_core::TaskState;
use lyra_downloads_ipc::{MoveDirection, Op, Snapshot, TaskView};
use uuid::Uuid;

use crate::add_dialog::{self, Prefill};
use crate::backend;
use crate::format;
use crate::i18n::{tr, trf};
use crate::task_row::{state_label, TaskRow};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Todos,
    EmAndamento,
    Concluidos,
    ComErro,
}

const PAGES: [Page; 4] = [
    Page::Todos,
    Page::EmAndamento,
    Page::Concluidos,
    Page::ComErro,
];

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Todos => "todos",
            Page::EmAndamento => "andamento",
            Page::Concluidos => "concluidos",
            Page::ComErro => "erro",
        }
    }
    fn title(self) -> String {
        match self {
            Page::Todos => tr("Todos"),
            Page::EmAndamento => tr("Em andamento"),
            Page::Concluidos => tr("Concluídos"),
            Page::ComErro => tr("Com erro"),
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Page::Todos => "view-list-symbolic",
            Page::EmAndamento => "folder-download-symbolic",
            Page::Concluidos => "object-select-symbolic",
            Page::ComErro => "dialog-warning-symbolic",
        }
    }
    fn accepts(self, s: TaskState) -> bool {
        match self {
            Page::Todos => true,
            Page::EmAndamento => matches!(
                s,
                TaskState::Aguardando
                    | TaskState::Baixando
                    | TaskState::Pausado
                    | TaskState::Verificando
            ),
            Page::Concluidos => s == TaskState::Concluido,
            Page::ComErro => s == TaskState::Erro,
        }
    }
    fn empty(self) -> (String, String) {
        match self {
            Page::Todos => (
                tr("Nenhum download"),
                tr("Use “Novo download” ou, no Firefox, “Baixar com Lyra Downloads” no menu de um link. Fechar a janela não interrompe os downloads."),
            ),
            Page::EmAndamento => (tr("Nada em andamento"), tr("Downloads aguardando, baixando ou pausados aparecem aqui.")),
            Page::Concluidos => (tr("Nenhum download concluído"), String::new()),
            Page::ComErro => (tr("Nenhum erro"), tr("Downloads que falharem aparecem aqui, com a opção de tentar novamente.")),
        }
    }
}

struct PageUi {
    page: Page,
    stack: gtk::Stack,
    list: gtk::ListBox,
    rows: HashMap<Uuid, TaskRow>,
    order: Vec<Uuid>,
    view_page: adw::ViewStackPage,
}

pub struct Ui {
    pub window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    banner: adw::Banner,
    summary: gtk::Label,
    pages: RefCell<Vec<PageUi>>,
    search: RefCell<String>,
    pub settings: RefCell<Settings>,
    backend_error: RefCell<Option<String>>,
}

impl Ui {
    pub fn toast(&self, msg: &str) {
        let t = adw::Toast::new(msg);
        t.set_timeout(4);
        self.toasts.add_toast(t);
    }

    pub fn open_add_dialog(self: &Rc<Self>, prefill: Prefill) {
        let ui = self.clone();
        let settings = self.settings.borrow().clone();
        add_dialog::present(&self.window, &settings, prefill, move |msg| {
            ui.toast(&msg);
            ui.refresh();
        });
    }

    fn error_toast(self: &Rc<Self>) -> impl Fn(String) + 'static {
        let ui = self.clone();
        move |e| ui.toast(&e)
    }

    pub fn refresh(self: &Rc<Self>) {
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match backend::call::<Snapshot>(Op::ListTasks).await {
                Ok(s) => {
                    *ui.backend_error.borrow_mut() = None;
                    ui.apply(s);
                }
                Err(e) => {
                    ui.banner
                        .set_title(&trf("Serviço de downloads indisponível: {e}", &[("e", &e)]));
                    ui.banner.set_button_label(None);
                    ui.banner.set_revealed(true);
                    *ui.backend_error.borrow_mut() = Some(e);
                }
            }
        });
    }

    fn apply(self: &Rc<Self>, s: Snapshot) {
        match &s.engine_problem {
            Some(p) => {
                self.banner.set_title(p);
                self.banner
                    .set_button_label(Some(&tr("Tentar iniciar o motor")));
                self.banner.set_revealed(true);
            }
            None => self.banner.set_revealed(false),
        }
        let active = s
            .tasks
            .iter()
            .filter(|t| t.task.state == TaskState::Baixando)
            .count();
        let waiting = s
            .tasks
            .iter()
            .filter(|t| t.task.state == TaskState::Aguardando)
            .count();
        let paused = s
            .tasks
            .iter()
            .filter(|t| t.task.state == TaskState::Pausado)
            .count();
        let mut summary = vec![trf(
            "Velocidade total: {v}",
            &[("v", &format::speed(s.total_speed))],
        )];
        summary.push(trf("{n} baixando", &[("n", &active.to_string())]));
        if waiting > 0 {
            summary.push(trf("{n} aguardando", &[("n", &waiting.to_string())]));
        }
        if paused > 0 {
            summary.push(trf("{n} pausados", &[("n", &paused.to_string())]));
        }
        self.summary.set_label(&summary.join(" · "));

        let query = self.search.borrow().to_lowercase();
        let mut pages = self.pages.borrow_mut();
        for p in pages.iter_mut() {
            let visible: Vec<&TaskView> = s
                .tasks
                .iter()
                .filter(|t| p.page.accepts(t.task.state))
                .filter(|t| {
                    query.is_empty()
                        || t.task.filename.to_lowercase().contains(&query)
                        || t.task.url.to_lowercase().contains(&query)
                })
                .collect();
            let ids: Vec<Uuid> = visible.iter().map(|t| t.task.id).collect();

            p.rows.retain(|id, row| {
                let keep = ids.contains(id);
                if !keep {
                    p.list.remove(&row.row);
                }
                keep
            });
            for t in &visible {
                match p.rows.get(&t.task.id) {
                    Some(row) => row.update(t),
                    None => {
                        let row = TaskRow::new(t);
                        self.wire_row(&row);
                        p.rows.insert(t.task.id, row);
                    }
                }
            }
            if p.order != ids {
                while let Some(child) = p.list.first_child() {
                    p.list.remove(&child);
                }
                for id in &ids {
                    p.list.append(&p.rows[id].row);
                }
                p.order = ids.clone();
            }
            let count = visible.len();
            p.stack
                .set_visible_child_name(if count == 0 { "empty" } else { "list" });
            if p.page == Page::ComErro {
                p.view_page.set_needs_attention(count > 0);
                p.view_page.set_badge_number(count as u32);
            }
        }
    }

    fn wire_row(self: &Rc<Self>, row: &TaskRow) {
        let id = row.id;
        let b = row.buttons();
        let on_err = Rc::new(self.error_toast());
        let op_btn = |btn: &gtk::Button, make: fn(Uuid) -> Op| {
            let (ui, on_err) = (self.clone(), on_err.clone());
            btn.connect_clicked(move |_| {
                let (ui2, on_err) = (ui.clone(), on_err.clone());
                glib::spawn_future_local(async move {
                    if let Err(e) = backend::call::<serde_json::Value>(make(id)).await {
                        on_err(e);
                    }
                    ui2.refresh();
                });
            });
        };
        op_btn(b.pause, |id| Op::Pause { id });
        op_btn(b.resume, |id| Op::Resume { id });
        op_btn(b.cancel, |id| Op::Cancel { id });
        op_btn(b.retry, |id| Op::Retry { id });

        let current = row.current.clone();
        let window = self.window.clone();
        b.folder.connect_clicked(move |_| {
            let path = current.borrow().task.final_path();
            let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
            launcher.open_containing_folder(Some(&window), gio::Cancellable::NONE, |_| {});
        });

        // Menu de ações secundárias, com ações no escopo da própria linha.
        let group = gio::SimpleActionGroup::new();
        let menu = gio::Menu::new();
        let main = gio::Menu::new();
        main.append(Some(&tr("Detalhes")), Some("row.details"));
        main.append(Some(&tr("Tentar com um novo link…")), Some("row.new-url"));
        menu.append_section(None, &main);
        let order = gio::Menu::new();
        order.append(
            Some(&tr("Mover para o início da fila")),
            Some("row.move-top"),
        );
        order.append(Some(&tr("Subir na fila")), Some("row.move-up"));
        order.append(Some(&tr("Descer na fila")), Some("row.move-down"));
        menu.append_section(None, &order);
        let conns = gio::Menu::new();
        for n in [1u8, 4, 8, 16] {
            conns.append(
                Some(&trf("{n} conexões", &[("n", &n.to_string())])),
                Some(&format!("row.connections(byte {n})")),
            );
        }
        menu.append_submenu(Some(&tr("Conexões")), &conns);
        let danger = gio::Menu::new();
        danger.append(Some(&tr("Remover do histórico")), Some("row.remove"));
        danger.append(
            Some(&tr("Excluir arquivo do disco…")),
            Some("row.delete-file"),
        );
        menu.append_section(None, &danger);
        b.menu.set_menu_model(Some(&menu));

        let add = |name: &str, f: Rc<dyn Fn()>| {
            let a = gio::SimpleAction::new(name, None);
            a.connect_activate(move |_, _| f());
            group.add_action(&a);
            a
        };
        let ui = self.clone();
        let cur = row.current.clone();
        let a_details = add("details", Rc::new(move || ui.show_details(&cur.borrow())));
        let ui = self.clone();
        let a_newurl = add("new-url", Rc::new(move || ui.ask_new_url(id)));
        let mk_move = |dir: MoveDirection| {
            let ui = self.clone();
            Rc::new(move || {
                let ui2 = ui.clone();
                backend::fire(Op::Move { id, direction: dir }, move |e| ui2.toast(&e));
                self_refresh_later(&ui);
            }) as Rc<dyn Fn()>
        };
        let a_top = add("move-top", mk_move(MoveDirection::Top));
        let a_up = add("move-up", mk_move(MoveDirection::Up));
        let a_down = add("move-down", mk_move(MoveDirection::Down));
        let ui = self.clone();
        let a_remove = add(
            "remove",
            Rc::new(move || {
                let ui2 = ui.clone();
                glib::spawn_future_local(async move {
                    match backend::call::<serde_json::Value>(Op::RemoveFromHistory { id }).await {
                        Ok(_) => ui2.toast(&tr(
                            "Removido do histórico. O arquivo foi mantido no disco.",
                        )),
                        Err(e) => ui2.toast(&e),
                    }
                    ui2.refresh();
                });
            }),
        );
        let ui = self.clone();
        let cur = row.current.clone();
        let a_delete = add(
            "delete-file",
            Rc::new(move || ui.confirm_delete(&cur.borrow())),
        );

        let a_conn = gio::SimpleAction::new("connections", Some(glib::VariantTy::BYTE));
        let ui = self.clone();
        a_conn.connect_activate(move |_, v| {
            let Some(n) = v.and_then(|v| v.get::<u8>()) else { return };
            let ui2 = ui.clone();
            glib::spawn_future_local(async move {
                match backend::call::<serde_json::Value>(Op::ChangeConnections { id, connections: n }).await {
                    Ok(v) if v.get("restarted").and_then(|r| r.as_bool()) == Some(true) => ui2.toast(&tr(
                        "Conexões alteradas. A transferência foi reiniciada a partir do ponto em que estava.",
                    )),
                    Ok(_) => ui2.toast(&tr("Conexões alteradas.")),
                    Err(e) => ui2.toast(&e),
                }
                ui2.refresh();
            });
        });
        group.add_action(&a_conn);
        row.row.insert_action_group("row", Some(&group));

        // Habilita ações conforme o estado atual.
        let cur = row.current.clone();
        let sync = move || {
            let s = cur.borrow().task.state;
            let waiting = s == TaskState::Aguardando;
            for a in [&a_top, &a_up, &a_down] {
                a.set_enabled(waiting);
            }
            a_newurl.set_enabled(matches!(s, TaskState::Erro | TaskState::Cancelado));
            a_remove.set_enabled(s.is_terminal());
            a_delete.set_enabled(s.is_terminal());
            a_conn.set_enabled(matches!(
                s,
                TaskState::Aguardando | TaskState::Baixando | TaskState::Pausado
            ));
            let _ = &a_details;
        };
        sync();
        b.menu
            .connect_notify_local(Some("active"), move |_, _| sync());

        let ui = self.clone();
        let cur = row.current.clone();
        row.row
            .connect_activate(move |_| ui.show_details(&cur.borrow()));
    }

    fn show_details(&self, v: &TaskView) {
        let t = &v.task;
        let dialog = adw::Dialog::builder()
            .title(tr("Detalhes do download"))
            .content_width(520)
            .build();
        let page = adw::PreferencesPage::new();
        let group = adw::PreferencesGroup::new();
        let row = |title: String, value: String| {
            let r = adw::ActionRow::builder()
                .title(title)
                .subtitle(if value.is_empty() {
                    "—".to_string()
                } else {
                    value
                })
                .subtitle_selectable(true)
                .css_classes(["property"])
                .build();
            group.add(&r);
        };
        row(tr("Arquivo"), t.filename.clone());
        row(tr("Pasta"), t.destination_dir.to_string_lossy().to_string());
        row(tr("Endereço"), t.url.clone());
        row(tr("Estado"), state_label(t.state));
        row(
            tr("Tamanho"),
            t.total_bytes
                .map(format::bytes)
                .unwrap_or_else(|| tr("Desconhecido")),
        );
        row(tr("Recebido"), format::bytes(t.downloaded_bytes));
        row(
            tr("Conexões solicitadas (máximo)"),
            t.connections.as_u8().to_string(),
        );
        if t.state == TaskState::Baixando {
            row(
                tr("Conexões ativas agora"),
                v.active_connections.to_string(),
            );
            row(tr("Velocidade"), format::speed(v.download_speed));
        }
        row(
            tr("SHA-256 esperado"),
            t.expected_sha256.clone().unwrap_or_default(),
        );
        if let Some(e) = &t.error_message {
            row(tr("Erro"), e.clone());
        }
        row(tr("Identificador da tarefa"), t.id.to_string());
        row(tr("GID no aria2"), t.aria2_gid.clone().unwrap_or_default());
        page.add(&group);
        let tb = adw::ToolbarView::new();
        tb.add_top_bar(&adw::HeaderBar::new());
        tb.set_content(Some(&page));
        dialog.set_child(Some(&tb));
        dialog.present(Some(&self.window));
    }

    fn ask_new_url(self: &Rc<Self>, id: Uuid) {
        let d = adw::AlertDialog::new(
            Some(&tr("Tentar com um novo link")),
            Some(&tr("Será criada uma nova tarefa. O arquivo parcial do link anterior não é reaproveitado, porque não há como confirmar que é o mesmo conteúdo.")),
        );
        let entry = gtk::Entry::builder()
            .placeholder_text("https://")
            .input_purpose(gtk::InputPurpose::Url)
            .build();
        d.set_extra_child(Some(&entry));
        d.add_responses(&[("cancel", &tr("Cancelar")), ("create", &tr("Criar tarefa"))]);
        d.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("create"));
        d.set_close_response("cancel");
        let ui = self.clone();
        d.choose(Some(&self.window), gio::Cancellable::NONE, move |resp| {
            if resp != "create" {
                return;
            }
            let url = entry.text().trim().to_string();
            let ui2 = ui.clone();
            glib::spawn_future_local(async move {
                match backend::call::<serde_json::Value>(Op::RetryWithNewUrl { id, url }).await {
                    Ok(_) => ui2.toast(&tr("Nova tarefa criada.")),
                    Err(e) => ui2.toast(&e),
                }
                ui2.refresh();
            });
        });
    }

    fn confirm_delete(self: &Rc<Self>, v: &TaskView) {
        let id = v.task.id;
        let d = adw::AlertDialog::new(
            Some(&tr("Excluir arquivo do disco?")),
            Some(&trf(
                "“{name}” e seus dados parciais serão apagados de {dir} e o item sairá da lista. Isso não pode ser desfeito.",
                &[("name", &v.task.filename), ("dir", &v.task.destination_dir.to_string_lossy())],
            )),
        );
        d.add_responses(&[
            ("cancel", &tr("Cancelar")),
            ("delete", &tr("Excluir arquivo")),
        ]);
        d.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        d.set_default_response(Some("cancel"));
        d.set_close_response("cancel");
        let ui = self.clone();
        d.choose(Some(&self.window), gio::Cancellable::NONE, move |resp| {
            if resp != "delete" {
                return;
            }
            let ui2 = ui.clone();
            glib::spawn_future_local(async move {
                match backend::call::<serde_json::Value>(Op::DeleteFile { id }).await {
                    Ok(_) => ui2.toast(&tr("Arquivo excluído.")),
                    Err(e) => ui2.toast(&e),
                }
                ui2.refresh();
            });
        });
    }

    fn confirm_quit_backend(self: &Rc<Self>) {
        let d = adw::AlertDialog::new(
            Some(&tr("Pausar tudo e encerrar?")),
            Some(&tr("Todos os downloads serão pausados e o serviço em segundo plano será encerrado. Eles poderão ser retomados na próxima vez que você abrir o Lyra Downloads.")),
        );
        d.add_responses(&[
            ("cancel", &tr("Cancelar")),
            ("quit", &tr("Pausar e encerrar")),
        ]);
        d.set_response_appearance("quit", adw::ResponseAppearance::Destructive);
        d.set_close_response("cancel");
        let ui = self.clone();
        d.choose(Some(&self.window), gio::Cancellable::NONE, move |resp| {
            if resp != "quit" {
                return;
            }
            let ui2 = ui.clone();
            glib::spawn_future_local(async move {
                if let Ok(mut c) = gio::spawn_blocking(|| {
                    lyra_downloads_ipc::client::BackendClient::connect(
                        &lyra_downloads_core::paths::backend_socket_path()?,
                    )
                })
                .await
                .map_err(|_| ())
                .and_then(|r| r.map_err(|_| ()))
                {
                    let _ = gio::spawn_blocking(move || c.call::<serde_json::Value>(Op::Shutdown))
                        .await;
                }
                if let Some(app) = ui2.window.application() {
                    app.quit();
                }
            });
        });
    }
}

fn self_refresh_later(ui: &Rc<Ui>) {
    let ui = ui.clone();
    glib::timeout_add_local_once(Duration::from_millis(250), move || ui.refresh());
}

pub fn build(app: &adw::Application) -> Rc<Ui> {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Lyra Downloads")
        .default_width(1000)
        .default_height(620)
        .width_request(360)
        .height_request(320)
        .build();

    let header = adw::HeaderBar::new();
    let new_btn = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("list-add-symbolic")
                .label(tr("Novo download"))
                .build(),
        )
        .action_name("win.new-download")
        .css_classes(["suggested-action"])
        .tooltip_text(tr("Novo download (Ctrl+N)"))
        .build();
    header.pack_start(&new_btn);
    let menu = gio::Menu::new();
    let s1 = gio::Menu::new();
    s1.append(Some(&tr("Pausar tudo")), Some("win.pause-all"));
    s1.append(Some(&tr("Retomar tudo")), Some("win.resume-all"));
    menu.append_section(None, &s1);
    let s2 = gio::Menu::new();
    s2.append(Some(&tr("Preferências")), Some("win.preferences"));
    s2.append(Some(&tr("Sobre o Lyra Downloads")), Some("win.about"));
    menu.append_section(None, &s2);
    let s3 = gio::Menu::new();
    s3.append(
        Some(&tr("Pausar tudo e encerrar o serviço…")),
        Some("win.quit-backend"),
    );
    menu.append_section(None, &s3);
    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .primary(true)
        .tooltip_text(tr("Menu principal"))
        .build();
    header.pack_end(&menu_btn);
    let search_btn = gtk::ToggleButton::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text(tr("Buscar (Ctrl+F)"))
        .build();
    header.pack_end(&search_btn);

    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text(tr("Buscar por nome ou endereço"))
        .hexpand(true)
        .build();
    let search_bar = gtk::SearchBar::builder()
        .child(
            &adw::Clamp::builder()
                .child(&search_entry)
                .maximum_size(560)
                .build(),
        )
        .build();
    search_bar.connect_entry(&search_entry);
    search_bar.set_key_capture_widget(Some(&window));
    search_btn
        .bind_property("active", &search_bar, "search-mode-enabled")
        .bidirectional()
        .build();

    let banner = adw::Banner::new("");
    let stack = adw::ViewStack::new();
    let mut page_uis = Vec::new();
    for page in PAGES {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .valign(gtk::Align::Start)
            .build();
        list.update_property(&[gtk::accessible::Property::Label(&page.title())]);
        let clamp = adw::Clamp::builder()
            .child(&list)
            .maximum_size(900)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&clamp)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let (title, desc) = page.empty();
        let status = adw::StatusPage::builder()
            .icon_name(page.icon())
            .title(title)
            .description(desc)
            .build();
        let inner = gtk::Stack::new();
        inner.add_named(&scroller, Some("list"));
        inner.add_named(&status, Some("empty"));
        inner.set_visible_child_name("empty");
        let vp = stack.add_titled_with_icon(&inner, Some(page.name()), &page.title(), page.icon());
        page_uis.push(PageUi {
            page,
            stack: inner,
            list,
            rows: HashMap::new(),
            order: Vec::new(),
            view_page: vp,
        });
    }

    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    header.set_title_widget(Some(&switcher));
    let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();

    let summary = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["dim-label", "caption", "numeric"])
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();

    let toasts = adw::ToastOverlay::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.add_top_bar(&search_bar);
    toolbar.add_top_bar(&banner);
    toolbar.set_content(Some(&stack));
    toolbar.add_bottom_bar(&switcher_bar);
    toolbar.add_bottom_bar(&summary);
    toasts.set_child(Some(&toolbar));
    window.set_content(Some(&toasts));

    // Janela estreita: seletor de páginas vai para a barra inferior.
    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        860.0,
        adw::LengthUnit::Sp,
    ));
    bp.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
    bp.add_setter(
        &header,
        "title-widget",
        Some(&None::<gtk::Widget>.to_value()),
    );
    bp.add_setter(
        &new_btn,
        "child",
        Some(&gtk::Image::from_icon_name("list-add-symbolic").to_value()),
    );
    window.add_breakpoint(bp);
    new_btn.update_property(&[gtk::accessible::Property::Label(&tr("Novo download"))]);

    let ui = Rc::new(Ui {
        window: window.clone(),
        toasts,
        banner: banner.clone(),
        summary,
        pages: RefCell::new(page_uis),
        search: RefCell::new(String::new()),
        settings: RefCell::new(Settings::default()),
        backend_error: RefCell::new(None),
    });

    banner.connect_button_clicked({
        let ui = ui.clone();
        move |_| {
            let ui = ui.clone();
            backend::fire(Op::RestartEngine, move |e| ui.toast(&e));
        }
    });

    search_entry.connect_search_changed({
        let ui = ui.clone();
        move |e| {
            *ui.search.borrow_mut() = e.text().to_string();
            ui.refresh();
        }
    });

    let action = |name: &str, f: Rc<dyn Fn()>| {
        let a = gio::SimpleAction::new(name, None);
        a.connect_activate(move |_, _| f());
        window.add_action(&a);
    };
    let u = ui.clone();
    action(
        "new-download",
        Rc::new(move || u.open_add_dialog(Prefill::default())),
    );
    let sb = search_btn.clone();
    action("search", Rc::new(move || sb.set_active(!sb.is_active())));
    let u = ui.clone();
    action(
        "pause-all",
        Rc::new(move || {
            let u2 = u.clone();
            backend::fire(Op::PauseAll, move |e| u2.toast(&e));
            self_refresh_later(&u);
        }),
    );
    let u = ui.clone();
    action(
        "resume-all",
        Rc::new(move || {
            let u2 = u.clone();
            backend::fire(Op::ResumeAll, move |e| u2.toast(&e));
            self_refresh_later(&u);
        }),
    );
    let u = ui.clone();
    action(
        "preferences",
        Rc::new(move || {
            let u2 = u.clone();
            crate::prefs::present(
                &u.window,
                u.settings.borrow().clone(),
                move |res| match res {
                    Ok(s) => *u2.settings.borrow_mut() = s,
                    Err(e) => u2.toast(&e),
                },
            );
        }),
    );
    let u = ui.clone();
    action("quit-backend", Rc::new(move || u.confirm_quit_backend()));
    let w = window.clone();
    action(
        "about",
        Rc::new(move || {
            let about = adw::AboutDialog::builder()
                .application_name("Lyra Downloads")
                .application_icon(crate::APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .developer_name(tr("Projeto Lyra OS"))
                .license_type(gtk::License::Gpl30)
                .website("https://github.com/lyra-os-linux/lyra-downloads")
                .comments(tr(
                    "Gerenciador de downloads com conexões paralelas, usando o motor aria2.",
                ))
                .build();
            about.present(Some(&w));
        }),
    );

    app.set_accels_for_action("win.new-download", &["<Ctrl>N"]);
    app.set_accels_for_action("win.search", &["<Ctrl>F"]);
    app.set_accels_for_action("win.preferences", &["<Ctrl>comma"]);
    app.set_accels_for_action("window.close", &["<Ctrl>W", "<Ctrl>Q"]);

    // Configurações iniciais e atualização periódica (~1 s).
    {
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            match backend::call::<Settings>(Op::GetSettings).await {
                Ok(s) => *ui.settings.borrow_mut() = s,
                Err(e) => ui.toast(&e),
            }
        });
    }
    ui.refresh();
    {
        let weak = Rc::downgrade(&ui);
        glib::timeout_add_local(Duration::from_secs(1), move || match weak.upgrade() {
            Some(ui) => {
                ui.refresh();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }

    window.present();
    ui
}
