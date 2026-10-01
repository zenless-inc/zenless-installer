//! The install engine: runs the steps of an [`InstallPlan`] and rolls every
//! change back (files, folders, shortcuts, registry values) when a step fails
//! or the user cancels.

use crate::components::{self, Component, Layout, VERSION, WEBSITE};
use crate::payload;
use crate::platform::{self, Env, RawValue, keys, quoted};
use crate::report::{CANCELLED, Reporter, Stage};
use crate::running;
use crate::shared::kit::human_bytes;
use crate::shared::theme::Appearance;
use crate::uninstall;
use std::io::Cursor;
use std::path::{Component as PathPart, Path, PathBuf};

/// Chrome Web Store id of the Zenless extension. When set, installing the
/// Chrome component registers it as an external extension (Chrome then offers
/// to enable it) instead of relying on "Load unpacked".
pub const CHROME_WEB_STORE_ID: Option<&str> = None;
/// Chrome's update URL for Web Store extensions.
pub const CHROME_UPDATE_URL: &str = "https://clients2.google.com/service/update2/crx";
/// addons.mozilla.org listing. When set, the Finish page opens it instead of the local .xpi.
pub const FIREFOX_AMO_URL: Option<&str> = None;

pub const START_MENU_UNINSTALL: &str = "Uninstall Zenless.lnk";

/// Start menu / desktop shortcut file name of an app.
pub fn shortcut_name(c: Component) -> Option<&'static str> {
    match c {
        Component::Dm => Some("Zenless Download Manager.lnk"),
        Component::Torrent => Some("Zenless Torrent.lnk"),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub start_menu: bool,
    pub desktop: bool,
    pub autostart_dm: bool,
    pub autostart_torrent: bool,
    /// Make Zenless Torrent the handler for magnet links and .torrent files.
    pub associate: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { start_menu: true, desktop: true, autostart_dm: true, autostart_torrent: false, associate: true }
    }
}

#[derive(Clone, Debug)]
pub struct InstallPlan {
    pub root: PathBuf,
    /// Components to install or update.
    pub components: Vec<Component>,
    /// Previously installed components the user deselected (Modify).
    pub remove: Vec<Component>,
    pub options: Options,
    /// Theme to write into the shared `appearance.json`.
    pub theme: Option<String>,
}

#[derive(Clone, Debug)]
pub struct InstallOutcome {
    pub root: PathBuf,
    /// Everything installed after this run.
    pub components: Vec<Component>,
    pub warnings: usize,
}

// ---------------------------------------------------------------------------
// Command lines written to the registry
// ---------------------------------------------------------------------------

pub fn uninstall_string(layout: &Layout) -> String {
    format!("{} --uninstall", quoted(&layout.uninstaller()))
}

pub fn quiet_uninstall_string(layout: &Layout) -> String {
    format!("{} --uninstall --silent", quoted(&layout.uninstaller()))
}

pub fn modify_path(layout: &Layout) -> String {
    quoted(&layout.uninstaller())
}

pub fn run_value(exe: &Path) -> String {
    format!("{} --minimized", quoted(exe))
}

pub fn open_command(exe: &Path) -> String {
    format!("{} \"%1\"", quoted(exe))
}

pub fn default_icon(exe: &Path) -> String {
    format!("{},0", quoted(exe))
}

/// `true` when a registry command line refers to `exe` (case-insensitive).
pub fn points_to(value: &str, exe: &Path) -> bool {
    let v = value.to_ascii_lowercase().replace('/', "\\");
    v.contains(&exe.display().to_string().to_ascii_lowercase())
}

// ---------------------------------------------------------------------------
// Installed state
// ---------------------------------------------------------------------------

/// What an earlier run installed (from `HKCU\Software\Zenless\Suite`).
#[derive(Clone, Debug)]
pub struct Installed {
    pub root: PathBuf,
    pub version: String,
    pub components: Vec<Component>,
    pub options: Options,
}

impl Installed {
    pub fn detect(env: &Env) -> Option<Self> {
        let reg = env.reg();
        let root = PathBuf::from(reg.get_string(keys::STATE, "InstallDir")?);
        let version = reg.get_string(keys::STATE, "Version").unwrap_or_default();
        let components = reg
            .get_string(keys::STATE, "Components")
            .map(|s| s.split(',').filter_map(Component::from_key).collect())
            .unwrap_or_default();
        Some(Self::with(env, root, version, components))
    }

    /// Fallback when the registry entry is gone but we run as `<root>\uninstall.exe`.
    pub fn from_folder(env: &Env, root: PathBuf) -> Option<Self> {
        let layout = Layout::new(&root);
        let components: Vec<Component> =
            Component::ALL.into_iter().filter(|&c| layout.component_path(c).exists()).collect();
        (!components.is_empty()).then(|| Self::with(env, root, String::new(), components))
    }

    fn with(env: &Env, root: PathBuf, version: String, components: Vec<Component>) -> Self {
        let reg = env.reg();
        let layout = Layout::new(&root);
        let flag = |name: &str, default: bool| reg.get_dword(keys::STATE, name).map(|v| v != 0).unwrap_or(default);
        let ours = |name: &str, exe: &Path| reg.get_string(keys::RUN, name).is_some_and(|v| points_to(&v, exe));
        let options = Options {
            start_menu: flag("StartMenuShortcuts", true),
            desktop: flag("DesktopShortcuts", false),
            autostart_dm: ours(keys::RUN_DM, &layout.dm_exe()),
            autostart_torrent: ours(keys::RUN_TORRENT, &layout.torrent_exe()),
            associate: reg
                .get_string(&format!(r"{}\shell\open\command", keys::MAGNET), "")
                .is_some_and(|v| points_to(&v, &layout.torrent_exe())),
        };
        Self { root, version, components, options }
    }
}

