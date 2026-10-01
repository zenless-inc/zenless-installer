//! The uninstall engine. Unlike installation it is best effort: every step is
//! attempted and problems are reported as warnings.

use crate::components::{self, Component, Layout};
use crate::install::{self, Installed, START_MENU_UNINSTALL, points_to, shortcut_name};
use crate::platform::{self, Env, keys};
use crate::report::Reporter;
use crate::running;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct UninstallPlan {
    pub root: PathBuf,
    pub components: Vec<Component>,
    /// Also delete `%APPDATA%\Zenless\…` (settings, download history, themes).
    pub remove_settings: bool,
}

#[derive(Clone, Debug)]
pub struct UninstallOutcome {
    /// Everything was removed (not just some components).
    pub full: bool,
    /// Components still installed afterwards.
    pub remaining: Vec<Component>,
    pub warnings: usize,
    /// Command to run hidden after we exit, deleting what we could not delete
    /// while running (our own exe and the install folder).
    pub self_delete: Option<String>,
}

/// The `cmd /c` command that removes the install folder after we exited.
///
/// With `whole_tree` the folder is deleted recursively (only used when it
/// holds nothing but our own leftovers); otherwise only `uninstall.exe` is
/// deleted and the folder is removed if it is empty.
pub fn self_delete_command(root: &Path, whole_tree: bool) -> String {
    let root_s = root.display().to_string();
    let root_s = root_s.trim_end_matches('\\');
    if whole_tree {
        format!("ping 127.0.0.1 -n 3 >nul & rmdir /s /q \"{root_s}\"")
    } else {
        format!(
            "ping 127.0.0.1 -n 3 >nul & del /f /q \"{root_s}\\uninstall.exe\" & rmdir \"{root_s}\""
        )
    }
}

/// `true` when only files we own are left in `root` (so a recursive delete is safe).
pub fn only_our_leftovers(root: &Path) -> bool {
    fn check(dir: &Path, top: bool) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else { return true };
        entries.flatten().all(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if path.is_dir() {
                let known = ["download manager", "torrent", "browser extensions", "chrome"];
                known.contains(&name.as_str()) && check(&path, false)
            } else {
                name.ends_with(".old") || (top && name == "uninstall.exe")
            }
        })
    }
    check(root, true)
}

/// Deletes a file or folder; if that fails (file in use) it is renamed to
/// `*.old` so the folder can still be cleaned up later.
fn remove_robust(path: &Path, r: &dyn Reporter) -> usize {
    if !path.exists() {
        return 0;
    }
    let res = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    match res {
        Ok(()) => 0,
        Err(e) => {
            let old = install::free_old_name(path);
            if std::fs::rename(path, &old).is_ok() {
                r.info(&format!("{} is in use; it will be deleted later.", path.display()));
                0
            } else {
                r.warn(&format!("Could not remove {}: {e}", path.display()));
                1
            }
        }
    }
}

/// Removes a component's files and shortcuts. Returns the number of warnings.
pub fn remove_component(env: &Env, layout: &Layout, c: Component, r: &dyn Reporter) -> usize {
    let mut warnings = 0;
    match layout.component_dir(c) {
        Some(dir) => warnings += remove_robust(&dir, r),
        None => warnings += remove_robust(&layout.component_path(c), r),
    }
    if let Some(name) = shortcut_name(c) {
        for dir in [env.start_menu_dir(), env.desktop_dir()] {
            warnings += remove_robust(&dir.join(name), r);
        }
    }
    match c {
        Component::Dm => remove_run_value(env, keys::RUN_DM, &layout.dm_exe()),
        Component::Torrent => {
            remove_run_value(env, keys::RUN_TORRENT, &layout.torrent_exe());
            if remove_associations(env, &layout.torrent_exe()) {
                r.info("Magnet / .torrent associations removed.");
            }
        }
        Component::Chrome => {
            if let Some(id) = install::CHROME_WEB_STORE_ID {
                let _ = env.reg().delete_tree(&keys::chrome_external(id));
            }
        }
        Component::Firefox => {}
    }
    // An empty "Browser Extensions" folder is removed too.
    let ext = layout.extensions_dir();
    if std::fs::read_dir(&ext).is_ok_and(|mut d| d.next().is_none()) {
        let _ = std::fs::remove_dir(&ext);
    }
    r.ok(&format!("Removed {}.", c.name()));
    warnings
}

