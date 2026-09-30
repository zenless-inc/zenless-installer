//! Operating-system integration: registry, shortcuts, browsers, processes.
//!
//! Everything that touches the machine outside the install folder goes
//! through [`Env`], which implements **sandbox mode**: when
//! `ZENLESS_INSTALLER_SANDBOX=<dir>` is set, registry writes go below
//! `HKCU\Software\ZenlessSandbox\…`, shortcuts into `<dir>\shortcuts`, the
//! default install folder is `<dir>\install`, settings live in `<dir>\config`,
//! and no browser or real app is ever launched or asked to quit.
//!
//! The Windows implementation lives in `windows.rs`; `stub.rs` keeps
//! `cargo check` working on other platforms.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

#[cfg(not(windows))]
mod stub;
#[cfg(not(windows))]
use stub as sys;

pub use sys::{
    attach_parent_console, detect_browsers, free_space, launch, notify_assoc_changed, shell_open,
    spawn_hidden_cmd,
};

pub const SANDBOX_VAR: &str = "ZENLESS_INSTALLER_SANDBOX";
/// Registry key (below HKCU) that receives every write in sandbox mode.
pub const SANDBOX_REG_ROOT: &str = r"Software\ZenlessSandbox";

/// Real registry locations (relative to HKCU). Always pass these through
/// [`Reg`], which maps them into the sandbox when needed.
pub mod keys {
    pub const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\ZenlessSuite";
    pub const ZENLESS: &str = r"Software\Zenless";
    pub const STATE: &str = r"Software\Zenless\Suite";
    pub const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    pub const RUN_DM: &str = "ZenlessDownloadManager";
    pub const RUN_TORRENT: &str = "ZenlessTorrent";
    pub const MAGNET: &str = r"Software\Classes\magnet";
    pub const TORRENT_EXT: &str = r"Software\Classes\.torrent";
    pub const PROGID: &str = "Zenless.Torrent";
    pub const TORRENT_PROGID: &str = r"Software\Classes\Zenless.Torrent";

    /// Chrome "external extension" key for a Web Store id.
    pub fn chrome_external(id: &str) -> String {
        format!(r"Software\Google\Chrome\Extensions\{id}")
    }
}

/// Where side effects go. Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct Env {
    sandbox: Option<PathBuf>,
}

impl Env {
    /// Reads `ZENLESS_INSTALLER_SANDBOX`.
    pub fn from_env() -> Self {
        let sandbox = std::env::var_os(SANDBOX_VAR)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .map(|p| std::path::absolute(&p).unwrap_or(p));
        Self::with_sandbox(sandbox)
    }

    pub fn with_sandbox(sandbox: Option<PathBuf>) -> Self {
        Self { sandbox }
    }

    pub fn sandbox(&self) -> Option<&Path> {
        self.sandbox.as_deref()
    }

    pub fn is_sandbox(&self) -> bool {
        self.sandbox.is_some()
    }

    /// Maps a real HKCU-relative key path into the sandbox when enabled.
    pub fn reg_path(&self, real: &str) -> String {
        let real = real.trim_matches('\\');
        match self.sandbox {
            Some(_) => format!(r"{SANDBOX_REG_ROOT}\{real}"),
            None => real.to_owned(),
        }
    }

    pub fn reg(&self) -> Reg {
        Reg { env: self.clone() }
    }

    /// `%LOCALAPPDATA%\Programs\Zenless` (or `<sandbox>\install`).
    pub fn default_install_dir(&self) -> PathBuf {
        match &self.sandbox {
            Some(s) => s.join("install"),
            None => dirs::data_local_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("Programs")
                .join("Zenless"),
        }
    }

    /// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Zenless`.
    pub fn start_menu_dir(&self) -> PathBuf {
        match &self.sandbox {
            Some(s) => s.join("shortcuts").join("Start Menu").join("Programs").join("Zenless"),
            None => dirs::data_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Zenless"),
        }
    }

    pub fn desktop_dir(&self) -> PathBuf {
        match &self.sandbox {
            Some(s) => s.join("shortcuts").join("Desktop"),
            None => dirs::desktop_dir().unwrap_or_else(|| {
                dirs::home_dir().unwrap_or_else(std::env::temp_dir).join("Desktop")
            }),
        }
    }