// ---------------------------------------------------------------------------
// Rollback journal
// ---------------------------------------------------------------------------

enum Undo {
    /// A folder we created (with everything inside).
    CreatedDir(PathBuf),
    CreatedFile(PathBuf),
    /// An existing file/folder renamed out of the way; restored on rollback,
    /// deleted on commit.
    MovedAside { original: PathBuf, backup: PathBuf },
    /// A registry value we set; `previous` is restored on rollback.
    RegValue { path: String, name: String, previous: Option<RawValue> },
    /// A registry key we created (deleted with its subtree on rollback).
    RegKey { path: String },
}

pub struct Journal {
    env: Env,
    undo: Vec<Undo>,
}

/// Extra attempts (150 ms apart) to rename a file that is briefly in use.
const MOVE_RETRIES: u32 = 6;

impl Journal {
    pub fn new(env: Env) -> Self {
        Self { env, undo: Vec::new() }
    }

    /// Creates `dir` (and parents), remembering the topmost folder we created.
    pub fn ensure_dir(&mut self, dir: &Path) -> Result<(), String> {
        if dir.is_dir() {
            return Ok(());
        }
        let mut topmost = dir.to_path_buf();
        while let Some(parent) = topmost.parent() {
            if parent.as_os_str().is_empty() || parent.exists() {
                break;
            }
            topmost = parent.to_path_buf();
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        self.undo.push(Undo::CreatedDir(topmost));
        Ok(())
    }

    /// Renames an existing file/folder to a free `*.old` name. Retries for a
    /// moment first: virus scanners and indexers often hold fresh files briefly.
    fn move_aside(&mut self, path: &Path) -> Result<(), String> {
        let backup = free_old_name(path);
        let mut attempt = 0;
        loop {
            match std::fs::rename(path, &backup) {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) if attempt < MOVE_RETRIES => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
                Err(e) => {
                    return Err(format!("could not replace {} (is it open in another program?): {e}", path.display()));
                }
            }
        }
        self.undo.push(Undo::MovedAside { original: path.to_path_buf(), backup });
        Ok(())
    }

    pub fn write_file(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            self.ensure_dir(parent)?;
        }
        if path.exists() {
            self.move_aside(path)?;
        }
        std::fs::write(path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
        self.undo.push(Undo::CreatedFile(path.to_path_buf()));
        Ok(())
    }

    pub fn copy_file(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        if let Some(parent) = to.parent() {
            self.ensure_dir(parent)?;
        }
        if to.exists() {
            self.move_aside(to)?;
        }
        std::fs::copy(from, to).map_err(|e| format!("could not copy to {}: {e}", to.display()))?;
        self.undo.push(Undo::CreatedFile(to.to_path_buf()));
        Ok(())
    }

    /// Makes `dir` a fresh, empty folder (an existing one is moved aside).
    pub fn fresh_dir(&mut self, dir: &Path) -> Result<(), String> {
        if let Some(parent) = dir.parent() {
            self.ensure_dir(parent)?;
        }
        if dir.exists() {
            self.move_aside(dir)?;
        }
        std::fs::create_dir(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        self.undo.push(Undo::CreatedDir(dir.to_path_buf()));
        Ok(())
    }

    /// Removes a file so that a rollback can bring it back.
    pub fn remove_file(&mut self, path: &Path) -> Result<(), String> {
        if path.exists() { self.move_aside(path) } else { Ok(()) }
    }

    fn reg_prepare(&mut self, path: &str, name: &str) {
        let reg = self.env.reg();
        if !reg.key_exists(path) {
            // remember the topmost key we are about to create
            let mut topmost = path.to_owned();
            while let Some(i) = topmost.rfind('\\') {
                let parent = &topmost[..i];
                if reg.key_exists(parent) {
                    break;
                }
                topmost.truncate(i);
            }
            self.undo.push(Undo::RegKey { path: topmost });
        }
        let previous = reg.get_raw(path, name);
        self.undo.push(Undo::RegValue { path: path.to_owned(), name: name.to_owned(), previous });
    }

    pub fn reg_string(&mut self, path: &str, name: &str, value: &str) -> Result<(), String> {
        self.reg_prepare(path, name);
        self.env
            .reg()
            .set_string(path, name, value)
            .map_err(|e| format!("could not write registry value {path}\\{name}: {e}"))
    }

    pub fn reg_dword(&mut self, path: &str, name: &str, value: u32) -> Result<(), String> {
        self.reg_prepare(path, name);
        self.env
            .reg()
            .set_dword(path, name, value)
            .map_err(|e| format!("could not write registry value {path}\\{name}: {e}"))
    }

    /// Keeps the changes: deletes the moved-aside backups (locked ones stay
    /// as `*.old` and are cleaned up by the next run).
    pub fn commit(self, r: &dyn Reporter) {
        for u in self.undo {
            if let Undo::MovedAside { backup, .. } = u
                && remove_any(&backup).is_err()
            {
                r.info(&format!("{} is in use; it will be removed next time.", backup.display()));
            }
        }
    }

    /// Undoes every recorded change, newest first.
    pub fn rollback(self, r: &dyn Reporter) {
        let reg = self.env.reg();
        for u in self.undo.into_iter().rev() {
            let res = match &u {
                Undo::CreatedFile(p) => remove_missing_ok(p),
                Undo::CreatedDir(p) => remove_missing_ok(p),
                Undo::MovedAside { original, backup } => {
                    let _ = remove_missing_ok(original);
                    std::fs::rename(backup, original)
                }
                Undo::RegValue { path, name, previous } => match previous {
                    Some(v) => reg.set_raw(path, name, v),
                    None => reg.delete_value(path, name),
                },
                Undo::RegKey { path } => {
                    let r = reg.delete_tree(path);
                    reg.prune_sandbox(path);
                    r
                }
            };
            if let Err(e) = res {
                let what = match &u {
                    Undo::CreatedFile(p) | Undo::CreatedDir(p) => p.display().to_string(),
                    Undo::MovedAside { original, .. } => original.display().to_string(),
                    Undo::RegValue { path, name, .. } => format!("{path}\\{name}"),
                    Undo::RegKey { path } => path.clone(),
                };
                r.warn(&format!("Could not undo {what}: {e}"));
            }
        }
        r.info("All changes were rolled back.");
    }
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
}

fn remove_missing_ok(path: &Path) -> std::io::Result<()> {
    match remove_any(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// `zenless-dm.exe` → `zenless-dm.exe.old` (or `.1.old`, `.2.old`… if taken).
pub fn free_old_name(path: &Path) -> PathBuf {
    free_name(path, "old")
}

/// `<path>.<suffix>` next to `path`, numbered (`.1.<suffix>`…) if taken.
fn free_name(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut candidate = path.with_file_name(format!("{name}.{suffix}"));
    let mut i = 1;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{name}.{i}.{suffix}"));
        i += 1;
    }
    candidate
}

/// Files below `dir`, as paths relative to it (sorted).
fn files_below(dir: &Path) -> Result<Vec<PathBuf>, String> {
    fn walk(base: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(base.join(rel))? {
            let entry = entry?;
            let child = rel.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                walk(base, &child, out)?;
            } else {
                out.push(child);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, Path::new(""), &mut out).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    out.sort();
    Ok(out)
}

/// Deletes `*.old` leftovers from a previous run (best effort).
pub fn clean_old_files(root: &Path) -> usize {
    fn walk(dir: &Path, depth: usize, removed: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_old = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("old"));
            if is_old {
                if remove_any(&path).is_ok() {
                    *removed += 1;
                }
            } else if path.is_dir() && depth < 3 {
                walk(&path, depth + 1, removed);
            }
        }
    }
    let mut removed = 0;
    walk(root, 0, &mut removed);
    removed
}

/// Total size of the files below `dir` (ignoring `*.old` leftovers).
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(_) if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("old")) => 0,
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

