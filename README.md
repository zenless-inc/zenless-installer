# Zenless Setup

The graphical installer **and** uninstaller for the Zenless Suite — one small, native
Windows program (Rust + egui) that lets you install any mix of:

| Component | What it is | Installed to |
|---|---|---|
| **Zenless Download Manager** | Multi-connection download manager | `Download Manager\zenless-dm.exe` |
| **Zenless Torrent** | BitTorrent client | `Torrent\zenless-torrent.exe` |
| **Chrome / Chromium extension** | Browser integration for Chrome, Edge, Brave, Vivaldi, Opera | `Browser Extensions\Chrome\` (unpacked) |
| **Firefox extension** | Browser integration for Firefox | `Browser Extensions\zenless-firefox-extension.xpi` |

They are separate apps; Setup only installs them. Everything is **per-user**
(`%LOCALAPPDATA%\Programs\Zenless` by default) and never needs administrator rights —
the exe carries an `asInvoker` manifest so Windows does not force a UAC prompt on a
program called "Setup".

## The wizard

1. **Welcome** — install, or *Modify / Update* and *Uninstall* when Zenless is already installed.
2. **License** — MIT, must be accepted.
3. **Components** — cards with size (embedded) or "Download ~X MB" (online installer).
4. **Options** — install folder with free space, Start menu / desktop shortcuts, start
   Download Manager / Torrent with Windows, make Zenless Torrent the magnet/.torrent handler.
5. **Appearance** — pick one of the shared Zenless themes; the wizard re-themes live and
   the choice is written to `%APPDATA%\Zenless\appearance.json`, so the apps start with it.
6. **Installing** — progress, current step, log, *Cancel* (everything written so far is
   rolled back: files, folders, shortcuts and registry values).
7. **Finish** — launch the apps and connect your browsers: open each detected Chromium
   browser's extensions page, open / copy the unpacked extension folder ("Developer mode →
   Load unpacked → pick this folder"), install the `.xpi` in Firefox or load it temporarily
   from `about:debugging`.

What Setup changes on the system (all under `HKCU`):

- `Software\Microsoft\Windows\CurrentVersion\Uninstall\ZenlessSuite` (Apps & features entry,
  with `UninstallString`, `QuietUninstallString`, `ModifyPath`, `EstimatedSize`, …)
- `Software\Zenless\Suite` (`InstallDir`, `Version`, `Components`)
- optional `Run` values `ZenlessDownloadManager` / `ZenlessTorrent`
- optional `Software\Classes\magnet`, `.torrent` and `Zenless.Torrent` associations
- Start menu folder `Zenless` (apps + "Uninstall Zenless") and optional desktop shortcuts

Before replacing files Setup asks running apps to exit (`POST http://127.0.0.1:6812/quit`
and `:6813/quit`); an exe that is still locked after 5 s is renamed to `.old` and removed on
the next run.

**Uninstall** (Apps & features, the Start menu, or *Uninstall* on the Welcome page) lets you
pick components and optionally remove settings and download history (`%APPDATA%\Zenless`).
Associations and autostart entries are only removed when they point to our exe. The
install folder is deleted after Setup exits (only recursively when nothing but Zenless
files is left in it).

## Command line

```
ZenlessSetup.exe                                   wizard
ZenlessSetup.exe --uninstall                       uninstaller
ZenlessSetup.exe --silent [--components dm,torrent,chrome,firefox] [--dir PATH] [--no-shortcuts]
                 [--no-desktop] [--no-autostart] [--autostart-torrent] [--no-associate]
                 [--theme NAME] [--log FILE]
ZenlessSetup.exe --uninstall --silent [--purge]    (--purge also removes settings)
```

Without `--components`, a silent install installs everything (or updates what is already
installed). Exit codes: `0` ok, `1` failed (changes rolled back), `2` bad arguments,
`3` not installed. Setup is a GUI program: from `cmd` use `start /wait ZenlessSetup.exe --silent`,
from PowerShell `Start-Process -Wait`.

## Building

```bash
cargo build --release          # online installer: downloads components at install time
cargo test
```

### Full offline installer

```bash
cargo xtask dist               # → dist/ZenlessSetup.exe (+ .sha256)
cargo xtask dist --skip-apps   # reuse already built app binaries
cargo xtask dist --no-payload  # online installer
```

`cargo xtask dist` expects the sibling repositories next to this one
(`../zenless-download-manager`, `../zenless-torrent-client`, `../zenless-chrome-extension`,
`../zenless-firefox-extension`; override with `ZENLESS_DM_DIR`, `ZENLESS_TORRENT_DIR`,
`ZENLESS_CHROME_EXT_DIR`, `ZENLESS_FIREFOX_EXT_DIR`). It builds the apps in release mode
(respecting `CARGO_TARGET_DIR`), zips the extensions (extension files only — no `.git`,
tests, READMEs or `node_modules`) into `payload/`, then builds the installer.

`build.rs` compresses whatever is in `payload/` (or `$ZENLESS_PAYLOAD_DIR`) with DEFLATE and
embeds it with its size and SHA-256; missing files are downloaded from
`https://github.com/zenless-inc/<repo>/releases/latest/download/<artifact>` during
installation (with size, file-type and — if the release publishes `<artifact>.sha256` —
checksum verification).

## Testing safely: sandbox mode

```bash
export ZENLESS_INSTALLER_SANDBOX="$PWD/sandbox"
ZenlessSetup.exe --silent
```

With `ZENLESS_INSTALLER_SANDBOX=<dir>` **every** side effect is redirected: registry writes
go to `HKCU\Software\ZenlessSandbox\…`, shortcuts to `<dir>\shortcuts`, the default install
folder is `<dir>\install`, settings to `<dir>\config`, and Setup never launches browsers or
apps and never asks real apps to quit. A sandboxed uninstall removes the sandbox registry key
again. (The sandbox registry root is shared, so a second sandbox folder sees the first
sandbox install as "already installed" until it is uninstalled.) `ZENLESS_INSTALLER_DOWNLOAD_BASE=http://127.0.0.1:8000` makes online installs download
from a local server instead of GitHub.

### Screenshots

`ZENLESS_INSTALLER_PAGE=<page>` opens a page with demo data, `ZENLESS_SCREENSHOT=<file.png>`
saves a screenshot and exits. Pages: `welcome`, `welcome-installed`, `license`, `components`,
`components-modify`, `options`, `appearance`, `installing`, `failed`, `finish`, `uninstall`,
`uninstalling`, `uninstalled` (or `1`–`7` for the install steps).

## License

MIT — see [LICENSE](LICENSE).