    /// Shared Zenless settings folder (`%APPDATA%\Zenless`).
    pub fn config_dir(&self) -> PathBuf {
        match &self.sandbox {
            Some(s) => s.join("config").join("Zenless"),
            None => crate::shared::theme::zenless_config_dir(),
        }
    }

    pub fn appearance_path(&self) -> PathBuf {
        self.config_dir().join("appearance.json")
    }
}

/// A registry value as stored (type + raw bytes), used to restore previous values.
#[derive(Clone, Debug, PartialEq)]
pub struct RawValue {
    pub vtype: u32,
    pub bytes: Vec<u8>,
}

/// HKCU registry access through the sandbox mapping.
pub struct Reg {
    env: Env,
}

impl Reg {
    fn map(&self, real: &str) -> String {
        self.env.reg_path(real)
    }

    pub fn get_string(&self, path: &str, name: &str) -> Option<String> {
        sys::reg_get_string(&self.map(path), name)
    }

    pub fn get_dword(&self, path: &str, name: &str) -> Option<u32> {
        sys::reg_get_dword(&self.map(path), name)
    }

    pub fn get_raw(&self, path: &str, name: &str) -> Option<RawValue> {
        sys::reg_get_raw(&self.map(path), name)
    }

    pub fn set_raw(&self, path: &str, name: &str, value: &RawValue) -> io::Result<()> {
        sys::reg_set_raw(&self.map(path), name, value)
    }

    pub fn set_string(&self, path: &str, name: &str, value: &str) -> io::Result<()> {
        sys::reg_set_string(&self.map(path), name, value)
    }

    pub fn set_dword(&self, path: &str, name: &str, value: u32) -> io::Result<()> {
        sys::reg_set_dword(&self.map(path), name, value)
    }

    /// Deletes a value; a missing value is not an error.
    pub fn delete_value(&self, path: &str, name: &str) -> io::Result<()> {
        sys::reg_delete_value(&self.map(path), name)
    }

    /// Deletes a key and everything below it; a missing key is not an error.
    pub fn delete_tree(&self, path: &str) -> io::Result<()> {
        sys::reg_delete_tree(&self.map(path))
    }

    pub fn key_exists(&self, path: &str) -> bool {
        sys::reg_key_exists(&self.map(path))
    }

    /// `true` when the key exists and has neither values nor subkeys.
    pub fn key_is_empty(&self, path: &str) -> bool {
        sys::reg_key_is_empty(&self.map(path))
    }

    /// Walks up from `path`, deleting keys that became empty, stopping at `stop`
    /// (exclusive). In sandbox mode the walk continues up to (and including) the
    /// sandbox root so tests leave nothing behind.
    /// Sandbox only: removes keys left empty below the sandbox root, so tests
    /// leave nothing behind. A no-op on the real registry, where parent keys
    /// such as `Run` or `Classes` belong to Windows.
    pub fn prune_sandbox(&self, path: &str) {
        if self.env.is_sandbox() {
            self.prune_empty(path, "");
        }
    }

    pub fn prune_empty(&self, path: &str, stop: &str) {
        let sandbox = self.env.is_sandbox();
        let mut cur = self.map(path);
        let stop = (!sandbox).then(|| self.map(stop));
        loop {
            if stop.as_deref().is_some_and(|s| cur.eq_ignore_ascii_case(s)) {
                return;
            }
            if sys::reg_key_exists(&cur)
                && (!sys::reg_key_is_empty(&cur) || sys::reg_delete_tree(&cur).is_err())
            {
                return;
            }
            if sandbox && cur.eq_ignore_ascii_case(SANDBOX_REG_ROOT) {
                return;
            }
            match cur.rfind('\\') {
                Some(i) => cur.truncate(i),
                None => return,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Browsers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserKind {
    Chrome,
    Edge,
    Brave,
    Vivaldi,
    Opera,
    Firefox,
}

impl BrowserKind {
    pub const ALL: [BrowserKind; 6] =
        [Self::Chrome, Self::Edge, Self::Brave, Self::Vivaldi, Self::Opera, Self::Firefox];

    pub fn name(self) -> &'static str {
        match self {
            Self::Chrome => "Google Chrome",
            Self::Edge => "Microsoft Edge",
            Self::Brave => "Brave",
            Self::Vivaldi => "Vivaldi",
            Self::Opera => "Opera",
            Self::Firefox => "Firefox",
        }
    }

    pub fn is_chromium(self) -> bool {
        !matches!(self, Self::Firefox)
    }

    /// The browser's extensions page.
    pub fn extensions_url(self) -> &'static str {
        match self {
            Self::Chrome => "chrome://extensions",
            Self::Edge => "edge://extensions",
            Self::Brave => "brave://extensions",
            Self::Vivaldi => "vivaldi://extensions",
            Self::Opera => "opera://extensions",
            Self::Firefox => "about:debugging#/runtime/this-firefox",
        }
    }

    /// Executable name registered under `App Paths`.
    pub fn exe_name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome.exe",
            Self::Edge => "msedge.exe",
            Self::Brave => "brave.exe",
            Self::Vivaldi => "vivaldi.exe",
            Self::Opera => "opera.exe",
            Self::Firefox => "firefox.exe",
        }
    }