// ---------------------------------------------------------------------------
// Chrome extension zip
// ---------------------------------------------------------------------------

/// The folder inside the zip that holds `manifest.json` ("" for the zip root).
pub fn manifest_prefix(names: &[String]) -> Option<String> {
    if names.iter().any(|n| n == "manifest.json") {
        return Some(String::new());
    }
    names
        .iter()
        .filter_map(|n| n.strip_suffix("manifest.json"))
        .filter(|p| p.ends_with('/') && p.matches('/').count() == 1)
        .map(str::to_owned)
        .next()
}

/// Extracts an extension zip into `dest` (which must exist and be empty).
pub fn extract_extension(data: &[u8], dest: &Path, r: &dyn Reporter) -> Result<usize, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(data)).map_err(|e| format!("extension archive is damaged: {e}"))?;
    let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
    let prefix = manifest_prefix(&names).ok_or("extension archive has no manifest.json")?;
    let mut count = 0;
    for i in 0..zip.len() {
        r.check_cancel()?;
        let mut entry = zip.by_index(i).map_err(|e| format!("extension archive is damaged: {e}"))?;
        let Some(rel) = entry.enclosed_name() else {
            r.warn(&format!("Skipped unsafe path in archive: {}", entry.name()));
            continue;
        };
        let Ok(rel) = rel.strip_prefix(&prefix) else { continue };
        if rel.as_os_str().is_empty() || rel.components().any(|c| !matches!(c, PathPart::Normal(_))) {
            continue;
        }
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| format!("could not create {}: {e}", out.display()))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        let mut file = std::fs::File::create(&out).map_err(|e| format!("could not write {}: {e}", out.display()))?;
        std::io::copy(&mut entry, &mut file).map_err(|e| format!("could not extract {}: {e}", out.display()))?;
        count += 1;
    }
    Ok(count)
}

/// Installs the unpacked Chrome extension into `dir`, replacing whatever is
/// there (any version, including the same one).
///
/// Normally the old folder is moved aside and a fresh one is extracted. When
/// the folder itself can't be renamed (a browser, Explorer or a console has it
/// open) its files are replaced one by one instead, and files that are no
/// longer part of the extension are removed. Either way every change is in the
/// journal, so a rollback restores the previous folder.
pub fn install_chrome_extension(j: &mut Journal, dir: &Path, data: &[u8], r: &dyn Reporter) -> Result<usize, String> {
    let busy = match j.fresh_dir(dir) {
        Ok(()) => return extract_extension(data, dir, r),
        Err(e) if dir.is_dir() => e,
        Err(e) => return Err(e),
    };
    r.info(&format!("{busy}; replacing the extension files in place instead."));
    let staging = free_name(dir, "new");
    j.fresh_dir(&staging)?;
    let count = extract_extension(data, &staging, r)?;
    let fresh = files_below(&staging)?;
    for rel in &fresh {
        r.check_cancel()?;
        j.copy_file(&staging.join(rel), &dir.join(rel))?;
    }
    for rel in files_below(dir)? {
        let backup = rel.extension().is_some_and(|e| e.eq_ignore_ascii_case("old"));
        if !backup && !fresh.contains(&rel) {
            j.remove_file(&dir.join(&rel))?;
        }
    }
    // Rollback removes it as well; it's only scaffolding.
    let _ = std::fs::remove_dir_all(&staging);
    Ok(count)
}

// ---------------------------------------------------------------------------
// Firefox add-on
// ---------------------------------------------------------------------------

