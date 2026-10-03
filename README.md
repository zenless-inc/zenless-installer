# Zenless Setup

The graphical installer **and** uninstaller for the Zenless Suite — one small, native
Windows program (Rust + egui) that lets you install any mix of:

| Component | What it is | Installed to |
|---|---|---|
| **Zenless Download Manager** | Multi-connection download manager | `Download Manager\zenless-dm.exe` |
| **Zenless Torrent** | BitTorrent client | `Torrent\zenless-torrent.exe` |
| **Chrome / Chromium extension** | Browser integration for Chrome, Edge, Brave, Vivaldi, Opera, Helium | `Browser Extensions\Chrome\` (unpacked) |
| **Firefox extension** | Browser integration for Firefox | `Browser Extensions\zenless-firefox-extension.xpi` |

They are separate apps; Setup only installs them. Everything is **per-user**
(`%LOCALAPPDATA%\Programs\Zenless` by default) and never needs administrator rights —
the exe carries an `asInvoker` manifest so Windows does not force a UAC prompt on a
program called "Setup".

### What's new in 0.2.1

- Bundles Zenless Download Manager 0.2.1: files up to 100 MB use 2 connections by default, so
  sites don't mistake the download for a bot (adjustable in Settings → General).

### What's new in 0.2.0

- Bundles Zenless Download Manager 0.2.0, Zenless Torrent 0.2.0 and the 0.2.0 browser
  extensions (the apps now update themselves; the Download Manager also refreshes the
  installed Chrome extension).
- Finish page: browser setup that actually works — the extensions address is copied for
  pasting (browsers ignore it on the command line), *Show folder* selects the extension folder
  in Explorer for drag-and-drop, Helium is detected, and Firefox explains Mozilla signing
  instead of offering an install that can only fail.
- Removes the leftovers of an unreleased pre-release build (see *Legacy clean-up*).
- Installing over an existing installation always replaces the files, also for the same version.

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
7. **Finish** — launch the apps and connect your browsers (see below).

### Connecting the browsers (Finish page)

**Chromium browsers** (Chrome, Edge, Brave, Vivaldi, Opera and Helium are detected —
Helium by its folder `%LOCALAPPDATA%\imput\Helium\Application\chrome.exe`, because it
registers itself as `chrome.exe`). Browsers ignore `chrome://extensions`, `edge://extensions`
and friends when another program passes them on the command line (they open a New Tab page),
so the page walks through three steps:

1. **Open the extensions page** — *Open Chrome* / *Open Helium* / *Open Edge* … copies the
   browser's extensions address (`chrome://extensions`, `edge://extensions`, `brave://…`,
   `vivaldi://…`, `opera://…`) to the clipboard and opens the browser on the website guide
   (`https://zenless-suite.vercel.app/download#chromium`). Paste the address into the address
   bar: **Ctrl+L, Ctrl+V, Enter**.
2. **Turn on Developer mode** on that page.
3. **Drag the Chrome folder onto the page.** *Show folder* opens Explorer with
   `Browser Extensions\Chrome` selected (`explorer.exe /select,"…"`), ready to drag; or click
   *Load unpacked* and paste the path from *Copy folder path*.

**Firefox** only installs add-ons signed by Mozilla permanently. Setup looks inside the
`.xpi` for a Mozilla signature (`META-INF/mozilla.rsa` or `META-INF/cose.sig`):

- **Unsigned** (current builds — signing is pending): no install button (it could only fail).
  *Load temporarily* copies `about:debugging#/runtime/this-firefox` and starts Firefox with it
  (Firefox does open this address from the command line; the copy is a fallback), then *Load
  Temporary Add-on…* with the path from *Copy file path*. Temporary add-ons are removed when
  Firefox closes.
- **Signed**: *Install in Firefox* opens the `.xpi` in Firefox, which asks to add it.

What Setup changes on the system (all under `HKCU`):

- `Software\Microsoft\Windows\CurrentVersion\Uninstall\ZenlessSuite` (Apps & features entry,
  with `UninstallString`, `QuietUninstallString`, `ModifyPath`, `EstimatedSize`, …)
- `Software\Zenless\Suite` (`InstallDir`, `Version`, `Components`)
- optional `Run` values `ZenlessDownloadManager` / `ZenlessTorrent`
- optional `Software\Classes\magnet`, `.torrent` and `Zenless.Torrent` associations
- Start menu folder `Zenless` (apps + "Uninstall Zenless") and optional desktop shortcuts

Before replacing files Setup asks running apps to exit (`POST http://127.0.0.1:6812/quit`
and `:6813/quit`); an exe that is still locked after 5 s is renamed to `.old` and removed on
the next run. Installing over an existing installation always replaces every selected
component's files, even when the same version is installed (that doubles as a repair). If
the unpacked Chrome folder can't be renamed because a program has it open, its files are
replaced one by one instead.

**Legacy clean-up.** An early, never released build registered things no release creates.
After every install, modify and uninstall Setup removes them if present (exact names only):
the scheduled task `Zenless Update` (`schtasks /Delete /TN "Zenless Update" /F`) and the files
`%APPDATA%\Zenless\update-state.json`, `update.lock`, `update.log`, `updates.json`,
`%APPDATA%\Zenless\DownloadManager\last-version` and `%APPDATA%\Zenless\Torrent\last-version`.
That old task runs `uninstall.exe --update --background`; after an upgrade that exe is this
Setup, which then just does the same clean-up silently and exits with code 0. (The apps now
update themselves.)

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
ZenlessSetup.exe --update [--background]           legacy: clean-up only (see above), no window, exit 0
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
folder is `<dir>\install`, settings to `<dir>\config`, and Setup never launches browsers,
Explorer or apps and never asks real apps to quit. The legacy clean-up removes the leftover
files from `<dir>\config\Zenless\…` and only logs that it would remove the `Zenless Update`
task. A sandboxed uninstall removes the sandbox registry key
again. (The sandbox registry root is shared, so a second sandbox folder sees the first
sandbox install as "already installed" until it is uninstalled.) `ZENLESS_INSTALLER_DOWNLOAD_BASE=http://127.0.0.1:8000` makes online installs download
from a local server instead of GitHub.

### Screenshots

`ZENLESS_INSTALLER_PAGE=<page>` opens a page with demo data, `ZENLESS_SCREENSHOT=<file.png>`
saves a screenshot and exits. Pages: `welcome`, `welcome-installed`, `license`, `components`,
`components-modify`, `options`, `appearance`, `installing`, `failed`, `finish`,
`finish-copied` (after clicking *Open Helium*), `finish-firefox` (unsigned add-on only),
`finish-firefox-signed`, `uninstall`, `uninstalling`, `uninstalled` (or `1`–`7` for the
install steps). With a demo page, `ZENLESS_INSTALLER_THEME=<name>` (e.g. `Paper`) previews
another theme without touching `appearance.json`.

## License

MIT — see [LICENSE](LICENSE).
