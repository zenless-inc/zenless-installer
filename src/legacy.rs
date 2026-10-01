//! Clean-up of what an earlier, never released Zenless build registered on
//! some test machines: a "Zenless Update" scheduled task (it ran
//! `uninstall.exe --update --background` at logon) and a few state files in
//! `%APPDATA%\Zenless`. No released version creates any of this; the apps now
//! update themselves.
//!
//! Runs after every install, modify and uninstall, and for the legacy
//! `--update [--background]` command line (see [`crate::cli`]). Only these
//! exact names are ever removed. In sandbox mode the files are looked up in
//! the sandbox's config folder and the real scheduled task is left alone.

use crate::platform::{self, Env};
use crate::report::Reporter;
use std::path::PathBuf;

/// Name of the scheduled task the unreleased build created.
pub const TASK_NAME: &str = "Zenless Update";

/// The command that removes the task (also shown in logs).
pub fn task_command() -> String {
    format!("schtasks /Delete /TN \"{TASK_NAME}\" /F")
}

/// The leftover files, below the (sandbox-mapped) Zenless config folder.
pub fn files(env: &Env) -> Vec<PathBuf> {
    let cfg = env.config_dir();
    vec![
        cfg.join("update-state.json"),
        cfg.join("update.lock"),
        cfg.join("update.log"),
        cfg.join("updates.json"),
        cfg.join("DownloadManager").join("last-version"),
        cfg.join("Torrent").join("last-version"),
    ]
}

/// Removes the legacy task and files (best effort). Returns the number of
/// warnings; nothing is logged when there was nothing to remove.
pub fn cleanup(env: &Env, r: &dyn Reporter) -> usize {
    let mut warnings = 0;
    if env.is_sandbox() {
        r.info(&format!(
            "Sandbox mode: would remove the old \"{TASK_NAME}\" scheduled task if it exists ({}).",
            task_command()
        ));
    } else {
        match platform::delete_scheduled_task(TASK_NAME) {
            Ok(true) => r.ok(&format!("Removed the old \"{TASK_NAME}\" scheduled task.")),
            Ok(false) => {}
            Err(e) => {
                r.warn(&format!("Could not remove the old \"{TASK_NAME}\" scheduled task: {e}"));
                warnings += 1;
            }
        }
    }
    for file in files(env) {
        // Exact names only, and only plain files.
        if !file.is_file() {
            continue;
        }
        match std::fs::remove_file(&file) {
            Ok(()) => r.ok(&format!("Removed leftover {}", file.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                r.warn(&format!("Could not remove leftover {}: {e}", file.display()));
                warnings += 1;
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Level;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Collect(Mutex<Vec<(Level, String)>>);
    impl Reporter for Collect {
        fn log(&self, level: Level, text: &str) {
            self.0.lock().unwrap().push((level, text.to_owned()));
        }
        fn status(&self, _: &str) {}
        fn progress(&self, _: f32) {}
        fn cancelled(&self) -> bool {
            false
        }
    }

    #[test]
    fn task_command_line() {
        assert_eq!(task_command(), r#"schtasks /Delete /TN "Zenless Update" /F"#);
    }

    #[test]
    fn sandbox_cleanup_removes_only_the_exact_files() {
        let sb = std::env::temp_dir().join(format!("zenless-legacy-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sb);
        let env = Env::with_sandbox(Some(sb.clone()));
        let cfg = env.config_dir();
        assert!(cfg.starts_with(&sb), "legacy paths must be sandbox-mapped");
        for f in files(&env) {
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(&f, b"legacy").unwrap();
        }
        // Look-alikes and real app data stay.
        let keep = [
            cfg.join("appearance.json"),
            cfg.join("update.log.bak"),
            cfg.join("DownloadManager").join("settings.json"),
            cfg.join("DownloadManager").join("updater.json"),
            cfg.join("Torrent").join("last-version.txt"),
        ];
        for f in &keep {
            std::fs::write(f, b"keep").unwrap();
        }
        // A folder with a legacy name is not a file: left alone.
        std::fs::create_dir_all(cfg.join("Torrent").join("updates.json")).unwrap();

        let r = Collect::default();
        assert_eq!(cleanup(&env, &r), 0);
        for f in files(&env) {
            assert!(!f.exists(), "{} should be gone", f.display());
        }
        for f in &keep {
            assert!(f.is_file(), "{} should be kept", f.display());
        }
        let log = r.0.lock().unwrap();
        assert!(log.iter().any(|(l, t)| *l == Level::Info && t.contains("Sandbox mode") && t.contains(TASK_NAME)));
        assert_eq!(log.iter().filter(|(l, t)| *l == Level::Ok && t.starts_with("Removed leftover")).count(), 6);
        drop(log);

        // Running it again finds nothing to do.
        let again = Collect::default();
        assert_eq!(cleanup(&env, &again), 0);
        assert!(again.0.lock().unwrap().iter().all(|(l, _)| *l == Level::Info));
        let _ = std::fs::remove_dir_all(&sb);
    }

    /// A task that doesn't exist is "nothing to do", not an error. Uses a
    /// name no one has, so nothing on the machine is touched.
    #[cfg(windows)]
    #[test]
    fn missing_task_is_not_an_error() {
        let name = format!("Zenless Installer Test {} (does not exist)", std::process::id());
        assert!(!platform::delete_scheduled_task(&name).unwrap());
    }
}