/// `true` when an .xpi carries a Mozilla signature (`META-INF/mozilla.rsa` for
/// PKCS#7 or `META-INF/cose.sig` for COSE signing). Firefox only installs
/// signed add-ons permanently; unsigned ones can only be loaded temporarily
/// from `about:debugging`.
pub fn xpi_names_signed<S: AsRef<str>>(names: &[S]) -> bool {
    names.iter().any(|n| {
        let n = n.as_ref().replace('\\', "/");
        n.eq_ignore_ascii_case("META-INF/mozilla.rsa") || n.eq_ignore_ascii_case("META-INF/cose.sig")
    })
}

/// Reads an .xpi and reports whether it is signed (`None`: not a readable zip).
pub fn xpi_is_signed(path: &Path) -> Option<bool> {
    let file = std::fs::File::open(path).ok()?;
    let zip = zip::ZipArchive::new(std::io::BufReader::new(file)).ok()?;
    let names: Vec<&str> = zip.file_names().collect();
    Some(xpi_names_signed(&names))
}

// ---------------------------------------------------------------------------
// The install run
// ---------------------------------------------------------------------------

/// Checks an install folder chosen by the user. Returns a human message on error.
pub fn validate_root(root: &Path) -> Result<(), String> {
    if root.as_os_str().is_empty() {
        return Err("Choose an install folder.".into());
    }
    if !root.is_absolute() {
        return Err("Use a full path, like C:\\Users\\you\\AppData\\Local\\Programs\\Zenless.".into());
    }
    if root.parent().is_none() || root.components().count() < 2 {
        return Err("Pick a folder, not the root of a drive.".into());
    }
    let lower = root.display().to_string().to_ascii_lowercase();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "SystemRoot"] {
        if let Some(v) = std::env::var_os(var) {
            let v = v.to_string_lossy().to_ascii_lowercase();
            if !v.is_empty() && (lower == v || lower.starts_with(&format!("{v}\\"))) {
                return Err("That folder needs administrator rights. Zenless installs for your account only — pick a folder you own.".into());
            }
        }
    }
    Ok(())
}

/// Runs an installation. On error or cancellation everything is rolled back
/// and the error message (or [`CANCELLED`]) is returned.
pub fn run(env: &Env, plan: &InstallPlan, r: &dyn Reporter) -> Result<InstallOutcome, String> {
    r.info(&format!("Zenless Setup {VERSION}{}", if env.is_sandbox() { " (sandbox mode)" } else { "" }));
    let mut journal = Journal::new(env.clone());
    match execute(env, plan, r, &mut journal) {
        Ok(outcome) => {
            r.status("Finishing…");
            journal.commit(r);
            let mut warnings = finish_removals(env, plan, &outcome, r);
            warnings += crate::legacy::cleanup(env, r);
            r.progress(1.0);
            r.ok("Installation complete.");
            Ok(InstallOutcome { warnings, ..outcome })
        }
        Err(e) => {
            if e == CANCELLED {
                r.warn("Installation cancelled — rolling back…");
            } else {
                r.error(&e);
                r.warn("Rolling back…");
            }
            r.status("Rolling back…");
            journal.rollback(r);
            Err(e)
        }
    }
}

