//! Finish page (launch + browser extension setup) and the uninstall pages.

use super::widgets;
use super::{App, lock, section};
use crate::components::{Component, Layout, WEBSITE};
use crate::install::{CHROME_WEB_STORE_ID, FIREFOX_AMO_URL};
use crate::platform::{self, Browser, BrowserKind};
use crate::shared::theme::{Palette, alpha, mix};
use eframe::egui::{self, Align2, CornerRadius, FontId, Margin, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use egui_phosphor::regular as ph;
use std::ffi::OsStr;

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

/// Path shown in an input-like well.
fn path_well(ui: &mut egui::Ui, p: &Palette, path: &str, width: f32) {
    egui::Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(width);
            ui.add(egui::Label::new(RichText::new(path).monospace().size(12.0).color(p.text)).truncate());
        });
}

/// Numbered step chip: "1  Open the extensions page".
fn step(ui: &mut egui::Ui, p: &Palette, n: usize, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(12.5), p.text);
    let size = vec2(26.0 + galley.size().x + 12.0, 26.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(13), alpha(p.accent, 0.07), Stroke::new(1.0, alpha(p.accent, 0.25)), StrokeKind::Inside);
    let c = pos2(rect.left() + 13.0, rect.center().y);
    painter.circle_filled(c, 9.0, p.accent);
    painter.text(c, Align2::CENTER_CENTER, n.to_string(), FontId::proportional(11.0), p.accent_fg);
    painter.galley(pos2(rect.left() + 27.0, rect.center().y - galley.size().y / 2.0), galley, p.text);
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

    fn chrome_card(&mut self, ui: &mut egui::Ui, p: &Palette, layout: &Layout) {
        let folder = layout.chrome_dir();
        let folder_s = folder.display().to_string();
        let chromium: Vec<Browser> = self.browsers().into_iter().filter(|b| b.kind.is_chromium()).collect();
        section_card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(ph::GOOGLE_CHROME_LOGO).size(17.0).color(p.accent));
                ui.label(RichText::new("Chrome, Edge, Brave, Vivaldi & Opera").size(14.5).color(p.text).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let note = if CHROME_WEB_STORE_ID.is_some() {
                        "Chrome offers to enable it on its next start"
                    } else {
                        "Load it once — it stays installed"
                    };
                    ui.label(RichText::new(note).size(12.0).color(p.text_dim));
                });
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                step(ui, p, 1, "Turn on Developer mode");
                ui.label(RichText::new(ph::CARET_RIGHT).color(p.text_dim));
                step(ui, p, 2, "Load unpacked");
                ui.label(RichText::new(ph::CARET_RIGHT).color(p.text_dim));
                step(ui, p, 3, "Pick this folder");
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let buttons_w = 222.0;
                path_well(ui, p, &folder_s, (ui.available_width() - buttons_w - 20.0).max(120.0));
                if widgets::action(ui, p, ph::FOLDER_OPEN, "Open folder").clicked() {
                    self.open_external("open the extension folder", folder.as_os_str());
                }
                if widgets::action(ui, p, ph::COPY, "Copy path").clicked() {
                    ui.ctx().copy_text(folder_s.clone());
                    self.toast(format!("{}  Folder path copied", ph::CHECK));
                }
            });
            ui.add_space(8.0);
            if chromium.is_empty() {
                ui.label(
                    RichText::new("No Chromium browser found — open your browser's extensions page (for example chrome://extensions) yourself.")
                        .size(12.0)
                        .color(p.text_dim),
                );
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for b in &chromium {
                        let label = format!("Open {} extensions", short(b.kind));
                        if widgets::action(ui, p, browser_icon(b.kind), &label)
                            .on_hover_text(format!("{} {}", b.exe.display(), b.kind.extensions_url()))
                            .clicked()
                        {
                            let what = format!("open {} in {}", b.kind.extensions_url(), b.kind.name());
                            self.run_external(&what, &b.exe.clone(), &[OsStr::new(b.kind.extensions_url())]);
                        }
                    }
                });
            }
        });
    }

    fn firefox_card(&mut self, ui: &mut egui::Ui, p: &Palette, layout: &Layout) {
        let xpi = layout.firefox_xpi();
        let firefox = self.browsers().into_iter().find(|b| b.kind == BrowserKind::Firefox);
        section_card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(ph::GLOBE_HEMISPHERE_WEST).size(17.0).color(p.accent));
                ui.label(RichText::new("Firefox").size(14.5).color(p.text).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Unsigned test builds: load temporarily").size(12.0).color(p.text_dim))
                        .on_hover_text("Firefox only installs add-ons signed by Mozilla permanently. Unsigned builds can be loaded from about:debugging until Firefox restarts.");
                });
            });
            ui.add_space(8.0);
            match firefox {
                Some(ff) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        if widgets::action(ui, p, ph::DOWNLOAD_SIMPLE, "Install in Firefox").clicked() {
                            let target = FIREFOX_AMO_URL.map(|u| u.into()).unwrap_or_else(|| xpi.clone().into_os_string());
                            self.run_external("open the add-on in Firefox", &ff.exe, &[target.as_os_str()]);
                        }
                        if widgets::action(ui, p, ph::FLASK, "Load temporarily").clicked() {
                            let url = BrowserKind::Firefox.extensions_url();
                            self.run_external(&format!("open {url} in Firefox"), &ff.exe, &[OsStr::new(url)]);
                            ui.ctx().copy_text(xpi.display().to_string());
                            if !self.env.is_sandbox() {
                                self.toast("Choose \"Load Temporary Add-on\" and paste the copied path");
                            }
                        }
                        if widgets::action(ui, p, ph::FOLDER_OPEN, "Show file").clicked()
                            && let Some(dir) = xpi.parent()
                        {
                            self.open_external("open the extensions folder", dir.as_os_str());
                        }
                    });
                }
                None => {
                    ui.horizontal(|ui| {
                        path_well(ui, p, &xpi.display().to_string(), (ui.available_width() - 120.0).max(120.0));
                        if widgets::action(ui, p, ph::COPY, "Copy path").clicked() {
                            ui.ctx().copy_text(xpi.display().to_string());
                            self.toast(format!("{}  File path copied", ph::CHECK));
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new("Firefox was not found on this PC; drag the .xpi file into Firefox after installing it.").size(12.0).color(p.text_dim));
                }
            }
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

fn short(kind: BrowserKind) -> &'static str {
    match kind {
        BrowserKind::Chrome => "Chrome",
        BrowserKind::Edge => "Edge",
        other => other.name(),
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
