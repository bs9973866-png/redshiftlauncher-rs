#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]
//! Redshift Launcher — нативный интерфейс (egui) в оформлении оригинального Redshift.
//! Перерисовка только по событиям: в простое процесс спит, анимаций и таймеров нет.

mod theme;
mod widgets;

use eframe::egui::{self, Color32, RichText};
use redshift_core::{accounts, install, modrinth, net, perf, play, settings::{self, Account, Instance, Store}};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use theme::*;
use widgets::*;

const LOG_MAX: usize = 500;

enum Msg {
    Log(String), Status(String), Progress(f32),
    Versions(Vec<install::VersionEntry>), Hits(Vec<serde_json::Value>),
    Finished(Result<String, String>),
}

#[derive(PartialEq, Clone, Copy)]
enum Page { Play, Mods, Skins, Accounts, Settings }

struct App {
    root: PathBuf, store: Store, page: Page, tex: Textures,
    tx: Sender<Msg>, rx: Receiver<Msg>,
    busy: bool, status: String, progress: f32, log: VecDeque<String>,
    versions: Vec<install::VersionEntry>, show_snapshots: bool,
    dlg_new: bool, new_name: String, new_mc: String, new_loader: String,
    confirm_delete: Option<String>,
    nick: String, ely_user: String, ely_pass: String,
    query: String, hits: Vec<serde_json::Value>, error: Option<String>,
}