fn execute(env: &Env, plan: &InstallPlan, r: &dyn Reporter, j: &mut Journal) -> Result<InstallOutcome, String> {
    validate_root(&plan.root)?;
    if plan.components.is_empty() && plan.remove.is_empty() {
        return Err("Nothing to install.".into());
    }
    let layout = Layout::new(&plan.root);
    let previous = Installed::detect(env).filter(|i| paths_equal(&i.root, &plan.root));
    let mut final_set: Vec<Component> = previous.as_ref().map(|p| p.components.clone()).unwrap_or_default();
    final_set.extend(plan.components.iter().copied());
    final_set.retain(|c| !plan.remove.contains(c));
    final_set.sort();
    final_set.dedup();

    // 1. Prepare -----------------------------------------------------------
    r.status("Preparing…");
    r.info(&format!("Install folder: {}", plan.root.display()));
    j.ensure_dir(&plan.root)?;
    let cleaned = clean_old_files(&plan.root);
    if cleaned > 0 {
        r.info(&format!("Removed {cleaned} leftover file(s) from a previous update."));
    }
    r.progress(0.03);

    // 2. Ask running apps to exit -----------------------------------------
    for c in [Component::Dm, Component::Torrent] {
        if plan.components.contains(&c) || plan.remove.contains(&c) {
            let exe = layout.app_exe(c).expect("app");
            r.status(&format!("Closing {}…", c.name()));
            running::quit_and_wait(env, c, &exe, r);
            r.check_cancel()?;
        }
    }
    r.progress(0.06);

    // 3. Get the payloads (0.06 → 0.70) -------------------------------------
    let total_weight: f32 = plan.components.iter().map(|c| weight(*c)).sum::<f32>().max(1.0);
    let mut base = 0.06;
    let mut blobs = Vec::new();
    for &c in &plan.components {
        r.check_cancel()?;
        let span = 0.64 * weight(c) / total_weight;
        r.status(&if c.embedded().is_embedded() {
            format!("Unpacking {}…", c.name())
        } else {
            format!("Downloading {}…", c.name())
        });
        let data = payload::obtain(c, Stage::new(r, base, span))?;
        base += span;
        blobs.push((c, data));
    }
    r.progress(0.70);

    // 4. Write files (0.70 → 0.85) -----------------------------------------
    // Always replaced, whatever version is installed (repairs damaged files).
    for (i, (c, data)) in blobs.iter().enumerate() {
        r.check_cancel()?;
        r.status(&format!("Installing {}…", c.name()));
        install_component_files(j, &layout, *c, data, r)?;
        r.progress(0.70 + 0.15 * (i + 1) as f32 / blobs.len().max(1) as f32);
    }
    drop(blobs);

    // The uninstaller is a copy of this program.
    r.check_cancel()?;
    let me = std::env::current_exe().map_err(|e| format!("could not locate the installer: {e}"))?;
    let uninstaller = layout.uninstaller();
    if paths_equal(&me, &uninstaller) {
        r.info("Running from the installed uninstaller; keeping it.");
    } else {
        j.copy_file(&me, &uninstaller)?;
        r.ok(&format!("Uninstaller → {}", rel(&plan.root, &uninstaller)));
    }

    if let Some(theme) = &plan.theme {
        write_appearance(env, theme, j)?;
        r.ok(&format!("Theme \"{theme}\" saved for all Zenless apps."));
    }
    r.progress(0.87);

    // 5. Shortcuts -----------------------------------------------------------
    r.check_cancel()?;
    r.status("Creating shortcuts…");
    let apps: Vec<Component> = final_set.iter().copied().filter(|c| c.is_app()).collect();
    let start_menu = env.start_menu_dir();
    let desktop = env.desktop_dir();
    for &c in &[Component::Dm, Component::Torrent] {
        let name = shortcut_name(c).expect("app");
        let exe = layout.app_exe(c).expect("app");
        let wanted = apps.contains(&c);
        for (dir, enabled) in [(&start_menu, plan.options.start_menu), (&desktop, plan.options.desktop)] {
            let lnk = dir.join(name);
            if wanted && enabled {
                make_shortcut(j, &lnk, &exe, None, c.name())?;
            } else if lnk.exists() {
                j.remove_file(&lnk)?;
            }
        }
    }
    let un_lnk = start_menu.join(START_MENU_UNINSTALL);
    if plan.options.start_menu {
        make_shortcut(j, &un_lnk, &uninstaller, Some("--uninstall"), "Uninstall Zenless Suite")?;
        r.ok(&format!("Start menu shortcuts → {}", start_menu.display()));
    } else if un_lnk.exists() {
        j.remove_file(&un_lnk)?;
    }
    if plan.options.desktop && !apps.is_empty() {
        r.ok(&format!("Desktop shortcuts → {}", desktop.display()));
    }
    r.progress(0.92);

    // 6. Registry --------------------------------------------------------------
    r.check_cancel()?;
    r.status("Registering Zenless…");
    let keys_list = components::join_keys(&final_set);
    j.reg_string(keys::STATE, "InstallDir", &plan.root.display().to_string())?;
    j.reg_string(keys::STATE, "Version", VERSION)?;
    j.reg_string(keys::STATE, "Components", &keys_list)?;
    j.reg_dword(keys::STATE, "StartMenuShortcuts", plan.options.start_menu as u32)?;
    j.reg_dword(keys::STATE, "DesktopShortcuts", plan.options.desktop as u32)?;

    let size_kb = (dir_size(&plan.root) / 1024).min(u32::MAX as u64) as u32;
    let u = keys::UNINSTALL;
    j.reg_string(u, "DisplayName", "Zenless Suite")?;
    j.reg_string(u, "DisplayVersion", VERSION)?;
    j.reg_string(u, "Publisher", "Zenless")?;
    j.reg_string(u, "InstallLocation", &plan.root.display().to_string())?;
    j.reg_string(u, "DisplayIcon", &format!("{},0", uninstaller.display()))?;
    j.reg_string(u, "UninstallString", &uninstall_string(&layout))?;
    j.reg_string(u, "QuietUninstallString", &quiet_uninstall_string(&layout))?;
    j.reg_string(u, "ModifyPath", &modify_path(&layout))?;
    j.reg_string(u, "URLInfoAbout", WEBSITE)?;
    j.reg_string(u, "HelpLink", "https://github.com/zenless-inc")?;
    j.reg_string(u, "InstallDate", &install_date())?;
    j.reg_dword(u, "EstimatedSize", size_kb)?;
    j.reg_dword(u, "NoRepair", 1)?;
    r.ok(&format!("Added to Apps & features ({}).", human_bytes(size_kb as u64 * 1024)));

    if plan.options.autostart_dm && final_set.contains(&Component::Dm) {
        j.reg_string(keys::RUN, keys::RUN_DM, &run_value(&layout.dm_exe()))?;
        r.ok("Download Manager starts with Windows.");
    }
    if plan.options.autostart_torrent && final_set.contains(&Component::Torrent) {
        j.reg_string(keys::RUN, keys::RUN_TORRENT, &run_value(&layout.torrent_exe()))?;
        r.ok("Zenless Torrent starts with Windows.");
    }
    if plan.options.associate && final_set.contains(&Component::Torrent) {
        register_associations(j, &layout.torrent_exe())?;
        r.ok("Zenless Torrent opens magnet links and .torrent files.");
    }
    if let Some(id) = CHROME_WEB_STORE_ID
        && plan.components.contains(&Component::Chrome)
    {
        j.reg_string(&keys::chrome_external(id), "update_url", CHROME_UPDATE_URL)?;
        r.ok("Chrome will offer to enable the Zenless extension on its next start.");
    }
    if !env.is_sandbox() && (plan.options.associate || previous.as_ref().is_some_and(|p| p.options.associate)) {
        platform::notify_assoc_changed();
    }
    r.progress(0.97);

    Ok(InstallOutcome { root: plan.root.clone(), components: final_set, warnings: 0 })
}

