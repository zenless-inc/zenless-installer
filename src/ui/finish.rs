//! Finish page (launch + browser extension setup) and the uninstall pages.
//!
//! Chromium browsers ignore `chrome://extensions` (and `edge://…` etc.) when
//! another program passes it on their command line: they open a New Tab page
//! instead. So "Open <browser>" copies the address to the clipboard, opens the
//! browser on the website's setup guide and tells the user to paste it. The
//! unpacked extension sits in a hidden AppData folder; "Show folder" opens
//! Explorer with it selected so it can be dragged onto the extensions page.

use super::widgets;
use super::{App, lock, section};
use crate::components::{CHROMIUM_GUIDE, Component, Layout, WEBSITE};
use crate::install::{self, CHROME_WEB_STORE_ID, FIREFOX_AMO_URL};
use crate::platform::{self, Browser, BrowserKind};
use crate::shared::theme::{Palette, alpha, mix};
use eframe::egui::{self, Align2, CornerRadius, FontId, Margin, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use egui_phosphor::regular as ph;
use std::ffi::OsStr;
use std::path::Path;

/// Keys to press after an address was copied.
const PASTE_KEYS: &str = "Ctrl+L, Ctrl+V, Enter";

fn browser_icon(kind: BrowserKind) -> &'static str {
    match kind {
        BrowserKind::Chrome => ph::GOOGLE_CHROME_LOGO,
        BrowserKind::Firefox => ph::GLOBE_HEMISPHERE_WEST,
        BrowserKind::Edge => ph::COMPASS,
        _ => ph::GLOBE,
    }
}

/// Card frame used for the browser sections.
fn section_card(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(16, 13))
}

/// Path shown in an input-like well, filling `width` (frame included).
fn path_well(ui: &mut egui::Ui, p: &Palette, path: &str, width: f32) {
    egui::Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(width - 22.0);
            ui.add(egui::Label::new(RichText::new(path).monospace().size(12.0).color(p.text)).truncate())
                .on_hover_text(path);
        });
}

/// A numbered step: badge and title, with `body` below the title.
fn step(ui: &mut egui::Ui, p: &Palette, n: usize, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
        let painter = ui.painter();
        painter.circle_filled(rect.center(), 10.5, p.accent);
        painter.text(rect.center(), Align2::CENTER_CENTER, n.to_string(), FontId::proportional(11.5), p.accent_fg);
        ui.add_space(2.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add_space(2.0);
            ui.label(RichText::new(title).size(13.5).color(p.text).strong());
            body(ui);
        });
    });
}

/// Secondary explanation text (wraps).
fn note(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(p.text_dim));
}

/// Highlighted "the address is on your clipboard" line.
fn copied_hint(ui: &mut egui::Ui, p: &Palette, text: &str) {
    egui::Frame::new()
        .fill(alpha(p.accent2, 0.10))
        .stroke(Stroke::new(1.0, alpha(p.accent2, 0.40)))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(9, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(format!("{}  {text}", ph::CLIPBOARD_TEXT)).size(12.0).color(p.text));
        });
}

/// Width of the Explorer + copy buttons next to a path well.
fn path_buttons_width(ui: &egui::Ui, labels: &[&str]) -> f32 {
    let font = FontId::proportional(13.0);
    let spacing = ui.spacing().item_spacing.x;
    labels
        .iter()
        .map(|l| {
            let w = ui.painter().layout_no_wrap(format!("{}  {l}", ph::COPY), font.clone(), egui::Color32::WHITE).size().x;
            w + 2.0 * ui.spacing().button_padding.x + spacing
        })
        .sum()
}

impl App {
    fn browsers(&mut self) -> Vec<Browser> {
        self.browsers.get_or_insert_with(platform::detect_browsers).clone()
    }

