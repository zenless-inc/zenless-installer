//! The setup wizard (eframe/egui): fixed-size window with a step sidebar,
//! a content area and a navigation bar.

mod finish;
mod pages;
mod widgets;

use crate::components::{Component, VERSION, WEBSITE};
use crate::install::{self, InstallOutcome, InstallPlan, Installed, Options};
use crate::platform::{self, Browser, Env};
use crate::report::{Level, LogLine, SharedReporter, TaskState};
use crate::shared::kit;
use crate::shared::theme::{self, Appearance, Palette, Theme, alpha, mix};
use crate::uninstall::{self, UninstallOutcome, UninstallPlan};
use eframe::egui::{self, Align, Color32, Layout, Margin, RichText, Stroke, pos2, vec2};
use egui_phosphor::regular as ph;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use widgets::Icons;

pub const WINDOW_SIZE: [f32; 2] = [860.0, 580.0];
const SIDEBAR_W: f32 = 200.0;
const NAV_H: f32 = 66.0;
const MARGIN: i8 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Welcome,
    License,
    Components,
    Options,
    Appearance,
    Installing,
    Finish,
    UninstallChoose,
    Uninstalling,
    UninstallDone,
}

impl Page {
    fn is_uninstall(self) -> bool {
        matches!(self, Self::UninstallChoose | Self::Uninstalling | Self::UninstallDone)
    }
}

/// How the wizard was started.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    /// `--uninstall`: open the uninstaller directly.
    pub uninstall: bool,
    /// `ZENLESS_INSTALLER_PAGE`: open a page with demo data (screenshots).
    pub demo_page: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Theme list + the one currently previewed.
pub struct Look {
    pub appearance: Appearance,
    pub themes: Vec<Theme>,
    pub current: Theme,
}

impl Look {
    fn load() -> Self {
        let appearance = theme::load_appearance();
        let mut themes = theme::builtin_themes();
        themes.extend(theme::load_custom_themes());
        let current = themes
            .iter()
            .find(|t| t.name == appearance.theme)
            .cloned()
            .unwrap_or_else(|| themes[0].clone());
        Self { appearance, themes, current }
    }

    /// Applies the theme with a fixed text size / density: the installer has a
    /// fixed-size window and must not reflow with the user's app preferences.
    fn apply(&self, ctx: &egui::Context) {
        let a = Appearance { font_scale: 1.0, density: 1.0, ..self.appearance.clone() };
        theme::apply(ctx, &self.current, &a);
    }

    fn select(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(t) = self.themes.iter().find(|t| t.name == name) {
            self.current = t.clone();
            self.apply(ctx);
        }
    }
}

enum TaskResult {
    Install(Result<InstallOutcome, String>),
    Uninstall(Result<UninstallOutcome, String>),
}

/// A running background install / uninstall.
struct Task {
    cancel: Arc<AtomicBool>,
    result: Arc<Mutex<Option<TaskResult>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Modal {
    ConfirmExit,
    ConfirmCancel,
}

pub struct App {
    env: Env,
    page: Page,
    installed: Option<Installed>,
    modify: bool,
    license_ok: bool,
    selected: Vec<Component>,
    options: Options,
    dir: String,
    free: Option<(String, Option<u64>)>,
    look: Look,
    icons: Icons,

    task: Option<Task>,
    progress: Arc<Mutex<TaskState>>,
    install_result: Option<Result<InstallOutcome, String>>,
    uninstall_result: Option<Result<UninstallOutcome, String>>,
    demo_running: bool,

    launch_dm: bool,
    launch_torrent: bool,
    browsers: Option<Vec<Browser>>,
    toast: Option<(String, Instant)>,

    un_selected: Vec<Component>,
    un_settings: bool,
    un_from_welcome: bool,

