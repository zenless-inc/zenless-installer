//! The installable parts of the Zenless Suite and where they live on disk.

use crate::payload::{self, Embedded};
use std::path::{Path, PathBuf};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GITHUB_ORG: &str = "zenless-inc";
pub const WEBSITE: &str = "https://zenless-suite.vercel.app";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Component {
    Dm,
    Torrent,
    Chrome,
    Firefox,
}

impl Component {
    pub const ALL: [Component; 4] = [Self::Dm, Self::Torrent, Self::Chrome, Self::Firefox];

    /// Stable identifier used on the command line and in the registry.
    pub fn key(self) -> &'static str {
        match self {
            Self::Dm => "dm",
            Self::Torrent => "torrent",
            Self::Chrome => "chrome",
            Self::Firefox => "firefox",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "dm" | "download-manager" | "downloadmanager" => Some(Self::Dm),
            "torrent" | "bittorrent" => Some(Self::Torrent),
            "chrome" | "chromium" => Some(Self::Chrome),
            "firefox" | "ff" => Some(Self::Firefox),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Dm => "Zenless Download Manager",
            Self::Torrent => "Zenless Torrent",
            Self::Chrome => "Chrome / Chromium extension",
            Self::Firefox => "Firefox extension",
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            Self::Dm => "Download Manager",
            Self::Torrent => "Torrent",
            Self::Chrome => "Chrome extension",
            Self::Firefox => "Firefox extension",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Dm => "Multi-connection downloads with pause, resume, queues and speed limits.",
            Self::Torrent => "A calm, fast BitTorrent client for magnet links and .torrent files.",
            Self::Chrome => "Sends downloads to Zenless from Chrome, Edge, Brave, Vivaldi and Opera.",
            Self::Firefox => "Sends downloads and magnet links to Zenless from Firefox.",
        }
    }

    pub fn is_app(self) -> bool {
        matches!(self, Self::Dm | Self::Torrent)
    }

    /// GitHub repository that publishes this component.
    pub fn repo(self) -> &'static str {
        match self {
            Self::Dm => "zenless-download-manager",
            Self::Torrent => "zenless-torrent-client",
            Self::Chrome => "zenless-chrome-extension",
            Self::Firefox => "zenless-firefox-extension",
        }
    }

    /// Release artifact / payload file name.
    pub fn artifact(self) -> &'static str {
        self.embedded().file
    }

    pub fn embedded(self) -> &'static Embedded {
        match self {
            Self::Dm => &payload::DM,
            Self::Torrent => &payload::TORRENT,
            Self::Chrome => &payload::CHROME,
            Self::Firefox => &payload::FIREFOX,
        }
    }

    /// Where an online install downloads this component from.
    /// `ZENLESS_INSTALLER_DOWNLOAD_BASE` overrides the host (used for testing).
    pub fn download_url(self) -> String {
        match std::env::var("ZENLESS_INSTALLER_DOWNLOAD_BASE") {
            Ok(base) if !base.trim().is_empty() => {
                format!("{}/{}", base.trim().trim_end_matches('/'), self.artifact())
            }
            _ => format!(
                "https://github.com/{GITHUB_ORG}/{}/releases/latest/download/{}",
                self.repo(),
                self.artifact()
            ),
        }
    }

    /// Rough download size shown when the payload is not embedded.
    pub fn approx_download_size(self) -> u64 {
        match self {
            Self::Dm => 9 << 20,
            Self::Torrent => 16 << 20,
            Self::Chrome | Self::Firefox => 80 << 10,
        }
    }

    /// Size on disk after installation (exact when embedded, estimated otherwise).
    pub fn install_size(self) -> u64 {
        let e = self.embedded();
        if e.data.is_some() { e.size } else { self.approx_download_size() }
    }

    /// Largest download we accept (sanity check against HTML error pages / junk).
    pub fn max_download_size(self) -> u64 {
        match self {
            Self::Dm | Self::Torrent => 512 << 20,
            Self::Chrome | Self::Firefox => 64 << 20,
        }
    }

    /// Port of the app's local integration API.
    pub fn api_port(self) -> Option<u16> {
        match self {
            Self::Dm => Some(6812),
            Self::Torrent => Some(6813),
            _ => None,
        }
    }
}