    pub(super) fn page_finish(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let Some(Ok(outcome)) = self.install_result.clone() else { return };
        let layout = Layout::new(&outcome.root);

        // Header with a success badge.
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
            let painter = ui.painter();
            widgets::glow(painter, rect.center(), 44.0, p.success, 0.6);
            painter.circle_filled(rect.center(), 21.0, alpha(p.success, 0.18));
            painter.text(rect.center(), Align2::CENTER_CENTER, ph::CHECK_FAT, FontId::proportional(21.0), p.success);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(if self.modify { "Zenless is up to date" } else { "Zenless is ready" }).size(23.0).color(p.text).strong());
                ui.label(RichText::new(format!("Installed to {}", outcome.root.display())).size(13.0).color(p.text_dim));
            });
        });
        ui.add_space(10.0);

        let apps: Vec<Component> = outcome.components.iter().copied().filter(|c| c.is_app()).collect();
        let has_chrome = outcome.components.contains(&Component::Chrome);
        let has_firefox = outcome.components.contains(&Component::Firefox);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // Keep the card strokes inside the scroll viewport.
            ui.set_max_width(ui.available_width() - 3.0);
            if !apps.is_empty() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    if apps.contains(&Component::Dm) {
                        widgets::check_row(ui, p, &mut self.launch_dm, "Launch Download Manager");
                    }
                    if apps.contains(&Component::Torrent) {
                        widgets::check_row(ui, p, &mut self.launch_torrent, "Launch Zenless Torrent");
                    }
                });
                ui.add_space(10.0);
            }

            if has_chrome || has_firefox {
                section(ui, p, "Connect your browser");
            }
            if has_chrome {
                self.chrome_card(ui, p, &layout);
                ui.add_space(10.0);
            }
            if has_firefox {
                self.firefox_card(ui, p, &layout);
            }
            if !has_chrome && !has_firefox {
                widgets::banner(
                    ui,
                    p,
                    ph::PUZZLE_PIECE,
                    p.info,
                    "Want downloads from your browser?",
                    "Run Setup again and add the Chrome or Firefox extension to send downloads to Zenless with one click.",
                );
            }
        });
    }

    /// "Open <browser>": copy its extensions-page address, open it on the guide.
    fn open_chromium(&mut self, ctx: &egui::Context, b: &Browser) {
        let url = b.kind.extensions_url();
        ctx.copy_text(url.to_owned());
        self.copied_for = Some(b.kind);
        let what = format!("open {} on the setup guide", b.kind.name());
        if self.run_external(&what, &b.exe, &[OsStr::new(CHROMIUM_GUIDE)]) {
            self.toast(format!("{}  {url} copied — paste it into the address bar", ph::CHECK));
        }
    }

    fn chrome_card(&mut self, ui: &mut egui::Ui, p: &Palette, layout: &Layout) {
        let folder = layout.chrome_dir();
        let folder_s = folder.display().to_string();
        let chromium: Vec<Browser> = self.browsers().into_iter().filter(|b| b.kind.is_chromium()).collect();
        let copied = self.copied_for.filter(|k| k.is_chromium());
        section_card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(ph::GOOGLE_CHROME_LOGO).size(17.0).color(p.accent));
                ui.label(RichText::new("Chrome, Edge, Brave, Vivaldi, Opera, Helium").size(14.5).color(p.text).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let note = if CHROME_WEB_STORE_ID.is_some() {
                        "Chrome offers to enable it on its next start"
                    } else {
                        "Load it once — it stays"
                    };
                    ui.label(RichText::new(note).size(12.0).color(p.text_dim));
                });
            });
            ui.add_space(10.0);

            step(ui, p, 1, "Open the extensions page", |ui| {
                if chromium.is_empty() {
                    note(ui, p, "No Chromium browser was found. Type chrome://extensions (edge://extensions in Edge) into your browser's address bar.");
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for b in &chromium {
                        let label = format!("Open {}", b.kind.short_name());
                        let hover = format!("Copies {} and opens {}", b.kind.extensions_url(), b.kind.name());
                        if widgets::action(ui, p, browser_icon(b.kind), &label).on_hover_text(hover).clicked() {
                            self.open_chromium(ui.ctx(), b);
                        }
                    }
                });
                match copied {
                    Some(kind) => copied_hint(
                        ui,
                        p,
                        &format!(
                            "{} is copied. In {}, paste it into the address bar: {PASTE_KEYS}.",
                            kind.extensions_url(),
                            kind.short_name()
                        ),
                    ),
                    None => note(
                        ui,
                        p,
                        &format!(
                            "Browsers don't let other apps open their extensions page, so the button copies its address \
                             and opens the browser: paste it into the address bar ({PASTE_KEYS})."
                        ),
                    ),
                }
            });
            ui.add_space(8.0);
            step(ui, p, 2, "Turn on Developer mode", |ui| {
                note(ui, p, "It's a switch on the extensions page: top right in most browsers, in the left sidebar in Edge.");
            });
            ui.add_space(8.0);
            step(ui, p, 3, "Drag the Chrome folder onto the page", |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let buttons = path_buttons_width(ui, &["Show folder", "Copy folder path"]);
                    path_well(ui, p, &folder_s, (ui.available_width() - buttons - 6.0).max(120.0));
                    if widgets::action(ui, p, ph::FOLDER_OPEN, "Show folder")
                        .on_hover_text("Opens Explorer with the Chrome folder selected, ready to drag")
                        .clicked()
                    {
                        self.reveal("show the Chrome folder in Explorer", &folder);
                    }
                    if widgets::action(ui, p, ph::COPY, "Copy folder path").clicked() {
                        ui.ctx().copy_text(folder_s.clone());
                        self.toast(format!("{}  Folder path copied", ph::CHECK));
                    }
                });
                note(ui, p, "Show folder selects it in Explorer. Or click Load unpacked and paste the folder path.");
            });
        });
    }

    fn firefox_card(&mut self, ui: &mut egui::Ui, p: &Palette, layout: &Layout) {
        let xpi = layout.firefox_xpi();
        let firefox = self.browsers().into_iter().find(|b| b.kind == BrowserKind::Firefox);
        let signed = *self.xpi_signed.get_or_insert_with(|| install::xpi_is_signed(&xpi).unwrap_or(false));
        section_card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(ph::GLOBE_HEMISPHERE_WEST).size(17.0).color(p.accent));
                ui.label(RichText::new("Firefox").size(14.5).color(p.text).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if signed {
                        ui.label(RichText::new(format!("{}  Signed by Mozilla", ph::SEAL_CHECK)).size(12.0).color(p.success));
                    } else {
                        ui.label(RichText::new(format!("{}  Mozilla signing pending", ph::HOURGLASS_MEDIUM)).size(12.0).color(p.warning))
                            .on_hover_text("Firefox only installs add-ons signed by Mozilla permanently.");
                    }
                });
            });
            ui.add_space(8.0);
            if signed {
                self.firefox_signed(ui, p, &xpi, firefox.as_ref());
            } else {
                self.firefox_unsigned(ui, p, &xpi, firefox.as_ref());
            }
        });
    }

    /// Path well with "Show file" and "Copy file path" for the .xpi.
    fn xpi_row(&mut self, ui: &mut egui::Ui, p: &Palette, xpi: &Path) {
        let xpi_s = xpi.display().to_string();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let buttons = path_buttons_width(ui, &["Show file", "Copy file path"]);
            path_well(ui, p, &xpi_s, (ui.available_width() - buttons - 6.0).max(120.0));
            if widgets::action(ui, p, ph::FOLDER_OPEN, "Show file").on_hover_text("Opens Explorer with the .xpi file selected").clicked() {
                self.reveal("show the .xpi file in Explorer", xpi);
            }
            if widgets::action(ui, p, ph::COPY, "Copy file path").clicked() {
                ui.ctx().copy_text(xpi_s.clone());
                self.toast(format!("{}  File path copied", ph::CHECK));
            }
        });
    }

    /// Signed add-on: Firefox installs it permanently from the .xpi.
    fn firefox_signed(&mut self, ui: &mut egui::Ui, p: &Palette, xpi: &Path, firefox: Option<&Browser>) {
        match firefox {
            Some(ff) => {
                if widgets::action(ui, p, ph::DOWNLOAD_SIMPLE, "Install in Firefox").clicked() {
                    let target = FIREFOX_AMO_URL.map(|u| u.into()).unwrap_or_else(|| xpi.as_os_str().to_owned());
                    self.run_external("open the add-on in Firefox", &ff.exe, &[target.as_os_str()]);
                }
                note(ui, p, "Firefox asks you to confirm adding Zenless Browser Integration, and keeps it updated afterwards.");
            }
            None => note(ui, p, "Firefox was not found on this PC. After installing it, drag the .xpi file into a Firefox window."),
        }
        ui.add_space(4.0);
        self.xpi_row(ui, p, xpi);
    }

    /// Unsigned add-on: Firefox can only load it temporarily (about:debugging).
    fn firefox_unsigned(&mut self, ui: &mut egui::Ui, p: &Palette, xpi: &Path, firefox: Option<&Browser>) {
        let url = BrowserKind::Firefox.extensions_url();
        let copied = self.copied_for == Some(BrowserKind::Firefox);
        note(
            ui,
            p,
            "Firefox only installs add-ons signed by Mozilla permanently, and the signature for Zenless is still pending. \
             Until then, load it temporarily: it stays until Firefox is closed.",
        );
        ui.add_space(8.0);
        step(ui, p, 1, "Open \"This Firefox\" in about:debugging", |ui| {
            match firefox {
                Some(ff) => {
                    if widgets::action(ui, p, ph::FLASK, "Load temporarily")
                        .on_hover_text(format!("Opens {url} in Firefox and copies that address"))
                        .clicked()
                    {
                        ui.ctx().copy_text(url.to_owned());
                        self.copied_for = Some(BrowserKind::Firefox);
                        if self.run_external("open about:debugging in Firefox", &ff.exe, &[OsStr::new(url)]) {
                            self.toast(format!("{}  Opening about:debugging in Firefox", ph::CHECK));
                        }
                    }
                    if copied {
                        copied_hint(ui, p, &format!("{url} is copied. If Firefox shows another page, paste it into the address bar: {PASTE_KEYS}."));
                    } else {
                        note(ui, p, &format!("Opens {url} (the address is copied too, in case Firefox shows another page: {PASTE_KEYS})."));
                    }
                }
                None => note(ui, p, &format!("Firefox was not found on this PC. After installing it, open {url} in Firefox.")),
            }
        });
        ui.add_space(8.0);
        step(ui, p, 2, "Click \"Load Temporary Add-on…\" and pick the .xpi file", |ui| {
            self.xpi_row(ui, p, xpi);
            note(ui, p, "Paste the copied file path into the file picker's File name box and press Enter.");
        });
    }

    // -----------------------------------------------------------------------
    // Uninstall
    // -----------------------------------------------------------------------

    pub(super) fn page_uninstall_choose(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let Some(installed) = self.installed.clone() else {
            Self::header(ui, p, "Uninstall Zenless", "There is nothing to remove.");
            widgets::banner(ui, p, ph::INFO, p.info, "Zenless Suite is not installed", "No Zenless installation was found for this account.");
            return;
        };
        Self::header(ui, p, "Uninstall Zenless", &format!("Choose what to remove from {}.", installed.root.display()));
        let w = ui.available_width();
        let mut toggle = None;
        for &c in &installed.components {
            let selected = self.un_selected.contains(&c);
            let resp = select_row(ui, p, self.card_icon(c), c.name(), c.description(), selected, w);
            if resp.clicked() {
                toggle = Some(c);
            }
            ui.add_space(6.0);
        }
        if let Some(c) = toggle {
            if let Some(i) = self.un_selected.iter().position(|x| *x == c) {
                self.un_selected.remove(i);
            } else {
                self.un_selected.push(c);
                self.un_selected.sort();
            }
        }
        ui.add_space(8.0);
        let cfg = self.env.config_dir();
        // Show the familiar %APPDATA% form for the real folder.
        let cfg_text = match dirs::config_dir().and_then(|base| cfg.strip_prefix(base).ok().map(|r| r.to_path_buf())) {
            Some(rel) if !self.env.is_sandbox() => format!("%APPDATA%\\{}", rel.display()),
            _ => cfg.display().to_string(),
        };
        let what = if self.un_selected.len() == installed.components.len() {
            format!("Deletes {cfg_text} (settings, themes, download history). Downloaded files are never touched.")
        } else {
            "Deletes the settings and history of the selected apps. Downloaded files are never touched.".to_owned()
        };
        widgets::toggle_row(
            ui,
            p,
            &mut self.un_settings,
            "Also remove my settings and download history",
            &what,
            !self.un_selected.is_empty(),
        );
        if self.un_selected.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("Select at least one component to remove.").size(12.5).color(p.warning));
        }
    }

    pub(super) fn page_uninstalling(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let (status, progress, log) = {
            let s = lock(&self.progress);
            (s.status.clone(), s.progress, s.log.clone())
        };
        Self::header(ui, p, "Removing Zenless", if status.is_empty() { "Starting…" } else { &status });
        widgets_progress(ui, p, progress, p.danger);
        ui.add_space(18.0);
        section(ui, p, "Details");
        let h = ui.available_height() - 22.0;
        widgets::log_view(ui, p, &log, h.max(80.0));
    }

    pub(super) fn page_uninstall_done(&mut self, ui: &mut egui::Ui, p: &Palette) {
        match self.uninstall_result.clone() {
            Some(Ok(o)) => {
                let block = if o.warnings > 0 { 240.0 } else { 220.0 };
                ui.add_space(if o.warnings > 0 { 16.0 } else { ((ui.available_height() - block) / 2.0 - 10.0).max(16.0) });
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(84.0), Sense::hover());
                    let painter = ui.painter();
                    widgets::glow(painter, rect.center(), 90.0, p.accent2, 0.5);
                    painter.circle_filled(rect.center(), 34.0, alpha(p.success, 0.16));
                    painter.text(rect.center(), Align2::CENTER_CENTER, ph::CHECK_FAT, FontId::proportional(32.0), p.success);
                    ui.add_space(16.0);
                    let title = if o.full { "Zenless was removed" } else { "The selected components were removed" };
                    ui.label(RichText::new(title).size(24.0).color(p.text).strong());
                    ui.add_space(6.0);
                    let body = if o.full {
                        "Thanks for giving Zenless a try. Your downloaded files were left untouched.".to_owned()
                    } else {
                        format!(
                            "Still installed: {}.",
                            o.remaining.iter().map(|c| c.short_name()).collect::<Vec<_>>().join(", ")
                        )
                    };
                    ui.label(RichText::new(body).size(13.5).color(p.text_dim));
                    ui.add_space(4.0);
                    if o.full {
                        ui.label(RichText::new(format!("You can reinstall any time from {}", WEBSITE.trim_start_matches("https://"))).size(12.5).color(p.text_dim));
                    }
                    if o.warnings > 0 {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(format!("{}  {} item(s) could not be removed — see the details below.", ph::WARNING, o.warnings))
                                .size(12.5)
                                .color(p.warning),
                        );
                    }
                });
                if o.warnings > 0 {
                    ui.add_space(12.0);
                    let log = lock(&self.progress).log.clone();
                    let h = ui.available_height() - 22.0;
                    widgets::log_view(ui, p, &log, h.max(60.0));
                }
            }
            Some(Err(e)) => {
                Self::header(ui, p, "Uninstall incomplete", "Some parts could not be removed.");
                widgets::banner(ui, p, ph::X_CIRCLE, p.danger, "Uninstall failed", &e);
                ui.add_space(12.0);
                let log = lock(&self.progress).log.clone();
                let h = ui.available_height() - 22.0;
                widgets::log_view(ui, p, &log, h.max(60.0));
            }
            None => {}
        }
    }
}