/// Writes one component's files, replacing what is installed (journaled).
pub fn install_component_files(j: &mut Journal, layout: &Layout, c: Component, data: &[u8], r: &dyn Reporter) -> Result<(), String> {
    match c {
        Component::Dm | Component::Torrent => {
            let exe = layout.app_exe(c).expect("app");
            j.write_file(&exe, data)?;
            r.ok(&format!("{} → {}", c.name(), rel(&layout.root, &exe)));
        }
        Component::Chrome => {
            let dir = layout.chrome_dir();
            let n = install_chrome_extension(j, &dir, data, r)?;
            r.ok(&format!("Chrome extension ({n} files) → {}", rel(&layout.root, &dir)));
        }
        Component::Firefox => {
            let xpi = layout.firefox_xpi();
            j.write_file(&xpi, data)?;
            r.ok(&format!("Firefox extension → {}", rel(&layout.root, &xpi)));
        }
    }
    Ok(())
}

/// `path` relative to the install folder for log lines (full path if outside).
fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).map(|p| p.display().to_string()).unwrap_or_else(|_| path.display().to_string())
}

/// Relative cost of obtaining a component (drives the progress bar).
fn weight(c: Component) -> f32 {
    let e = c.embedded();
    let mb = (c.install_size() as f32 / (1 << 20) as f32).max(0.2);
    if e.is_embedded() { 1.0 + mb * 0.1 } else { 2.0 + mb }
}

fn make_shortcut(j: &mut Journal, lnk: &Path, target: &Path, args: Option<&str>, desc: &str) -> Result<(), String> {
    if let Some(parent) = lnk.parent() {
        j.ensure_dir(parent)?;
    }
    if lnk.exists() {
        j.remove_file(lnk)?;
    }
    platform::create_shortcut(lnk, target, args, desc)
        .map_err(|e| format!("could not create shortcut {}: {e}", lnk.display()))?;
    j.undo.push(Undo::CreatedFile(lnk.to_path_buf()));
    Ok(())
}

/// magnet: and .torrent → Zenless Torrent, exactly as described in the suite spec.
fn register_associations(j: &mut Journal, exe: &Path) -> Result<(), String> {
    let m = keys::MAGNET;
    j.reg_string(m, "", "URL:Magnet Link")?;
    j.reg_string(m, "URL Protocol", "")?;
    j.reg_string(&format!(r"{m}\DefaultIcon"), "", &default_icon(exe))?;
    j.reg_string(&format!(r"{m}\shell\open\command"), "", &open_command(exe))?;
    j.reg_string(keys::TORRENT_EXT, "", keys::PROGID)?;
    let p = keys::TORRENT_PROGID;
    j.reg_string(p, "", "Torrent file")?;
    j.reg_string(&format!(r"{p}\DefaultIcon"), "", &default_icon(exe))?;
    j.reg_string(&format!(r"{p}\shell\open\command"), "", &open_command(exe))?;
    Ok(())
}

fn write_appearance(env: &Env, theme: &str, j: &mut Journal) -> Result<(), String> {
    let path = env.appearance_path();
    let mut appearance: Appearance = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    appearance.theme = theme.to_owned();
    let json = serde_json::to_vec_pretty(&appearance).map_err(|e| e.to_string())?;
    j.write_file(&path, &json)
}

/// Post-commit clean-up for Modify: components, shortcuts and registry entries
/// the user switched off. Best effort; returns the number of warnings.
fn finish_removals(env: &Env, plan: &InstallPlan, outcome: &InstallOutcome, r: &dyn Reporter) -> usize {
    let layout = Layout::new(&plan.root);
    let mut warnings = 0;
    for &c in &plan.remove {
        r.status(&format!("Removing {}…", c.name()));
        warnings += uninstall::remove_component(env, &layout, c, r);
    }
    let has = |c| outcome.components.contains(&c);
    if !(plan.options.autostart_dm && has(Component::Dm)) {
        uninstall::remove_run_value(env, keys::RUN_DM, &layout.dm_exe());
    }
    if !(plan.options.autostart_torrent && has(Component::Torrent)) {
        uninstall::remove_run_value(env, keys::RUN_TORRENT, &layout.torrent_exe());
    }
    if !(plan.options.associate && has(Component::Torrent)) && uninstall::remove_associations(env, &layout.torrent_exe()) {
        r.info("Magnet / .torrent associations removed.");
    }
    warnings
}

/// `YYYYMMDD` (UTC) for the uninstall entry.
fn install_date() -> String {
    crate::shared::kit::human_date(crate::shared::kit::unix_now())[..10].replace('-', "")
}