    modal: Option<Modal>,
    shot: kit::AutoScreenshot,
    self_delete: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, env: Env, launch: Launch) -> Self {
        let ctx = &cc.egui_ctx;
        theme::install_fonts(ctx);
        let look = Look::load();
        look.apply(ctx);

        let installed = Installed::detect(&env).or_else(|| {
            // Running as <root>\uninstall.exe without registry state.
            let me = std::env::current_exe().ok()?;
            let is_uninstaller = me.file_name().is_some_and(|n| n.eq_ignore_ascii_case("uninstall.exe"));
            if is_uninstaller { Installed::from_folder(&env, me.parent()?.to_path_buf()) } else { None }
        });
        let dir = installed
            .as_ref()
            .map(|i| i.root.clone())
            .unwrap_or_else(|| env.default_install_dir())
            .display()
            .to_string();
        let mut app = Self {
            page: Page::Welcome,
            modify: false,
            license_ok: false,
            selected: Component::ALL.to_vec(),
            options: Options::default(),
            dir,
            free: None,
            look,
            icons: Icons::load(ctx),
            task: None,
            progress: Arc::new(Mutex::new(TaskState::default())),
            install_result: None,
            uninstall_result: None,
            demo_running: false,
            launch_dm: true,
            launch_torrent: false,
            browsers: None,
            toast: None,
            un_selected: installed.as_ref().map(|i| i.components.clone()).unwrap_or_default(),
            un_settings: false,
            un_from_welcome: false,
            modal: None,
            shot: kit::AutoScreenshot::from_env(),
            self_delete: None,
            installed,
            env,
        };
        if launch.uninstall {
            app.page = Page::UninstallChoose;
        }
        if let Some(page) = &launch.demo_page {
            app.apply_demo(page);
        }
        app
    }

    fn installed_components(&self) -> Vec<Component> {
        self.installed.as_ref().map(|i| i.components.clone()).unwrap_or_default()
    }

    /// Components that were installed and are now deselected (Modify).
    fn removals(&self) -> Vec<Component> {
        if !self.modify {
            return Vec::new();
        }
        self.installed_components().into_iter().filter(|c| !self.selected.contains(c)).collect()
    }