/// Deletes an autostart value if it launches `exe`.
pub fn remove_run_value(env: &Env, name: &str, exe: &Path) {
    let reg = env.reg();
    if reg.get_string(keys::RUN, name).is_some_and(|v| points_to(&v, exe)) {
        let _ = reg.delete_value(keys::RUN, name);
        reg.prune_sandbox(keys::RUN);
    }
}

/// Removes the magnet / .torrent registrations that point to `exe`.
/// Returns `true` if anything was removed.
pub fn remove_associations(env: &Env, exe: &Path) -> bool {
    let reg = env.reg();
    let mut changed = false;
    let magnet_cmd = format!(r"{}\shell\open\command", keys::MAGNET);
    if reg.get_string(&magnet_cmd, "").is_some_and(|v| points_to(&v, exe)) {
        let _ = reg.delete_tree(keys::MAGNET);
        changed = true;
    }
    let progid_cmd = format!(r"{}\shell\open\command", keys::TORRENT_PROGID);
    if reg.get_string(&progid_cmd, "").is_some_and(|v| points_to(&v, exe)) {
        let _ = reg.delete_tree(keys::TORRENT_PROGID);
        changed = true;
    }
    if reg.get_string(keys::TORRENT_EXT, "").is_some_and(|v| v == keys::PROGID) {
        let _ = reg.delete_value(keys::TORRENT_EXT, "");
        if reg.key_is_empty(keys::TORRENT_EXT) {
            let _ = reg.delete_tree(keys::TORRENT_EXT);
        }
        changed = true;
    }
    if changed {
        reg.prune_sandbox(r"Software\Classes");
        if !env.is_sandbox() {
            platform::notify_assoc_changed();
        }
    }
    changed
}