/// Compares two paths the way Windows does (case-insensitive, `/` == `\`).
pub fn paths_equal(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        p.display().to_string().trim_start_matches(r"\\?\").replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Level, Reporter};
    use std::io::Write;

    struct Quiet;
    impl Reporter for Quiet {
        fn log(&self, _: Level, _: &str) {}
        fn status(&self, _: &str) {}
        fn progress(&self, _: f32) {}
        fn cancelled(&self) -> bool {
            false
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("zenless-installer-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn registry_command_lines() {
        let layout = Layout::new(r"C:\Users\me\AppData\Local\Programs\Zenless");
        assert_eq!(
            uninstall_string(&layout),
            r#""C:\Users\me\AppData\Local\Programs\Zenless\uninstall.exe" --uninstall"#
        );
        assert_eq!(
            quiet_uninstall_string(&layout),
            r#""C:\Users\me\AppData\Local\Programs\Zenless\uninstall.exe" --uninstall --silent"#
        );
        assert_eq!(modify_path(&layout), r#""C:\Users\me\AppData\Local\Programs\Zenless\uninstall.exe""#);
        let exe = Path::new(r"C:\Z\Torrent\zenless-torrent.exe");
        assert_eq!(open_command(exe), r#""C:\Z\Torrent\zenless-torrent.exe" "%1""#);
        assert_eq!(default_icon(exe), r#""C:\Z\Torrent\zenless-torrent.exe",0"#);
        assert_eq!(run_value(exe), r#""C:\Z\Torrent\zenless-torrent.exe" --minimized"#);
        assert!(points_to(r#""c:\z\TORRENT\zenless-torrent.exe" "%1""#, exe));
        assert!(!points_to(r#""C:\Other\qbittorrent.exe" "%1""#, exe));
    }

    #[test]
    fn old_names() {
        let d = temp_dir("old");
        let f = d.join("a.exe");
        assert_eq!(free_old_name(&f), d.join("a.exe.old"));
        std::fs::write(d.join("a.exe.old"), b"x").unwrap();
        assert_eq!(free_old_name(&f), d.join("a.exe.1.old"));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub").join("b.exe.old"), b"x").unwrap();
        assert_eq!(clean_old_files(&d), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn journal_rollback_restores_files() {
        let d = temp_dir("journal");
        let existing = d.join("keep.txt");
        std::fs::write(&existing, b"original").unwrap();
        let mut j = Journal::new(Env::with_sandbox(Some(d.join("sb"))));
        j.write_file(&existing, b"new").unwrap();
        j.write_file(&d.join("deep").join("er").join("new.txt"), b"x").unwrap();
        j.fresh_dir(&d.join("ext")).unwrap();
        assert_eq!(std::fs::read(&existing).unwrap(), b"new");
        j.rollback(&Quiet);
        assert_eq!(std::fs::read(&existing).unwrap(), b"original");
        assert!(!d.join("deep").exists());
        assert!(!d.join("ext").exists());
        assert!(!d.join("keep.txt.old").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn journal_commit_drops_backups() {
        let d = temp_dir("commit");
        let existing = d.join("app.exe");
        std::fs::write(&existing, b"v1").unwrap();
        let mut j = Journal::new(Env::with_sandbox(Some(d.join("sb"))));
        j.write_file(&existing, b"v2").unwrap();
        assert!(d.join("app.exe.old").exists());
        j.commit(&Quiet);
        assert!(!d.join("app.exe.old").exists());
        assert_eq!(std::fs::read(&existing).unwrap(), b"v2");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Registry rollback, entirely inside `HKCU\Software\ZenlessSandbox`.
    #[cfg(windows)]
    #[test]
    fn journal_rollback_restores_registry() {
        let env = Env::with_sandbox(Some(std::env::temp_dir().join("zenless-reg-test")));
        let reg = env.reg();
        let base = format!(r"Software\ZenlessInstallerTest{}", std::process::id());
        let existing = format!(r"{base}\Existing");
        reg.set_string(&existing, "Kept", "old value").unwrap();

        let mut j = Journal::new(env.clone());
        j.reg_string(&existing, "Kept", "new value").unwrap();
        j.reg_string(&existing, "Added", "x").unwrap();
        j.reg_dword(&format!(r"{base}\Brand\New\Key"), "N", 7).unwrap();
        assert_eq!(reg.get_string(&existing, "Kept").as_deref(), Some("new value"));
        assert_eq!(reg.get_dword(&format!(r"{base}\Brand\New\Key"), "N"), Some(7));
        j.rollback(&Quiet);

        assert_eq!(reg.get_string(&existing, "Kept").as_deref(), Some("old value"));
        assert_eq!(reg.get_string(&existing, "Added"), None);
        assert!(!reg.key_exists(&format!(r"{base}\Brand")));
        // the real registry was never touched
        assert!(!Env::with_sandbox(None).reg().key_exists(&base));

        reg.delete_tree(&base).unwrap();
        reg.prune_sandbox(&base);
        assert!(!reg.key_exists(&base));
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, data) in entries {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn extension_zip_extraction() {
        let d = temp_dir("zip");
        let flat = make_zip(&[("manifest.json", b"{}"), ("js/background.js", b"//"), ("icons/icon-16.png", b"png")]);
        let n = extract_extension(&flat, &d, &Quiet).unwrap();
        assert_eq!(n, 3);
        assert!(d.join("js").join("background.js").is_file());

        let d2 = temp_dir("zip-nested");
        let nested = make_zip(&[("zenless/manifest.json", b"{}"), ("zenless/popup.html", b"<p>")]);
        assert_eq!(extract_extension(&nested, &d2, &Quiet).unwrap(), 2);
        assert!(d2.join("manifest.json").is_file());

        let bad = make_zip(&[("readme.txt", b"no manifest")]);
        assert!(extract_extension(&bad, &d2, &Quiet).is_err());
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&d2);
    }

    #[test]
    fn xpi_signatures() {
        assert!(!xpi_names_signed(&["manifest.json", "src/background.js"]));
        assert!(xpi_names_signed(&["manifest.json", "META-INF/mozilla.rsa", "META-INF/mozilla.sf"]));
        assert!(xpi_names_signed(&["META-INF/cose.sig", "META-INF/cose.manifest", "manifest.json"]));
        assert!(xpi_names_signed(&["meta-inf/MOZILLA.RSA"]));
        assert!(!xpi_names_signed(&["META-INF/manifest.mf", "src/META-INF/mozilla.rsa"]));

        let d = temp_dir("xpi");
        let unsigned = d.join("unsigned.xpi");
        std::fs::write(&unsigned, make_zip(&[("manifest.json", b"{}")])).unwrap();
        assert_eq!(xpi_is_signed(&unsigned), Some(false));
        let signed = d.join("signed.xpi");
        std::fs::write(&signed, make_zip(&[("manifest.json", b"{}"), ("META-INF/cose.sig", b"sig")])).unwrap();
        assert_eq!(xpi_is_signed(&signed), Some(true));
        assert_eq!(xpi_is_signed(&d.join("missing.xpi")), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Installing over an existing installation replaces every component's
    /// files, even when the very same version is installed already.
    #[test]
    fn reinstall_overwrites_component_files() {
        let d = temp_dir("reinstall");
        let layout = Layout::new(d.join("Zenless"));
        let ext_v1 = make_zip(&[("manifest.json", br#"{"version":"0.2.0"}"#), ("js/a.js", b"v1"), ("js/gone.js", b"x")]);
        let ext_v2 = make_zip(&[("manifest.json", br#"{"version":"0.2.0"}"#), ("js/a.js", b"v2"), ("popup.html", b"<p>")]);
        let mut first = Journal::new(Env::with_sandbox(Some(d.join("sb"))));
        for (c, data) in [(Component::Dm, b"MZ same version".as_slice()), (Component::Firefox, b"PK same".as_slice())] {
            install_component_files(&mut first, &layout, c, data, &Quiet).unwrap();
        }
        install_component_files(&mut first, &layout, Component::Chrome, &ext_v1, &Quiet).unwrap();
        first.commit(&Quiet);
        // Damage the installed files a bit.
        std::fs::write(layout.dm_exe(), b"MZ damaged").unwrap();
        std::fs::write(layout.chrome_dir().join("js").join("a.js"), b"edited").unwrap();

        let mut again = Journal::new(Env::with_sandbox(Some(d.join("sb"))));
        install_component_files(&mut again, &layout, Component::Dm, b"MZ same version", &Quiet).unwrap();
        install_component_files(&mut again, &layout, Component::Firefox, b"PK same", &Quiet).unwrap();
        install_component_files(&mut again, &layout, Component::Chrome, &ext_v2, &Quiet).unwrap();
        again.commit(&Quiet);
        assert_eq!(std::fs::read(layout.dm_exe()).unwrap(), b"MZ same version");
        assert_eq!(std::fs::read(layout.firefox_xpi()).unwrap(), b"PK same");
        let chrome = layout.chrome_dir();
        assert_eq!(std::fs::read(chrome.join("js").join("a.js")).unwrap(), b"v2");
        assert!(chrome.join("popup.html").is_file());
        assert!(!chrome.join("js").join("gone.js").exists());
        assert_eq!(clean_old_files(&layout.root), 0, "no backups left behind");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// When the Chrome folder itself can't be renamed (another program has it
    /// open) its files are replaced in place, and a rollback restores them.
    #[cfg(windows)]
    #[test]
    fn busy_chrome_folder_is_replaced_in_place() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ_WRITE: u32 = 0x1 | 0x2; // no FILE_SHARE_DELETE
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000; // needed to open a folder

        let d = temp_dir("busy-chrome");
        let dir = d.join("Chrome");
        std::fs::create_dir_all(dir.join("js")).unwrap();
        std::fs::write(dir.join("manifest.json"), b"old").unwrap();
        std::fs::write(dir.join("js").join("background.js"), b"old").unwrap();
        std::fs::write(dir.join("js").join("removed.js"), b"old").unwrap();
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&dir)
            .unwrap();
        assert!(std::fs::rename(&dir, d.join("renamed")).is_err(), "the folder should be busy");

        let zip = make_zip(&[("manifest.json", b"new"), ("js/background.js", b"new"), ("popup.html", b"<p>")]);
        let snapshot = |dir: &Path| -> Vec<(PathBuf, Vec<u8>)> {
            files_below(dir).unwrap().into_iter().map(|rel| (rel.clone(), std::fs::read(dir.join(&rel)).unwrap())).collect()
        };
        let before = snapshot(&dir);

        // Rollback brings the old files back.
        let mut j = Journal::new(Env::with_sandbox(Some(d.join("sb"))));
        assert_eq!(install_chrome_extension(&mut j, &dir, &zip, &Quiet).unwrap(), 3);
        assert_eq!(std::fs::read(dir.join("manifest.json")).unwrap(), b"new");
        j.rollback(&Quiet);
        assert_eq!(snapshot(&dir), before);

        // Commit keeps exactly the new files.
        let mut j = Journal::new(Env::with_sandbox(Some(d.join("sb"))));
        install_chrome_extension(&mut j, &dir, &zip, &Quiet).unwrap();
        j.commit(&Quiet);
        let after: Vec<PathBuf> = snapshot(&dir).into_iter().map(|(p, _)| p).collect();
        let expected: Vec<PathBuf> =
            vec![Path::new("js").join("background.js"), PathBuf::from("manifest.json"), PathBuf::from("popup.html")];
        assert_eq!(after, expected);
        assert_eq!(std::fs::read(dir.join("js").join("background.js")).unwrap(), b"new");
        assert!(!d.join("Chrome.new").exists());
        drop(handle);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn manifest_prefixes() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(manifest_prefix(&v(&["manifest.json", "a/manifest.json"])), Some(String::new()));
        assert_eq!(manifest_prefix(&v(&["ext/manifest.json", "ext/a.js"])), Some("ext/".into()));
        assert_eq!(manifest_prefix(&v(&["a/b/manifest.json"])), None);
    }

    #[test]
    fn root_validation() {
        assert!(validate_root(Path::new("")).is_err());
        assert!(validate_root(Path::new("relative\\dir")).is_err());
        #[cfg(windows)]
        {
            assert!(validate_root(Path::new(r"C:\")).is_err());
            assert!(validate_root(Path::new(r"C:\Users\me\AppData\Local\Programs\Zenless")).is_ok());
            if let Some(pf) = std::env::var_os("ProgramFiles") {
                assert!(validate_root(&PathBuf::from(pf).join("Zenless")).is_err());
            }
        }
    }
}
