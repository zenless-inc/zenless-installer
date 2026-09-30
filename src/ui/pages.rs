//! Wizard pages before and during installation.

use super::widgets::{self, CardIcon, CardTag};
use super::{App, approx_size, lock, section};
use crate::components::{Component, VERSION};
use crate::install;
use crate::platform;
use crate::shared::kit::human_bytes;
use crate::shared::theme::{self, Palette, mix};
use eframe::egui::{self, Align2, CornerRadius, FontId, Margin, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use egui_phosphor::regular as ph;
use std::path::Path;

const LICENSE: &str = include_str!("../../LICENSE");

/// Card titles (the full product names are too long for a half-width card).
fn card_title(c: Component) -> &'static str {
    match c {
        Component::Dm => "Download Manager",
        Component::Torrent => "Zenless Torrent",
        Component::Chrome => "Chromium extension",
        Component::Firefox => "Firefox extension",
    }
}

/// Joins hard-wrapped lines into paragraphs so the text reflows.
fn reflow(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(|para| para.lines().map(str::trim).collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect()
}

impl App {
    pub(super) fn card_icon(&self, c: Component) -> CardIcon<'_> {
        let (tex, badge) = match c {
            Component::Dm => (&self.icons.dm, None),
            Component::Torrent => (&self.icons.torrent, None),
            Component::Chrome => (&self.icons.zenless, Some(ph::GOOGLE_CHROME_LOGO)),
            Component::Firefox => (&self.icons.zenless, Some(ph::GLOBE_HEMISPHERE_WEST)),
        };
        CardIcon { tex, badge }
    }

    // -----------------------------------------------------------------------
    // Welcome
    // -----------------------------------------------------------------------

    pub(super) fn page_welcome(&mut self, ui: &mut egui::Ui, p: &Palette) {
        // The hero block is ~370 px tall (~330 when installed): center it vertically.
        let block = if self.installed.is_some() { 330.0 } else { 372.0 };
        ui.add_space(((ui.available_height() - block) / 2.0 - 8.0).clamp(4.0, 60.0));
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
            widgets::glow(ui.painter(), rect.center(), 80.0, p.accent2, 0.45);
            widgets::paint_texture(ui.painter(), &self.icons.zenless, rect);
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.add_space(9.0);
                ui.label(RichText::new("Welcome to Zenless").size(28.0).color(p.text).strong());
                ui.add_space(1.0);
                ui.label(RichText::new("A calm, fast download suite for Windows.").size(14.5).color(p.text_dim));
            });
        });
        ui.add_space(20.0);

        let Some(installed) = self.installed.clone() else {
            ui.label(
                RichText::new(
                    "This wizard installs the Zenless apps you pick — just for your account, without \
                     administrator rights and without bundled extras. It only takes a minute.",
                )
                .size(13.5)
                .color(mix(p.text, p.text_dim, 0.35)),
            );
            ui.add_space(20.0);
            let w = (ui.available_width() - 20.0) / 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let tiles = [
                    (&self.icons.dm, None, "Download Manager", "Multi-connection downloads that pause, resume and never give up."),
                    (&self.icons.torrent, None, "Torrent", "Magnet links and .torrent files, handled calmly in the background."),
                    (&self.icons.zenless, Some(ph::BROWSERS), "Browser integration", "Catch downloads from Chrome, Edge, Brave, Vivaldi, Opera and Firefox."),
                ];
                for (tex, badge, title, desc) in tiles {
                    let (rect, _) = ui.allocate_exact_size(vec2(w, 132.0), Sense::hover());
                    let painter = ui.painter();
                    painter.rect(rect, CornerRadius::same(12), p.surface, Stroke::new(1.0, p.border), StrokeKind::Inside);
                    let icon = egui::Rect::from_min_size(rect.min + vec2(16.0, 16.0), Vec2::splat(38.0));
                    widgets::paint_texture(painter, tex, icon);
                    if let Some(b) = badge {
                        let c = icon.right_bottom() - vec2(2.0, 2.0);
                        painter.circle_filled(c, 10.0, p.surface2);
                        painter.circle_stroke(c, 10.0, Stroke::new(1.0, p.border));
                        painter.text(c, Align2::CENTER_CENTER, b, FontId::proportional(12.0), p.text);
                    }
                    painter.text(pos2(rect.left() + 16.0, icon.bottom() + 12.0), Align2::LEFT_TOP, title, FontId::proportional(14.5), p.text);
                    let g = painter.layout(desc.to_owned(), FontId::proportional(12.0), p.text_dim, rect.width() - 32.0);
                    painter.galley(pos2(rect.left() + 16.0, icon.bottom() + 33.0), g, p.text_dim);
                }
            });
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let bits = [
                    (ph::SHIELD_CHECK, "No admin rights"),
                    (ph::FEATHER, "Tiny & native"),
                    (ph::CODE, "Open source (MIT)"),
                    (ph::ARROW_COUNTER_CLOCKWISE, "Clean uninstall"),
                ];
                for (icon, text) in bits {
                    ui.label(RichText::new(icon).size(14.0).color(p.accent));
                    ui.add_space(-10.0);
                    ui.label(RichText::new(text).size(12.5).color(p.text_dim));
                }
            });
            return;
        };

        // Already installed: Modify / Uninstall.
        let names: Vec<&str> = installed.components.iter().map(|c| c.short_name()).collect();
        let version = if installed.version.is_empty() { String::new() } else { format!(" {}", installed.version) };
        widgets::banner(
            ui,
            p,
            ph::CHECK_CIRCLE,
            p.success,
            &format!("Zenless Suite{version} is installed"),
            &format!("{} — {}", installed.root.display(), if names.is_empty() { "no components".into() } else { names.join(", ") }),
        );
        ui.add_space(16.0);
        let w = (ui.available_width() - 12.0) / 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let update = if installed.version != VERSION && !installed.version.is_empty() {
                format!("Update to {VERSION}, add or remove components, or change shortcuts and startup options.")
            } else {
                "Add or remove components, repair files, or change shortcuts and startup options.".to_owned()
            };
            if widgets::choice_card(ui, p, ph::ARROWS_CLOCKWISE, p.accent, "Modify or update", &update, vec2(w, 150.0)).clicked() {
                self.begin_modify();
            }
            if widgets::choice_card(
                ui,
                p,
                ph::TRASH,
                p.danger,
                "Uninstall",
                "Remove Zenless apps and browser extensions from this PC. Your downloads stay where they are.",
                vec2(w, 150.0),
            )
            .clicked()
            {
                self.begin_uninstall();
            }
        });
    }

    // -----------------------------------------------------------------------
    // License
    // -----------------------------------------------------------------------

    pub(super) fn page_license(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let height = ui.available_height() - 48.0;
        egui::Frame::new()
            .fill(p.input)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(18, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(height - 28.0).auto_shrink([false, false]).show(ui, |ui| {
                    for (i, para) in reflow(LICENSE).iter().enumerate() {
                        let text = RichText::new(para).size(13.0);
                        let text = if i == 0 { text.size(15.0).color(p.text).strong() } else { text.color(mix(p.text, p.text_dim, 0.25)) };
                        ui.label(text);
                        ui.add_space(8.0);
                    }
                });
            });
        ui.add_space(12.0);
        let mut ok = self.license_ok;
        if widgets::check_row(ui, p, &mut ok, "I accept the terms of the MIT License").changed() {
            self.license_ok = ok;
        }
    }

    // -----------------------------------------------------------------------
    // Components
    // -----------------------------------------------------------------------

    pub(super) fn page_components(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let installed = if self.modify { self.installed_components() } else { Vec::new() };
        let w = (ui.available_width() - 12.0) / 2.0;
        let mut toggle = None;
        for row in Component::ALL.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for &c in row {
                    let selected = self.selected.contains(&c);
                    let e = c.embedded();
                    let mut tags = Vec::new();
                    let removed = installed.contains(&c) && !selected;
                    if installed.contains(&c) {
                        tags.push(if selected {
                            CardTag { text: format!("{}  Installed", ph::CHECK), color: p.success }
                        } else {
                            CardTag { text: format!("{}  Will be removed", ph::TRASH), color: p.danger }
                        });
                    }
                    if !removed {
                        tags.push(if e.is_embedded() {
                        CardTag { text: human_bytes(e.size), color: p.text_dim }
                    } else {
                        CardTag { text: format!("{}  Download {}", ph::CLOUD_ARROW_DOWN, approx_size(c.approx_download_size())), color: p.info }
                        });
                    }
                    let resp = widgets::component_card(ui, p, self.card_icon(c), card_title(c), c.description(), &tags, selected, vec2(w, 140.0));
                    if resp.clicked() {
                        toggle = Some(c);
                    }
                }
            });
            ui.add_space(12.0);
        }
        if let Some(c) = toggle {
            if let Some(i) = self.selected.iter().position(|x| *x == c) {
                self.selected.remove(i);
            } else {
                self.selected.push(c);
                self.selected.sort();
            }
        }

        // Summary
        let disk: u64 = self.selected.iter().map(|c| c.install_size()).sum();
        let download: u64 = self
            .selected
            .iter()
            .filter(|c| !c.embedded().is_embedded())
            .map(|c| c.approx_download_size())
            .sum();
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(ph::HARD_DRIVE).size(15.0).color(p.text_dim));
            ui.label(RichText::new(format!("{} on disk", if disk == 0 { "0 B".into() } else { human_bytes(disk) })).size(13.0).color(p.text_dim));
            ui.add_space(12.0);
            if self.selected.is_empty() {
            } else if download > 0 {
                ui.label(RichText::new(ph::CLOUD_ARROW_DOWN).size(15.0).color(p.info));
                ui.label(RichText::new(format!("{} downloaded from GitHub during setup", approx_size(download))).size(13.0).color(p.text_dim));
            } else {
                ui.label(RichText::new(ph::SEAL_CHECK).size(15.0).color(p.success));
                ui.label(RichText::new("Everything is included — works offline").size(13.0).color(p.text_dim));
            }
        });
        let removals = self.removals();
        if !removals.is_empty() {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{}  {} will be removed. Your settings and downloads are kept.",
                    ph::INFO,
                    removals.iter().map(|c| c.short_name()).collect::<Vec<_>>().join(", ")
                ))
                .size(12.5)
                .color(p.warning),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Options
    // -----------------------------------------------------------------------

    pub(super) fn page_options(&mut self, ui: &mut egui::Ui, p: &Palette, frame: &eframe::Frame) {
        section(ui, p, "Install location");
        ui.horizontal(|ui| {
            let browse_w = if self.modify { 0.0 } else { 104.0 };
            let edit = egui::TextEdit::singleline(&mut self.dir)
                .desired_width(ui.available_width() - browse_w)
                .margin(Margin::symmetric(10, 7))
                .font(FontId::proportional(13.5))
                .interactive(!self.modify);
            ui.add(edit);
            if !self.modify && widgets::action(ui, p, ph::FOLDER_OPEN, "Browse…").clicked() {
                let start = Path::new(self.dir.trim());
                let mut dialog = rfd::FileDialog::new().set_title("Choose where to install Zenless").set_parent(frame);
                if let Some(existing) = start.ancestors().find(|a| a.is_dir()) {
                    dialog = dialog.set_directory(existing);
                }
                if let Some(mut picked) = dialog.pick_folder() {
                    // Always install into a dedicated folder, never straight into e.g. D:\Apps.
                    if !picked.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Zenless")) {
                        picked.push("Zenless");
                    }
                    self.dir = picked.display().to_string();
                }
            }
        });
        ui.add_space(6.0);
        let dir = self.dir.trim().to_owned();
        if self.free.as_ref().is_none_or(|(d, _)| *d != dir) {
            self.free = Some((dir.clone(), platform::free_space(Path::new(&dir))));
        }
        let required: u64 = self.selected.iter().map(|c| c.install_size()).sum::<u64>() + (4 << 20);
        let validation = install::validate_root(Path::new(&dir));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            match &validation {
                Err(e) => {
                    ui.label(RichText::new(ph::WARNING_CIRCLE).size(14.0).color(p.danger));
                    ui.label(RichText::new(e).size(12.5).color(p.danger));
                }
                Ok(()) => {
                    let free = self.free.as_ref().and_then(|(_, f)| *f);
                    let low = free.is_some_and(|f| f < required);
                    let color = if low { p.danger } else { p.text_dim };
                    ui.label(RichText::new(ph::HARD_DRIVE).size(14.0).color(color));
                    let drive = Path::new(&dir)
                        .components()
                        .next()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let text = match free {
                        Some(f) => format!("{} free on {drive}  ·  about {} needed", human_bytes(f), human_bytes(required)),
                        None => format!("About {} needed", human_bytes(required)),
                    };
                    ui.label(RichText::new(text).size(12.5).color(color));
                    if low {
                        ui.label(RichText::new("— not enough space").size(12.5).color(p.danger));
                    }
                }
            }
        });
        if self.modify {
            ui.label(RichText::new("Zenless is already installed here. To move it, uninstall it first.").size(12.0).color(p.text_dim));
        }

        ui.add_space(16.0);
        section(ui, p, "Shortcuts");
        let has_app = self.selected.iter().any(|c| c.is_app());
        ui.columns(2, |cols| {
            widgets::toggle_row(&mut cols[0], p, &mut self.options.start_menu, "Start menu", "", true);
            widgets::toggle_row(&mut cols[1], p, &mut self.options.desktop, "Desktop", "", has_app);
        });

        ui.add_space(12.0);
        section(ui, p, "Windows integration");
        let dm = self.selected.contains(&Component::Dm);
        let torrent = self.selected.contains(&Component::Torrent);
        let mut on = self.options.autostart_dm && dm;
        if widgets::toggle_row(ui, p, &mut on, "Start Download Manager with Windows", "Starts quietly in the tray, ready to catch downloads from your browser.", dm).changed() {
            self.options.autostart_dm = on;
        }
        let mut on = self.options.autostart_torrent && torrent;
        if widgets::toggle_row(ui, p, &mut on, "Start Zenless Torrent with Windows", "Keeps your torrents downloading and seeding in the background.", torrent).changed() {
            self.options.autostart_torrent = on;
        }
        let mut on = self.options.associate && torrent;
        if widgets::toggle_row(ui, p, &mut on, "Open magnet links and .torrent files with Zenless Torrent", "Makes Zenless Torrent the default torrent app for your account.", torrent).changed() {
            self.options.associate = on;
        }
    }

    // -----------------------------------------------------------------------
    // Appearance
    // -----------------------------------------------------------------------

    pub(super) fn page_appearance(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 8.0);
                for t in &self.look.themes {
                    if theme::theme_card(ui, t, t.name == self.look.current.name).clicked() {
                        clicked = Some(t.name.clone());
                    }
                }
            });
        });
        if let Some(name) = clicked {
            self.look.select(&ctx, &name);
        }
    }

    // -----------------------------------------------------------------------
    // Installing
    // -----------------------------------------------------------------------

    pub(super) fn page_installing(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let (status, progress, log) = {
            let s = lock(&self.progress);
            (s.status.clone(), s.progress, s.log.clone())
        };
        let failed = matches!(self.install_result, Some(Err(_)));
        let (title, subtitle) = if failed {
            ("Installation stopped", "Nothing was left behind — every change was rolled back.")
        } else if self.modify {
            ("Updating Zenless", "Hang tight, this only takes a moment.")
        } else {
            ("Installing Zenless", "Hang tight, this only takes a moment.")
        };
        Self::header(ui, p, title, subtitle);

        if let Some(Err(e)) = &self.install_result {
            if e == crate::report::CANCELLED {
                widgets::banner(ui, p, ph::PROHIBIT, p.warning, "Installation cancelled", "All changes were rolled back. You can go back and try again.");
            } else {
                widgets::banner(ui, p, ph::X_CIRCLE, p.danger, "Installation failed", e);
            }
            ui.add_space(12.0);
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{:.0}%", progress * 100.0)).size(30.0).color(p.text).strong());
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(13.0).color(p.accent));
                        ui.label(RichText::new(if status.is_empty() { "Starting…" } else { &status }).size(13.5).color(p.text_dim));
                    });
                });
            });
            ui.add_space(8.0);
            let w = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(vec2(w, 8.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(4), mix(p.surface2, p.bg, 0.3));
            if progress > 0.0 {
                let mut fill = rect;
                fill.set_width((w * progress).max(8.0));
                widgets::gradient_rect(painter, fill.shrink2(vec2(2.0, 0.0)), p.accent, mix(p.accent, p.accent2, progress));
                painter.circle_filled(pos2(fill.left() + 4.0, fill.center().y), 4.0, p.accent);
                painter.circle_filled(pos2(fill.right() - 4.0, fill.center().y), 4.0, mix(p.accent, p.accent2, progress));
            }
            ui.add_space(18.0);
        }
        section(ui, p, "Details");
        let h = ui.available_height() - 22.0;
        widgets::log_view(ui, p, &log, h.max(80.0));
    }
}