    /// Typical install locations relative to Program Files / Local AppData.
    pub fn known_paths(self) -> &'static [&'static str] {
        match self {
            Self::Chrome => &[r"Google\Chrome\Application\chrome.exe"],
            Self::Edge => &[r"Microsoft\Edge\Application\msedge.exe"],
            Self::Brave => &[r"BraveSoftware\Brave-Browser\Application\brave.exe"],
            Self::Vivaldi => &[r"Vivaldi\Application\vivaldi.exe"],
            Self::Opera => &[r"Programs\Opera\opera.exe", r"Programs\Opera\launcher.exe", r"Opera\launcher.exe"],
            Self::Firefox => &[r"Mozilla Firefox\firefox.exe"],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Browser {
    pub kind: BrowserKind,
    pub exe: PathBuf,
}

// ---------------------------------------------------------------------------
// Shortcuts & small helpers
// ---------------------------------------------------------------------------

/// Creates a `.lnk` shortcut (parent folders included).
pub fn create_shortcut(lnk: &Path, target: &Path, args: Option<&str>, description: &str) -> io::Result<()> {
    if let Some(parent) = lnk.parent() {
        std::fs::create_dir_all(parent)?;
    }
    sys::create_shortcut(lnk, target, args, description)
}

/// `true` when another process holds the file open (e.g. a running exe).
pub fn is_file_locked(path: &Path) -> bool {
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => false,
        // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
        Err(e) => matches!(e.raw_os_error(), Some(32) | Some(33)),
    }
}

/// Wraps a path in quotes for a command line (`"C:\x y\z.exe"`).
pub fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_registry_mapping() {
        let real = Env::with_sandbox(None);
        assert_eq!(real.reg_path(keys::STATE), r"Software\Zenless\Suite");
        let sb = Env::with_sandbox(Some(PathBuf::from("sb")));
        assert_eq!(sb.reg_path(keys::STATE), r"Software\ZenlessSandbox\Software\Zenless\Suite");
        assert_eq!(
            sb.reg_path(keys::UNINSTALL),
            r"Software\ZenlessSandbox\Software\Microsoft\Windows\CurrentVersion\Uninstall\ZenlessSuite"
        );
        assert_eq!(sb.reg_path(r"\Software\Classes\magnet\"), r"Software\ZenlessSandbox\Software\Classes\magnet");
    }

    #[test]
    fn sandbox_paths() {
        let root = PathBuf::from("sandbox-root");
        let sb = Env::with_sandbox(Some(root.clone()));
        assert_eq!(sb.default_install_dir(), root.join("install"));
        assert!(sb.start_menu_dir().starts_with(root.join("shortcuts")));
        assert!(sb.start_menu_dir().ends_with("Zenless"));
        assert_eq!(sb.desktop_dir(), root.join("shortcuts").join("Desktop"));
        assert_eq!(sb.appearance_path(), root.join("config").join("Zenless").join("appearance.json"));

        let real = Env::with_sandbox(None);
        assert!(!real.default_install_dir().starts_with(&root));
        assert!(real.default_install_dir().ends_with(Path::new("Programs").join("Zenless")));
        assert!(real.start_menu_dir().ends_with(Path::new("Start Menu").join("Programs").join("Zenless")));
    }

    #[test]
    fn quoting() {
        assert_eq!(quoted(Path::new(r"C:\a b\c.exe")), "\"C:\\a b\\c.exe\"");
    }
}