/// `"dm,torrent"` → components. Unknown names are an error; duplicates are removed.
pub fn parse_list(s: &str) -> Result<Vec<Component>, String> {
    let mut out = Vec::new();
    for part in s.split([',', ';', ' ']).filter(|p| !p.trim().is_empty()) {
        let c = Component::from_key(part).ok_or_else(|| {
            format!("unknown component {:?} (expected dm, torrent, chrome, firefox)", part.trim())
        })?;
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out.sort();
    Ok(out)
}

/// Components → `"dm,torrent"`.
pub fn join_keys(list: &[Component]) -> String {
    let mut v = list.to_vec();
    v.sort();
    v.dedup();
    v.iter().map(|c| c.key()).collect::<Vec<_>>().join(",")
}

/// Paths inside an installation root (see the suite spec for the layout).
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn dm_exe(&self) -> PathBuf {
        self.root.join("Download Manager").join("zenless-dm.exe")
    }

    pub fn torrent_exe(&self) -> PathBuf {
        self.root.join("Torrent").join("zenless-torrent.exe")
    }

    pub fn extensions_dir(&self) -> PathBuf {
        self.root.join("Browser Extensions")
    }

    pub fn chrome_dir(&self) -> PathBuf {
        self.extensions_dir().join("Chrome")
    }

    pub fn firefox_xpi(&self) -> PathBuf {
        self.extensions_dir().join("zenless-firefox-extension.xpi")
    }

    pub fn uninstaller(&self) -> PathBuf {
        self.root.join("uninstall.exe")
    }

    pub fn app_exe(&self, c: Component) -> Option<PathBuf> {
        match c {
            Component::Dm => Some(self.dm_exe()),
            Component::Torrent => Some(self.torrent_exe()),
            _ => None,
        }
    }

    /// The file or folder a component occupies.
    pub fn component_path(&self, c: Component) -> PathBuf {
        match c {
            Component::Dm => self.dm_exe(),
            Component::Torrent => self.torrent_exe(),
            Component::Chrome => self.chrome_dir(),
            Component::Firefox => self.firefox_xpi(),
        }
    }

    /// The folder that is removed when the component is uninstalled.
    pub fn component_dir(&self, c: Component) -> Option<PathBuf> {
        match c {
            Component::Dm => self.dm_exe().parent().map(Path::to_path_buf),
            Component::Torrent => self.torrent_exe().parent().map(Path::to_path_buf),
            Component::Chrome => Some(self.chrome_dir()),
            Component::Firefox => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_components() {
        assert_eq!(parse_list("dm,torrent").unwrap(), vec![Component::Dm, Component::Torrent]);
        assert_eq!(
            parse_list(" firefox ; chrome,dm,dm").unwrap(),
            vec![Component::Dm, Component::Chrome, Component::Firefox]
        );
        assert!(parse_list("dm,opera").is_err());
        assert!(parse_list("").unwrap().is_empty());
        assert_eq!(join_keys(&[Component::Firefox, Component::Dm]), "dm,firefox");
    }

    #[test]
    fn layout_paths() {
        let l = Layout::new("Z");
        assert_eq!(l.dm_exe(), Path::new("Z").join("Download Manager").join("zenless-dm.exe"));
        assert_eq!(l.chrome_dir(), Path::new("Z").join("Browser Extensions").join("Chrome"));
        assert_eq!(l.uninstaller(), Path::new("Z").join("uninstall.exe"));
        assert_eq!(l.component_dir(Component::Firefox), None);
    }

    #[test]
    fn download_urls() {
        if std::env::var_os("ZENLESS_INSTALLER_DOWNLOAD_BASE").is_none() {
            assert_eq!(
                Component::Torrent.download_url(),
                "https://github.com/zenless-inc/zenless-torrent-client/releases/latest/download/zenless-torrent.exe"
            );
        }
    }
}
