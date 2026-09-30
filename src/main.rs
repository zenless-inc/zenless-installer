//! Zenless Setup: graphical installer and uninstaller for the Zenless Suite.
//!
//! * `ZenlessSetup.exe` opens the wizard (Install, or Modify / Uninstall when
//!   Zenless is already installed).
//! * `--uninstall` opens the uninstaller, `--silent` runs without a window
//!   (see [`cli::USAGE`]).
//! * `ZENLESS_INSTALLER_SANDBOX=<dir>` redirects every side effect into
//!   `<dir>` and `HKCU\Software\ZenlessSandbox` (testing).
//! * `ZENLESS_INSTALLER_PAGE=<page>` opens a wizard page with demo data and
//!   `ZENLESS_SCREENSHOT=<png>` saves a screenshot of it (visual checks).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod components;
mod install;
mod payload;
mod payload_format;
mod platform;
mod report;
mod running;
// Shared verbatim by all Zenless apps; the installer does not use every helper.
#[allow(dead_code)]
mod shared;
mod ui;
mod uninstall;

use eframe::egui;

fn main() {
    let parsed = match cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            platform::attach_parent_console();
            eprintln!("ZenlessSetup: {e}\n\n{}", cli::USAGE);
            std::process::exit(cli::EXIT_USAGE);
        }
    };
    if parsed.help {
        platform::attach_parent_console();
        println!("{}", cli::USAGE);
        return;
    }
    let env = platform::Env::from_env();
    if parsed.silent {
        platform::attach_parent_console();
        std::process::exit(cli::run_silent(&env, &parsed));
    }

    let demo_page = std::env::var("ZENLESS_INSTALLER_PAGE").ok().filter(|s| !s.trim().is_empty());
    if demo_page.is_none()
        && let Some(installed) = install::Installed::detect(&env)
    {
        // Leftovers of an update that replaced a running exe.
        install::clean_old_files(&installed.root);
    }

    let launch = ui::Launch { uninstall: parsed.uninstall, demo_page };
    let title = if launch.uninstall { "Uninstall Zenless" } else { "Zenless Setup" };
    let icon = egui::IconData {
        rgba: include_bytes!("../assets/icon-128.rgba").to_vec(),
        width: 128,
        height: 128,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(ui::WINDOW_SIZE)
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(icon)
            .with_app_id("zenless-setup"),
        centered: true,
        ..Default::default()
    };
    let result = eframe::run_native(
        "zenless-setup",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, env, launch)))),
    );
    if let Err(e) = result {
        platform::attach_parent_console();
        eprintln!("ZenlessSetup: could not open the window: {e}");
        std::process::exit(cli::EXIT_FAILED);
    }
}