/// Runs an uninstallation.
pub fn run(env: &Env, plan: &UninstallPlan, r: &dyn Reporter) -> Result<UninstallOutcome, String> {
    if plan.components.is_empty() {
        return Err("Nothing selected to uninstall.".into());
    }
    let layout = Layout::new(&plan.root);
    let installed = Installed::detect(env)
        .filter(|i| install::paths_equal(&i.root, &plan.root))
        .map(|i| i.components)
        .unwrap_or_else(|| Component::ALL.into_iter().filter(|&c| layout.component_path(c).exists()).collect());
    let remaining: Vec<Component> = installed.iter().copied().filter(|c| !plan.components.contains(c)).collect();
    let full = remaining.is_empty();
    r.info(&format!(
        "Uninstalling {} from {}{}",
        plan.components.iter().map(|c| c.short_name()).collect::<Vec<_>>().join(", "),
        plan.root.display(),
        if env.is_sandbox() { " (sandbox mode)" } else { "" }
    ));
    let mut warnings = 0;
    let steps = plan.components.len() as f32 + 3.0;
    let mut done = 0.0;
    let mut tick = |r: &dyn Reporter| {
        done += 1.0;
        r.progress(done / steps);
    };

    // Quit running apps.
    for c in [Component::Dm, Component::Torrent] {
        if plan.components.contains(&c) {
            r.status(&format!("Closing {}…", c.name()));
            running::quit_and_wait(env, c, &layout.app_exe(c).expect("app"), r);
        }
    }

    // Files, shortcuts and per-component registry entries.
    for &c in &plan.components {
        r.status(&format!("Removing {}…", c.name()));
        warnings += remove_component(env, &layout, c, r);
        tick(r);
    }

    // Settings.
    if plan.remove_settings {
        r.status("Removing settings…");
        let cfg = env.config_dir();
        let targets: Vec<PathBuf> = if full {
            vec![cfg.clone()]
        } else {
            plan.components
                .iter()
                .filter_map(|c| match c {
                    Component::Dm => Some(cfg.join("DownloadManager")),
                    Component::Torrent => Some(cfg.join("Torrent")),
                    _ => None,
                })
                .collect()
        };
        for t in targets {
            if t.exists() {
                warnings += remove_robust(&t, r);
                r.ok(&format!("Removed {}", t.display()));
            }
        }
    }
    tick(r);

    // Registry & shortcuts that belong to the suite as a whole.
    r.status("Updating Windows…");
    warnings += crate::legacy::cleanup(env, r);
    let reg = env.reg();
    let mut self_delete = None;
    if full {
        let start_menu = env.start_menu_dir();
        let _ = std::fs::remove_file(start_menu.join(START_MENU_UNINSTALL));
        if std::fs::read_dir(&start_menu).is_ok_and(|mut d| d.next().is_none()) {
            let _ = std::fs::remove_dir(&start_menu);
        }
        let _ = reg.delete_tree(keys::UNINSTALL);
        reg.prune_sandbox(keys::UNINSTALL);
        let _ = reg.delete_tree(keys::STATE);
        reg.prune_empty(keys::ZENLESS, "Software");
        r.ok("Removed from Apps & features.");
        tick(r);

        // The folder itself.
        let running_inside = std::env::current_exe()
            .ok()
            .is_some_and(|me| install::paths_equal(&me, &layout.uninstaller()));
        let whole = only_our_leftovers(&plan.root);
        if running_inside {
            self_delete = Some(self_delete_command(&plan.root, whole));
            r.info("The install folder will be deleted when Setup closes.");
        } else {
            let _ = std::fs::remove_file(layout.uninstaller());
            if whole {
                warnings += remove_robust(&plan.root, r);
            } else {
                let _ = std::fs::remove_dir(&plan.root);
                if plan.root.exists() {
                    r.info(&format!("Kept {} because it contains other files.", plan.root.display()));
                }
            }
        }
    } else {
        let _ = reg.set_string(keys::STATE, "Components", &components::join_keys(&remaining));
        let size_kb = (install::dir_size(&plan.root) / 1024).min(u32::MAX as u64) as u32;
        let _ = reg.set_dword(keys::UNINSTALL, "EstimatedSize", size_kb);
        r.ok(&format!(
            "Still installed: {}.",
            remaining.iter().map(|c| c.short_name()).collect::<Vec<_>>().join(", ")
        ));
        tick(r);
    }
    r.progress(1.0);
    r.ok(if full { "Zenless Suite was uninstalled." } else { "The selected components were removed." });
    Ok(UninstallOutcome { full, remaining, warnings, self_delete })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_delete_commands() {
        let root = Path::new(r"C:\Users\me\AppData\Local\Programs\Zenless");
        assert_eq!(
            self_delete_command(root, true),
            r#"ping 127.0.0.1 -n 3 >nul & rmdir /s /q "C:\Users\me\AppData\Local\Programs\Zenless""#
        );
        assert_eq!(
            self_delete_command(Path::new(r"D:\Apps\Zenless\"), false),
            r#"ping 127.0.0.1 -n 3 >nul & del /f /q "D:\Apps\Zenless\uninstall.exe" & rmdir "D:\Apps\Zenless""#
        );
    }

    #[test]
    fn leftovers_detection() {
        let d = std::env::temp_dir().join(format!("zenless-leftovers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Download Manager")).unwrap();
        std::fs::write(d.join("uninstall.exe"), b"x").unwrap();
        std::fs::write(d.join("Download Manager").join("zenless-dm.exe.old"), b"x").unwrap();
        assert!(only_our_leftovers(&d));
        std::fs::write(d.join("my-notes.txt"), b"precious").unwrap();
        assert!(!only_our_leftovers(&d));
        let _ = std::fs::remove_dir_all(&d);
    }
}