fn data_root() -> PathBuf {
    // Portable: папка `data` рядом с exe. Иначе %APPDATA%\RedshiftLauncher.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent().map(|p| p.join("data")) { if d.is_dir() { return d; } }
    }
    let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("XDG_DATA_HOME")).map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))).unwrap_or_else(|| PathBuf::from("."));
    base.join("RedshiftLauncher")
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx); apply_style(&cc.egui_ctx);
        let root = data_root(); let (tx, rx) = channel();
        App { store: settings::load(&root), root, page: Page::Play, tex: load_textures(&cc.egui_ctx), tx, rx, busy: false, status: "Готов к запуску".into(),
              progress: 0.0, log: VecDeque::new(), versions: vec![], show_snapshots: false, dlg_new: false, new_name: String::new(),
              new_mc: "1.20.1".into(), new_loader: "vanilla".into(), confirm_delete: None, nick: String::new(), ely_user: String::new(),
              ely_pass: String::new(), query: String::new(), hits: vec![], error: None }
    }
    fn save(&mut self) { if let Err(e) = settings::save(&self.root, &self.store) { self.error = Some(format!("Не удалось сохранить настройки: {e}")); } }

    fn spawn(&mut self, ctx: &egui::Context, f: impl FnOnce(Sender<Msg>, egui::Context) -> Result<String, String> + Send + 'static) {
        if self.busy { return; }
        self.busy = true; self.error = None; self.progress = 0.0;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || { let r = f(tx.clone(), ctx.clone()); let _ = tx.send(Msg::Finished(r)); ctx.request_repaint(); });
    }

    fn drain(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Log(l) => { if self.log.len() >= LOG_MAX { self.log.pop_front(); } self.log.push_back(l); }
                Msg::Status(s) => self.status = s, Msg::Progress(p) => self.progress = p,
                Msg::Versions(v) => self.versions = v, Msg::Hits(h) => self.hits = h,
                Msg::Finished(r) => {
                    self.busy = false;
                    match r { Ok(s) => { self.status = s; self.progress = 0.0; } Err(e) => { self.error = Some(e); self.status = "Ошибка".into(); } }
                    self.store = settings::load(&self.root);
                }
            }
        }
    }

    fn selected_instance(&self) -> Option<&Instance> {
        self.store.settings.selected_instance.as_ref().and_then(|id| self.store.instances.iter().find(|i| &i.id == id)).or(self.store.instances.first())
    }
    fn selected_account(&self) -> Option<&Account> {
        self.store.settings.selected_account.as_ref().and_then(|id| self.store.accounts.iter().find(|a| &a.id == id)).or(self.store.accounts.first())
    }

    fn start_play(&mut self, ctx: &egui::Context, inst: Instance, acc: Account) {
        self.log.clear();
        let (root, store) = (self.root.clone(), self.store.clone());
        self.spawn(ctx, move |tx, ctx| {
            // Store живёт столько же, сколько запуск: Box::leak один раз на запуск (несколько КБ).
            let st: &'static Store = Box::leak(Box::new(store));
            let c = play::Ctx { dirs: install::Dirs { root: root.clone() }, store: st, agent: net::agent(), sys: play::system() };
            let (t1, c1, t2, c2) = (tx.clone(), ctx.clone(), tx.clone(), ctx.clone());
            let out = play::play(&c, &inst, &acc, &move |s| { let _ = t1.send(Msg::Status(s.into())); c1.request_repaint(); },
                &move |l| { let _ = t2.send(Msg::Log(l.into())); c2.request_repaint_after(std::time::Duration::from_millis(200)); })?;
            match out.code { Some(0) => Ok("Игра закрыта".into()), c => Err(format!("Игра завершилась с кодом {c:?}. Подробности — в журнале запуска.")) }
        });
    }

    fn start_optimize(&mut self, ctx: &egui::Context, inst: Instance) {
        let root = self.root.clone();
        self.spawn(ctx, move |tx, ctx| {
            let dir = settings::instance_dir(&root, &inst.id);
            perf::apply_video_tweaks(&dir, false)?;
            let mut ok = 0; let mut failed = vec![];
            let mods = perf::fps_mods(&inst.loader);
            for (n, m) in mods.iter().enumerate() {
                let _ = tx.send(Msg::Status(format!("FPS-моды: {m}"))); let _ = tx.send(Msg::Progress(n as f32 / mods.len().max(1) as f32)); ctx.request_repaint();
                match modrinth::install_mod(&net::agent(), modrinth::API, m, &inst.mc, &inst.loader, &dir.join("mods")) { Ok(_) => ok += 1, Err(_) => failed.push(*m) }
            }
            Ok(format!("Настройки видео применены. Модов установлено: {ok}/{}{}", mods.len(), if failed.is_empty() { String::new() } else { format!(" (нет для этой версии: {})", failed.join(", ")) }))
        });
    }

    // ---------- интерфейс ----------

    fn titlebar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("tb").exact_height(38.0).frame(egui::Frame::none()).show(ctx, |ui| {
            let r = ui.max_rect();
            let drag = ui.interact(r, ui.id().with("drag"), egui::Sense::click_and_drag());
            if drag.drag_started() { ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
            if drag.double_clicked() { let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false)); ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!m)); }
            let img = egui::Image::new(egui::load::SizedTexture::new(self.tex.logo.id(), egui::vec2(16.0, 16.0))).rounding(4.0);
            img.paint_at(ui, egui::Rect::from_min_size(r.min + egui::vec2(14.0, 11.0), egui::vec2(16.0, 16.0)));
            ui.painter().text(r.min + egui::vec2(38.0, 19.0), egui::Align2::LEFT_CENTER, "Redshift Launcher", egui::FontId::proportional(12.0), MUTE);
            for (n, kind) in [WinBtn::Close, WinBtn::Max, WinBtn::Min].into_iter().enumerate() {
                let br = egui::Rect::from_min_size(egui::pos2(r.max.x - 46.0 * (n as f32 + 1.0), r.min.y), egui::vec2(46.0, 38.0));
                if window_button(ui, br, kind).clicked() {
                    match kind {
                        WinBtn::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        WinBtn::Min => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                        WinBtn::Max => { let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false)); ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!m)); }
                    }
                }
            }
        });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let frame = egui::Frame::none().fill(ca(8, 11, 20, 115)).inner_margin(egui::Margin { left: 14.0, right: 14.0, top: 10.0, bottom: 16.0 });
        egui::SidePanel::left("side").exact_width(236.0).resizable(false).frame(frame).show(ctx, |ui| {
            let r = ui.max_rect();
            ui.painter().line_segment([egui::pos2(r.max.x + 14.0, r.min.y - 10.0), egui::pos2(r.max.x + 14.0, r.max.y + 16.0)], egui::Stroke::new(1.0, line()));
            // бренд
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let (rc, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                glow_rect(ui.painter(), rc, 10.0, A1);
                egui::Image::new(egui::load::SizedTexture::new(self.tex.logo.id(), egui::vec2(40.0, 40.0))).rounding(10.0).paint_at(ui, rc);
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.label(RichText::new("R E D S H I F T").size(17.0).strong().color(A1));
                    ui.label(RichText::new("L A U N C H E R").size(9.0).color(MUTE));
                });
            });
            ui.add_space(14.0);
            for (p, t, ic) in [(Page::Play, "Играть", Icon::Play), (Page::Mods, "Моды", Icon::Cube), (Page::Skins, "Скины", Icon::Person), (Page::Accounts, "Аккаунты", Icon::Users), (Page::Settings, "Настройки", Icon::Sliders)] {
                if nav_button(ui, self.page == p, t, ic).clicked() { self.page = p; }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                let (name, sub) = match self.selected_account() { Some(a) => (a.name.clone(), match a.kind.as_str() { "ely" => "Ely.by".to_string(), _ => "Офлайн".to_string() }), None => ("Нет аккаунта".into(), "Нажмите, чтобы войти".into()) };
                if account_chip(ui, &self.tex, &name, &sub, self.selected_account().is_some()).clicked() { self.page = Page::Accounts; }
            });
        });
    }

    fn page_play(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let inst = self.selected_instance().cloned();
        let acc = self.selected_account().cloned();
        glass(ui, 28.0, |ui| {
            ui.label(RichText::new("ВЫБРАННАЯ СБОРКА").size(11.0).strong().color(A1));
            ui.add_space(6.0);
            ui.label(RichText::new(inst.as_ref().map(|i| i.name.as_str()).unwrap_or("Создайте первую сборку")).size(34.0).strong());
            if let Some(i) = &inst { ui.label(RichText::new(format!("Minecraft {} · {}", i.mc, i.loader)).color(MUTE)); } else { ui.add_space(22.0); }
            ui.add_space(18.0);
            let can_play = !self.busy && inst.is_some() && acc.is_some();
            if grad_button(ui, &self.tex, "И Г Р А Т Ь", egui::vec2(228.0, 62.0), can_play).clicked() { if let (Some(i), Some(a)) = (inst.clone(), acc.clone()) { self.start_play(ctx, i, a); } }
            if inst.is_some() && acc.is_none() { ui.label(RichText::new("Добавьте аккаунт во вкладке «Аккаунты».").color(DANGER)); }
            ui.horizontal(|ui| {
                let en = !self.busy && inst.is_some();
                if soft_button(ui, "⚡ Оптимизировать FPS", false, en).clicked() { if let Some(i) = inst.clone() { self.start_optimize(ctx, i); } }
                if soft_button(ui, "Открыть папку", false, inst.is_some()).clicked() { if let Some(i) = &inst { let d = settings::instance_dir(&self.root, &i.id); let _ = std::fs::create_dir_all(&d); open_path(&d); } }
                let confirm = self.confirm_delete.as_deref() == inst.as_ref().map(|i| i.id.as_str()) && inst.is_some();
                if soft_button(ui, if confirm { "Точно удалить?" } else { "Удалить" }, true, inst.is_some() && !self.busy).clicked() {
                    if let Some(i) = &inst {
                        if confirm { let _ = std::fs::remove_dir_all(settings::instance_dir(&self.root, &i.id)); self.store.instances.retain(|x| x.id != i.id); self.confirm_delete = None; self.save(); }
                        else { self.confirm_delete = Some(i.id.clone()); }
                    }
                }
            });
            ui.add_space(10.0);
            ui.label(RichText::new(&self.status).size(12.0).color(MUTE));
            progress_bar(ui, &self.tex, self.progress, self.busy);
            if let Some(e) = self.error.clone() { ui.horizontal_wrapped(|ui| { ui.colored_label(DANGER, format!("⚠ {e}")); if ui.small_button("✕").clicked() { self.error = None; } }); }
        });
        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Сборки").size(20.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if grad_button(ui, &self.tex, "+ Новая сборка", egui::vec2(150.0, 40.0), true).clicked() { self.dlg_new = true; if self.versions.is_empty() && !self.busy { self.fetch_versions(ctx); } }
            });
        });
        let mut select = None;
        for i in &self.store.instances {
            let sel = inst.as_ref().map(|x| x.id == i.id).unwrap_or(false);
            if instance_card(ui, sel, &i.name, &format!("Minecraft {} · {}", i.mc, i.loader)).clicked() { select = Some(i.id.clone()); }
        }
        if let Some(id) = select { self.store.settings.selected_instance = Some(id); self.save(); }
        ui.add_space(10.0);
        egui::CollapsingHeader::new(RichText::new("Журнал запуска").strong()).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(220.0).stick_to_bottom(true).show(ui, |ui| { for l in &self.log { ui.label(RichText::new(l).monospace().size(11.0).color(MUTE)); } });
        });
    }

    fn fetch_versions(&mut self, ctx: &egui::Context) {
        self.spawn(ctx, |tx, _| { let m = net::get_json(&net::agent(), install::MANIFEST_URL)?; let v = install::parse_manifest(&m); let n = v.len(); let _ = tx.send(Msg::Versions(v)); Ok(format!("Доступно версий: {n}")) });
    }

    fn dialog_new(&mut self, ctx: &egui::Context) {
        if !self.dlg_new { return; }
        let mut open = true; let mut create = false;
        egui::Window::new("Новая сборка").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label("Название"); ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("Моя сборка").desired_width(f32::INFINITY));
            ui.label("Версия Minecraft");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("mc").width(150.0).selected_text(&self.new_mc).show_ui(ui, |ui| {
                    for v in self.versions.iter().filter(|v| v.kind == "release" || self.show_snapshots) { ui.selectable_value(&mut self.new_mc, v.id.clone(), &v.id); }
                });
                ui.checkbox(&mut self.show_snapshots, "снапшоты, беты, альфы");
            });
            if self.versions.is_empty() { ui.label(RichText::new(if self.busy { "Загрузка списка версий…" } else { "Список версий не загружен (проверьте интернет)." }).color(MUTE)); }
            ui.label("Загрузчик");
            ui.horizontal(|ui| { for l in ["vanilla", "fabric", "quilt", "forge", "neoforge"] { ui.selectable_value(&mut self.new_loader, l.to_string(), l); } });
            ui.add_space(6.0);
            if grad_button(ui, &self.tex, "Создать", egui::vec2(380.0, 42.0), true).clicked() { create = true; }
        });
        if create {
            let name = if self.new_name.trim().is_empty() { format!("{} {}", self.new_loader, self.new_mc) } else { self.new_name.trim().to_string() };
            let id = settings::unique_id(&self.store.instances, &name);
            self.store.instances.push(Instance { id: id.clone(), name, mc: self.new_mc.clone(), loader: self.new_loader.clone(), ..Default::default() });
            self.store.settings.selected_instance = Some(id); self.new_name.clear(); self.save(); open = false;
        }
        self.dlg_new = open;
    }

    fn page_mods(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        title(ui, "Моды", "Поиск и установка с Modrinth в выбранную сборку");
        let sel = self.selected_instance().cloned();
        glass(ui, 18.0, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Название мода…").desired_width(ui.available_width() - 120.0));
                if soft_button(ui, "Искать", false, !self.busy).clicked() {
                    let (q, mc, ld) = (self.query.clone(), sel.as_ref().map(|i| i.mc.clone()), sel.as_ref().map(|i| i.loader.clone()));
                    self.spawn(ctx, move |tx, _| { let h = modrinth::search(&net::agent(), &q, "mod", mc.as_deref(), ld.as_deref())?; let n = h.len(); let _ = tx.send(Msg::Hits(h)); Ok(format!("Найдено: {n}")) });
                }
            });
            match &sel { None => { ui.label(RichText::new("Сначала создайте сборку на вкладке «Играть».").color(MUTE)); }
                Some(i) if i.loader == "vanilla" => { ui.label(RichText::new("Для vanilla-сборки моды не работают — выберите Fabric/Quilt/Forge/NeoForge.").color(A2)); } _ => {} }
        });
        let mut slug_to_install = None;
        for h in &self.hits {
            let t = h.get("title").and_then(|t| t.as_str()).unwrap_or("?"); let slug = h.get("slug").and_then(|t| t.as_str()).unwrap_or("");
            glass(ui, 14.0, |ui| { ui.horizontal(|ui| {
                ui.vertical(|ui| { ui.label(RichText::new(t).strong()); ui.label(RichText::new(h.get("description").and_then(|d| d.as_str()).unwrap_or("").chars().take(110).collect::<String>()).size(12.0).color(MUTE)); });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { if soft_button(ui, "Установить", false, !self.busy && sel.is_some()).clicked() { slug_to_install = Some(slug.to_string()); } });
            }); });
        }
        if let (Some(slug), Some(inst)) = (slug_to_install, sel) {
            let dest = settings::instance_dir(&self.root, &inst.id).join("mods");
            self.spawn(ctx, move |_, _| modrinth::install_mod(&net::agent(), modrinth::API, &slug, &inst.mc, &inst.loader, &dest).map(|n| format!("Установлено: {n}")));
        }
    }

    fn page_skins(&mut self, ui: &mut egui::Ui) {
        title(ui, "Скины", "Предпросмотр скина");
        glass(ui, 24.0, |ui| {
            ui.horizontal(|ui| {
                skin_preview(ui, &self.tex.steve, 8.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(self.selected_account().map(|a| a.name.as_str()).unwrap_or("Steve")).size(22.0).strong());
                    ui.label(RichText::new("Показан стандартный скин из ассетов лаунчера.").color(MUTE));
                    ui.add_space(8.0);
                    ui.label(RichText::new("Загрузка своего скина, 3D-просмотр и показ скина в игре ещё не перенесены на Rust\n(см. MIGRATION_HANDOFF.md) — в этой версии они недоступны.").size(12.0).color(A2));
                });
            });
        });
    }

    fn page_accounts(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        title(ui, "Аккаунты", "Офлайн и Ely.by");
        glass(ui, 18.0, |ui| {
            ui.label(RichText::new("Офлайн-аккаунт").strong());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.nick).hint_text("Ник (3–16 символов)").desired_width(240.0));
                if soft_button(ui, "Добавить", false, true).clicked() {
                    match accounts::new_offline(self.nick.trim()) {
                        Ok(a) => { if !self.store.accounts.iter().any(|x| x.id == a.id) { self.store.settings.selected_account = Some(a.id.clone()); self.store.accounts.push(a); self.save(); } self.nick.clear(); }
                        Err(e) => self.error = Some(e),
                    }
                }
            });
        });
        glass(ui, 18.0, |ui| {
            ui.label(RichText::new("Ely.by").strong());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.ely_user).hint_text("Логин или e-mail").desired_width(200.0));
                ui.add(egui::TextEdit::singleline(&mut self.ely_pass).password(true).hint_text("Пароль").desired_width(160.0));
                if soft_button(ui, "Войти", false, !self.busy).clicked() {
                    let (u, p) = (self.ely_user.clone(), std::mem::take(&mut self.ely_pass)); let root = self.root.clone();
                    self.spawn(ctx, move |_, _| {
                        let a = accounts::ely_login(&net::agent(), accounts::ELY_AUTH, &u, &p)?;
                        let mut st = settings::load(&root); st.accounts.retain(|x| x.id != a.id); st.settings.selected_account = Some(a.id.clone());
                        let n = a.name.clone(); st.accounts.push(a); settings::save(&root, &st)?; Ok(format!("Вошли как {n}"))
                    });
                }
            });
            ui.label(RichText::new("Лицензионный вход Microsoft и REDSHIFT ID ещё не перенесены на Rust.").size(12.0).color(A2));
        });
        if let Some(e) = self.error.clone() { ui.colored_label(DANGER, format!("⚠ {e}")); }
        let (mut remove, mut select) = (None, None);
        for a in &self.store.accounts {
            let sel = self.selected_account().map(|s| s.id == a.id).unwrap_or(false);
            ui.horizontal(|ui| {
                if instance_card(ui, sel, &a.name, if a.kind == "ely" { "Ely.by" } else { "Офлайн" }).clicked() { select = Some(a.id.clone()); }
            });
            if soft_button(ui, &format!("Удалить «{}»", a.name), true, true).clicked() { remove = Some(a.id.clone()); }
        }
        if let Some(id) = select { self.store.settings.selected_account = Some(id); self.save(); }
        if let Some(id) = remove { self.store.accounts.retain(|a| a.id != id); self.save(); }
    }

    fn page_settings(&mut self, ui: &mut egui::Ui) {
        title(ui, "Настройки", "Java, память и данные");
        let sys = play::system(); let mut changed = false;
        glass(ui, 22.0, |ui| {
            ui.label(RichText::new(format!("Ваш ПК: {} МБ RAM, {} потоков CPU", sys.ram_mb, sys.cpus)).color(MUTE));
            let mut auto = self.store.settings.ram_mb == 0;
            if ui.checkbox(&mut auto, "RAM для игры: авто (безопасно для этого ПК)").changed() { self.store.settings.ram_mb = if auto { 0 } else { 2048 }; changed = true; }
            if !auto { changed |= ui.add(egui::Slider::new(&mut self.store.settings.ram_mb, 1024..=16384).text("МБ")).changed(); }
            ui.horizontal(|ui| { ui.label("JVM-пресет");
                egui::ComboBox::from_id_salt("jp").selected_text(&self.store.settings.jvm_preset).show_ui(ui, |ui| { for p in ["auto", "safe", "balanced", "lowend", "max"] { changed |= ui.selectable_value(&mut self.store.settings.jvm_preset, p.to_string(), p).changed(); } }); });
            ui.label(RichText::new("safe — только -Xmx/-Xms. При сбое JVM лаунчер сам повторит запуск с safe.").size(12.0).color(MUTE));
            changed |= ui.add(egui::Slider::new(&mut self.store.settings.download_threads, 1..=8).text("потоков загрузки")).changed();
        });
        glass(ui, 22.0, |ui| {
            ui.label(RichText::new(format!("Папка данных: {}", self.root.display())).color(MUTE));
            ui.horizontal(|ui| {
                if soft_button(ui, "Открыть папку данных", false, true).clicked() { let _ = std::fs::create_dir_all(&self.root); open_path(&self.root); }
                if soft_button(ui, "Экспорт сборки (.mrpack)", false, self.selected_instance().is_some()).clicked() {
                    if let Some(i) = self.selected_instance().cloned() {
                        let out = self.root.join(format!("{}.mrpack", i.id));
                        match modrinth::export_mrpack(&settings::instance_dir(&self.root, &i.id), &out, &i.name, &i.mc, &i.loader, i.loader_version.as_deref()) {
                            Ok(n) => self.status = format!("Экспортировано {n} файлов → {}", out.display()), Err(e) => self.error = Some(e) }
                    }
                }
            });
            ui.label(RichText::new(&self.status).size(12.0).color(MUTE));
        });
        if changed { self.save(); }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] { [0.027, 0.039, 0.07, 1.0] }
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        self.drain();
        ctx.layer_painter(egui::LayerId::background()).image(self.tex.bg.id(), ctx.screen_rect(), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        self.titlebar(ctx); self.sidebar(ctx);
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin { left: 34.0, right: 34.0, top: 8.0, bottom: 20.0 })).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                match self.page { Page::Play => self.page_play(ui, ctx), Page::Mods => self.page_mods(ui, ctx), Page::Skins => self.page_skins(ui), Page::Accounts => self.page_accounts(ui, ctx), Page::Settings => self.page_settings(ui) }
            });
        });
        self.dialog_new(ctx);
    }
}

fn main() -> eframe::Result<()> {
    let vp = egui::ViewportBuilder::default().with_inner_size([1100.0, 700.0]).with_min_inner_size([860.0, 560.0])
        .with_decorations(false).with_title("Redshift Launcher").with_icon(std::sync::Arc::new(theme::window_icon()));
    eframe::run_native("Redshift Launcher", eframe::NativeOptions { viewport: vp, ..Default::default() }, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