/// Thin progress bar with a gradient fill.
fn widgets_progress(ui: &mut egui::Ui, p: &Palette, progress: f32, color: egui::Color32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 8.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), mix(p.surface2, p.bg, 0.3));
    if progress > 0.0 {
        let mut fill = rect;
        fill.set_width((w * progress).max(8.0));
        painter.rect_filled(fill, CornerRadius::same(4), color);
    }
}

/// Compact selectable row (uninstall page).
fn select_row(ui: &mut egui::Ui, p: &Palette, icon: widgets::CardIcon, title: &str, desc: &str, selected: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 58.0), Sense::click());
    let t = ui.ctx().animate_bool_with_time(resp.id, selected, 0.12);
    let painter = ui.painter();
    let hovered = resp.hovered();
    let stroke = if selected {
        Stroke::new(1.3, mix(p.border, p.danger, 0.55 * t))
    } else if hovered {
        Stroke::new(1.0, mix(p.border, p.danger, 0.3))
    } else {
        Stroke::new(1.0, p.border)
    };
    painter.rect(rect, CornerRadius::same(10), mix(p.surface, p.danger, 0.03 * t), stroke, StrokeKind::Inside);
    let icon_rect = egui::Rect::from_min_size(pos2(rect.left() + 12.0, rect.center().y - 17.0), Vec2::splat(34.0));
    icon.paint(painter, p, icon_rect);
    painter.text(pos2(icon_rect.right() + 12.0, rect.center().y - 1.0), Align2::LEFT_BOTTOM, title, FontId::proportional(14.5), p.text);
    painter.text(pos2(icon_rect.right() + 12.0, rect.center().y + 2.0), Align2::LEFT_TOP, desc, FontId::proportional(12.0), p.text_dim);
    // check box on the right
    let c = pos2(rect.right() - 24.0, rect.center().y);
    painter.circle_filled(c, 10.0, mix(p.surface2, p.danger, t));
    painter.circle_stroke(c, 10.0, Stroke::new(1.2, mix(p.border, p.danger, t)));
    if t > 0.05 {
        painter.text(c, Align2::CENTER_CENTER, ph::CHECK, FontId::proportional(12.5), alpha(egui::Color32::WHITE, t));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