    fn is_busy(&self) -> bool {
        self.task.is_some() || self.demo_running
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    // -----------------------------------------------------------------------
    // Flow
    // -----------------------------------------------------------------------

    fn steps(&self) -> (Vec<&'static str>, usize) {
        if self.page.is_uninstall() {
            let names = vec!["Choose", "Remove", "Done"];
            let i = match self.page {
                Page::UninstallChoose => 0,
                Page::Uninstalling => 1,
                _ => 2,
            };
            return (names, i);
        }
        let mut names = vec!["Welcome", "License", "Components", "Options", "Appearance", "Install", "Finish"];
        let mut pages = vec![
            Page::Welcome,
            Page::License,
            Page::Components,
            Page::Options,
            Page::Appearance,
            Page::Installing,
            Page::Finish,
        ];
        if self.modify {
            names.remove(1);
            pages.remove(1);
            names[4] = "Update";
        }
        let i = pages.iter().position(|p| *p == self.page).unwrap_or(0);
        (names, i)
    }

    fn next(&mut self, ctx: &egui::Context) {
        self.page = match self.page {
            Page::Welcome => {
                if self.modify { Page::Components } else { Page::License }
            }
            Page::License => Page::Components,
            Page::Components => Page::Options,
            Page::Options => Page::Appearance,
            Page::Appearance => {
                self.start_install(ctx);
                Page::Installing
            }
            other => other,
        };
    }

    fn back(&mut self) {
        self.page = match self.page {
            Page::License => Page::Welcome,
            Page::Components => {
                if self.modify { Page::Welcome } else { Page::License }
            }
            Page::Options => Page::Components,
            Page::Appearance => Page::Options,
            Page::Installing => Page::Options,
            Page::UninstallChoose => Page::Welcome,
            other => other,
        };
    }

    /// "Modify / Update" on the Welcome page.
    fn begin_modify(&mut self) {
        self.modify = true;
        if let Some(i) = &self.installed {
            self.selected = if i.components.is_empty() { Component::ALL.to_vec() } else { i.components.clone() };
            self.options = i.options.clone();
            self.dir = i.root.display().to_string();
        }
        self.page = Page::Components;
    }

    fn begin_uninstall(&mut self) {
        self.un_selected = self.installed_components();
        self.un_from_welcome = true;
        self.page = Page::UninstallChoose;
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        let has = |c| self.selected.contains(&c);
        let options = Options {
            autostart_dm: self.options.autostart_dm && has(Component::Dm),
            autostart_torrent: self.options.autostart_torrent && has(Component::Torrent),
            associate: self.options.associate && has(Component::Torrent),
            ..self.options.clone()
        };
        let plan = InstallPlan {
            root: PathBuf::from(self.dir.trim()),
            components: self.selected.clone(),
            remove: self.removals(),
            options,
            theme: Some(self.look.current.name.clone()),
        };
        self.install_result = None;
        self.launch_dm = has(Component::Dm);
        self.launch_torrent = false;
        let env = self.env.clone();
        self.spawn(ctx, move |r| TaskResult::Install(install::run(&env, &plan, r)));
    }

    fn start_uninstall(&mut self, ctx: &egui::Context) {
        let Some(installed) = &self.installed else { return };
        let plan = UninstallPlan {
            root: installed.root.clone(),
            components: self.un_selected.clone(),
            remove_settings: self.un_settings,
        };
        self.uninstall_result = None;
        let env = self.env.clone();
        self.spawn(ctx, move |r| TaskResult::Uninstall(uninstall::run(&env, &plan, r)));
        self.page = Page::Uninstalling;
    }

    fn spawn(&mut self, ctx: &egui::Context, job: impl FnOnce(&SharedReporter) -> TaskResult + Send + 'static) {
        self.progress = Arc::new(Mutex::new(TaskState::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let result = Arc::new(Mutex::new(None));
        let repaint_ctx = ctx.clone();
        let reporter = SharedReporter::new(self.progress.clone(), cancel.clone(), move || repaint_ctx.request_repaint());
        let slot = result.clone();
        let done_ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = job(&reporter);
            *lock(&slot) = Some(r);
            done_ctx.request_repaint();
        });
        self.task = Some(Task { cancel, result });
    }

    fn poll_task(&mut self) {
        let Some(task) = &self.task else { return };
        let Some(result) = lock(&task.result).take() else { return };
        self.task = None;
        match result {
            TaskResult::Install(r) => {
                if let Ok(o) = &r {
                    self.installed = Installed::detect(&self.env).or_else(|| {
                        Some(Installed {
                            root: o.root.clone(),
                            version: VERSION.into(),
                            components: o.components.clone(),
                            options: self.options.clone(),
                        })
                    });
                    self.page = Page::Finish;
                }
                self.install_result = Some(r);
            }
            TaskResult::Uninstall(r) => {
                if let Ok(o) = &r {
                    self.self_delete = o.self_delete.clone();
                    self.page = Page::UninstallDone;
                } else {
                    self.page = Page::UninstallDone;
                }
                self.uninstall_result = Some(r);
            }
        }
    }

    fn request_cancel(&mut self) {
        if let Some(t) = &self.task {
            t.cancel.store(true, Ordering::Relaxed);
        }
        self.demo_running = false;
    }

    fn exit(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Launches the apps ticked on the Finish page.
    fn launch_selected(&mut self) {
        let Some(Ok(o)) = &self.install_result else { return };
        let layout = crate::components::Layout::new(&o.root);
        let mut todo = Vec::new();
        if self.launch_dm && o.components.contains(&Component::Dm) {
            todo.push(layout.dm_exe());
        }
        if self.launch_torrent && o.components.contains(&Component::Torrent) {
            todo.push(layout.torrent_exe());
        }
        for exe in todo {
            if self.env.is_sandbox() {
                eprintln!("sandbox: would launch {}", exe.display());
            } else if let Err(e) = platform::launch(&exe, &[]) {
                eprintln!("could not launch {}: {e}", exe.display());
            }
        }
    }

    /// Runs an external program unless in sandbox mode (then it just says so).
    fn run_external(&mut self, what: &str, exe: &std::path::Path, args: &[&std::ffi::OsStr]) {
        if self.env.is_sandbox() {
            self.toast(format!("Sandbox mode: would {what}"));
        } else if let Err(e) = platform::launch(exe, args) {
            self.toast(format!("Could not {what}: {e}"));
        }
    }

    fn open_external(&mut self, what: &str, target: &std::ffi::OsStr) {
        if self.env.is_sandbox() {
            self.toast(format!("Sandbox mode: would {what}"));
        } else if let Err(e) = platform::shell_open(target) {
            self.toast(format!("Could not {what}: {e}"));
        }
    }

    // -----------------------------------------------------------------------
    // Demo states for screenshots (ZENLESS_INSTALLER_PAGE)
    // -----------------------------------------------------------------------

    fn demo_installed(&self) -> Installed {
        Installed {
            root: self.env.default_install_dir(),
            version: VERSION.into(),
            components: Component::ALL.to_vec(),
            options: Options::default(),
        }
    }

    fn demo_uninstall_log(&self) {
        let mut s = lock(&self.progress);
        let lines: &[(Level, &str)] = &[
            (Level::Info, "Uninstalling Download Manager, Torrent, Chrome extension, Firefox extension from C:\\Users\\you\\AppData\\Local\\Programs\\Zenless"),
            (Level::Ok, "Zenless Download Manager has exited."),
            (Level::Ok, "Removed Zenless Download Manager."),
            (Level::Info, "Magnet / .torrent associations removed."),
        ];
        for (level, text) in lines {
            s.log.push(LogLine { level: *level, text: (*text).into() });
        }
        s.status = "Removing Zenless Torrent…".into();
        s.progress = 0.45;
    }

    fn demo_log(&self, failed: bool) {
        let mut s = lock(&self.progress);
        let lines: &[(Level, &str)] = &[
            (Level::Info, "Zenless Setup 0.1.0"),
            (Level::Info, "Install folder: C:\\Users\\you\\AppData\\Local\\Programs\\Zenless"),
            (Level::Info, "Zenless Download Manager is running; asking it to exit…"),
            (Level::Ok, "Zenless Download Manager has exited."),
            (Level::Info, "Unpacking zenless-dm.exe (10.5 MB)"),
            (Level::Info, "Unpacking zenless-chrome-extension.zip (67.3 KB)"),
            (Level::Info, "Downloading https://github.com/zenless-inc/zenless-torrent-client/releases/latest/download/zenless-torrent.exe"),
            (Level::Info, "Downloaded zenless-torrent.exe (14.2 MB), SHA-256 3f9a…c21e"),
            (Level::Ok, "Checksum matches the published SHA-256."),
        ];
        // The failed demo stops at the download.
        let keep = if failed { lines.len() - 2 } else { lines.len() };
        for (level, text) in &lines[..keep] {
            s.log.push(LogLine { level: *level, text: (*text).into() });
        }
        if failed {
            s.log.push(LogLine {
                level: Level::Error,
                text: "download failed: HTTP 404 Not Found for https://github.com/zenless-inc/zenless-torrent-client/releases/latest/download/zenless-torrent.exe".into(),
            });
            s.log.push(LogLine { level: Level::Warn, text: "Rolling back…".into() });
            s.log.push(LogLine { level: Level::Info, text: "All changes were rolled back.".into() });
        } else {
            s.log.push(LogLine { level: Level::Ok, text: "Zenless Download Manager → Download Manager\\zenless-dm.exe".into() });
            s.log.push(LogLine { level: Level::Ok, text: "Chrome extension (36 files) → Browser Extensions\\Chrome".into() });
        }
        s.status = "Installing Zenless Torrent…".into();
        s.progress = 0.78;
    }

    fn apply_demo(&mut self, page: &str) {
        let page = page.trim().to_ascii_lowercase();
        let outcome = || InstallOutcome {
            root: self.env.default_install_dir(),
            components: Component::ALL.to_vec(),
            warnings: 0,
        };
        match page.as_str() {
            "1" | "welcome" => self.page = Page::Welcome,
            "welcome-installed" | "modify" => {
                self.installed = Some(self.demo_installed());
                self.page = Page::Welcome;
            }
            "2" | "license" => self.page = Page::License,
            "3" | "components" => self.page = Page::Components,
            "components-modify" => {
                self.installed = Some(self.demo_installed());
                self.begin_modify();
                self.selected.retain(|c| *c != Component::Firefox);
            }
            "4" | "options" => self.page = Page::Options,
            "5" | "appearance" => self.page = Page::Appearance,
            "6" | "installing" => {
                self.license_ok = true;
                self.demo_log(false);
                self.demo_running = true;
                self.page = Page::Installing;
            }
            "failed" => {
                self.demo_log(true);
                self.install_result = Some(Err("download failed: HTTP 404 Not Found".into()));
                self.page = Page::Installing;
            }
            "7" | "finish" => {
                self.install_result = Some(Ok(outcome()));
                self.launch_dm = true;
                self.page = Page::Finish;
            }
            "uninstall" => {
                self.installed = Some(self.demo_installed());
                self.un_selected = Component::ALL.to_vec();
                self.page = Page::UninstallChoose;
            }
            "uninstalling" => {
                self.installed = Some(self.demo_installed());
                self.demo_uninstall_log();
                self.demo_running = true;
                self.page = Page::Uninstalling;
            }
            "uninstalled" => {
                self.uninstall_result =
                    Some(Ok(UninstallOutcome { full: true, remaining: vec![], warnings: 0, self_delete: None }));
                self.page = Page::UninstallDone;
            }
            other => eprintln!("unknown ZENLESS_INSTALLER_PAGE {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let rect = ui.max_rect();
        let painter = ui.painter();
        // subtle brand glow in the corner + right border
        widgets::glow(painter, rect.left_top() + vec2(40.0, 40.0), 150.0, p.accent, 0.35);
        painter.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, p.border));

        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            widgets::image(ui, &self.icons.setup, 38.0);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(RichText::new("Zenless Setup").size(16.0).color(p.text).strong());
                ui.label(RichText::new(format!("Version {VERSION}")).size(11.5).color(p.text_dim));
            });
        });
        ui.add_space(26.0);
        let (names, current) = self.steps();
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.set_width(SIDEBAR_W - 12.0);
                widgets::steps(ui, p, &names, current);
            });
        });

        // footer
        let footer = egui::Rect::from_min_max(pos2(rect.left() + 20.0, rect.bottom() - 58.0), rect.right_bottom() - vec2(16.0, 14.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                let link = ui.add(
                    egui::Label::new(RichText::new(format!("{}  zenless-suite.vercel.app", ph::GLOBE)).size(12.0).color(p.text_dim))
                        .sense(egui::Sense::click()),
                );
                if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.open_external("open the Zenless website", std::ffi::OsStr::new(WEBSITE));
                }
                if self.env.is_sandbox() {
                    kit::pill(ui, &format!("{}  SANDBOX MODE", ph::FLASK), p.warning)
                        .on_hover_text(format!("All changes go to {}", self.env.sandbox().map(|s| s.display().to_string()).unwrap_or_default()));
                }
            });
        });
    }

    fn header(ui: &mut egui::Ui, p: &Palette, title: &str, subtitle: &str) {
        ui.label(RichText::new(title).size(23.0).color(p.text).strong());
        ui.add_space(2.0);
        ui.label(RichText::new(subtitle).size(13.5).color(p.text_dim));
        ui.add_space(16.0);
    }

    fn content(&mut self, ui: &mut egui::Ui, p: &Palette, frame: &eframe::Frame) {
        match self.page {
            Page::Welcome => self.page_welcome(ui, p),
            Page::License => {
                Self::header(ui, p, "License agreement", "Zenless is free and open-source software, released under the MIT License.");
                self.page_license(ui, p);
            }
            Page::Components => {
                let sub = if self.modify {
                    "Tick what you want to keep or add. Unticked components will be removed."
                } else {
                    "Pick the apps and browser integrations to install. They are separate apps — take any mix."
                };
                Self::header(ui, p, "Choose components", sub);
                self.page_components(ui, p);
            }
            Page::Options => {
                Self::header(ui, p, "Install options", "Where Zenless goes and how it fits into Windows. No administrator rights needed.");
                self.page_options(ui, p, frame);
            }
            Page::Appearance => {
                Self::header(ui, p, "Choose a look", "Every Zenless app shares this theme. Click one to preview it — you can change it later in each app.");
                self.page_appearance(ui);
            }
            Page::Installing => self.page_installing(ui, p),
            Page::Finish => self.page_finish(ui, p),
            Page::UninstallChoose => self.page_uninstall_choose(ui, p),
            Page::Uninstalling => self.page_uninstalling(ui, p),
            Page::UninstallDone => self.page_uninstall_done(ui, p),
        }
    }

    fn nav(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let rect = ui.max_rect();
        ui.painter().line_segment(
            [pos2(rect.left(), rect.top()), pos2(rect.right(), rect.top())],
            Stroke::new(1.0, alpha(p.border, 0.8)),
        );
        let ctx = ui.ctx().clone();
        ui.horizontal_centered(|ui| {
            let wizard_page = matches!(
                self.page,
                Page::Welcome | Page::License | Page::Components | Page::Options | Page::Appearance | Page::UninstallChoose
            );
            if wizard_page && widgets::ghost(ui, p.text_dim, "Cancel").clicked() {
                self.modal = Some(Modal::ConfirmExit);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let mut hint: Option<(String, Color32)> = None;
                match self.page {
                    Page::Welcome => {
                        if self.installed.is_none() && widgets::primary(ui, p, &format!("Get started  {}", ph::ARROW_RIGHT), true).clicked() {
                            self.next(&ctx);
                        }
                    }
                    Page::License => {
                        if widgets::primary(ui, p, &format!("Next  {}", ph::ARROW_RIGHT), self.license_ok).clicked() {
                            self.next(&ctx);
                        }
                        if widgets::secondary(ui, p, "Back").clicked() {
                            self.back();
                        }
                        if !self.license_ok {
                            hint = Some(("Accept the license to continue".into(), p.text_dim));
                        }
                    }
                    Page::Components => {
                        let ok = !self.selected.is_empty();
                        if widgets::primary(ui, p, &format!("Next  {}", ph::ARROW_RIGHT), ok).clicked() {
                            self.next(&ctx);
                        }
                        if widgets::secondary(ui, p, "Back").clicked() {
                            self.back();
                        }
                        if !ok {
                            let text = if self.modify {
                                "Select at least one — or use Uninstall to remove everything"
                            } else {
                                "Select at least one component"
                            };
                            hint = Some((text.into(), p.warning));
                        }
                    }
                    Page::Options => {
                        let err = install::validate_root(std::path::Path::new(self.dir.trim())).err();
                        if widgets::primary(ui, p, &format!("Next  {}", ph::ARROW_RIGHT), err.is_none()).clicked() {
                            self.next(&ctx);
                        }
                        if widgets::secondary(ui, p, "Back").clicked() {
                            self.back();
                        }
                    }
                    Page::Appearance => {
                        let label = if self.modify { format!("{}  Update", ph::ARROWS_CLOCKWISE) } else { format!("{}  Install", ph::DOWNLOAD_SIMPLE) };
                        if widgets::primary(ui, p, &label, true).clicked() {
                            self.next(&ctx);
                        }
                        if widgets::secondary(ui, p, "Back").clicked() {
                            self.back();
                        }
                        if crate::payload::needs_network(&self.selected) {
                            hint = Some((format!("{}  Some parts will be downloaded", ph::CLOUD_ARROW_DOWN), p.text_dim));
                        }
                    }
                    Page::Installing => {
                        if self.is_busy() {
                            if widgets::secondary(ui, p, "Cancel").clicked() {
                                self.modal = Some(Modal::ConfirmCancel);
                            }
                        } else {
                            if widgets::primary(ui, p, &format!("{}  Try again", ph::ARROW_LEFT), true).clicked() {
                                self.back();
                            }
                            if widgets::secondary(ui, p, "Close").clicked() {
                                self.exit(&ctx);
                            }
                        }
                    }
                    Page::Finish => {
                        if widgets::primary(ui, p, "Finish", true).clicked() {
                            self.launch_selected();
                            self.exit(&ctx);
                        }
                    }
                    Page::UninstallChoose => {
                        let can = self.installed.is_some() && !self.un_selected.is_empty();
                        if self.installed.is_some() {
                            if widgets::danger(ui, p, &format!("{}  Uninstall", ph::TRASH), can).clicked() {
                                self.start_uninstall(&ctx);
                            }
                        } else if widgets::primary(ui, p, "Close", true).clicked() {
                            self.exit(&ctx);
                        }
                        if self.un_from_welcome && widgets::secondary(ui, p, "Back").clicked() {
                            self.back();
                        }
                    }
                    Page::Uninstalling => {
                        ui.add(egui::Spinner::new().size(18.0).color(p.accent));
                        hint = Some(("Removing Zenless…".into(), p.text_dim));
                    }
                    Page::UninstallDone => {
                        if widgets::primary(ui, p, "Close", true).clicked() {
                            self.exit(&ctx);
                        }
                    }
                }
                // toast / hint text, right-aligned next to the buttons
                if let Some((text, at)) = &self.toast {
                    if at.elapsed() < Duration::from_secs(4) {
                        hint = Some((text.clone(), p.accent2));
                        ctx.request_repaint_after(Duration::from_millis(250));
                    } else {
                        self.toast = None;
                    }
                }
                if let Some((text, color)) = hint {
                    ui.add_space(6.0);
                    ui.label(RichText::new(text).size(12.5).color(color));
                }
            });
        });
    }

    fn modals(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(modal) = self.modal else { return };
        let (title, body, stay, leave) = match modal {
            Modal::ConfirmExit => (
                "Exit Zenless Setup?",
                "Nothing has been changed on this PC yet. You can run Setup again at any time.",
                "Stay",
                "Exit Setup",
            ),
            Modal::ConfirmCancel => (
                "Stop the installation?",
                "Files copied so far will be removed and every change rolled back.",
                "Keep installing",
                "Stop and roll back",
            ),
        };
        let resp = egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(ph::WARNING_CIRCLE).size(22.0).color(p.warning));
                ui.label(RichText::new(title).size(17.0).color(p.text).strong());
            });
            ui.add_space(6.0);
            ui.label(RichText::new(body).color(p.text_dim));
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::danger(ui, p, leave, true).clicked() {
                    match modal {
                        Modal::ConfirmExit => self.exit(ctx),
                        Modal::ConfirmCancel => self.request_cancel(),
                    }
                    ui.close();
                }
                if widgets::secondary(ui, p, stay).clicked() {
                    ui.close();
                }
            });
        });
        if resp.should_close() {
            self.modal = None;
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if self.task.is_some() {
            // Never quit in the middle of an install/uninstall.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.page == Page::Installing {
                self.modal = Some(Modal::ConfirmCancel);
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.shot.tick(ui.ctx());
        let ctx = ui.ctx().clone();
        self.poll_task();
        self.handle_close_request(&ctx);
        let p = self.look.current.palette.clone();

        egui::Panel::left("zen_steps")
            .exact_size(SIDEBAR_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(p.surface))
            .show(ui, |ui| self.sidebar(ui, &p));
        egui::Panel::bottom("zen_nav")
            .exact_size(NAV_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(MARGIN, 0)))
            .show(ui, |ui| self.nav(ui, &p));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin { left: MARGIN, right: MARGIN, top: 24, bottom: 6 }))
            .show(ui, |ui| self.content(ui, &p, frame));

        // Brand gradient line along the top edge of the window.
        let screen = ctx.content_rect();
        let top = egui::Rect::from_min_size(screen.min, vec2(screen.width(), 2.0));
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("zen_topline")));
        widgets::gradient_rect(&painter, top, p.accent, p.accent2);

        self.modals(&ctx, &p);
        if self.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(cmd) = self.self_delete.take()
            && let Err(e) = platform::spawn_hidden_cmd(&cmd)
        {
            eprintln!("could not schedule removal of the install folder: {e}");
        }
    }
}

/// Small helpers shared by the page modules.
pub(crate) fn approx_size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 { format!("~{mb:.0} MB") } else { format!("~{:.0} KB", (bytes as f64 / 1024.0).max(1.0)) }
}

pub(crate) fn section(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(11.0).strong().color(mix(p.text_dim, p.text, 0.15)));
    ui.add_space(4.0);
}
