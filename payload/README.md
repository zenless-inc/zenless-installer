# payload/

Files placed here are compressed and embedded into `ZenlessSetup.exe` by `build.rs`
(offline installer). Anything missing is downloaded from the latest GitHub release
at install time instead (online installer).

| File | Built from | Installed to |
|---|---|---|
| `zenless-dm.exe` | `zenless-download-manager` (`cargo build --release`) | `Download Manager\zenless-dm.exe` |
| `zenless-torrent.exe` | `zenless-torrent-client` (`cargo build --release`) | `Torrent\zenless-torrent.exe` |
| `zenless-chrome-extension.zip` | `zenless-chrome-extension` (zipped, `manifest.json` at the root) | `Browser Extensions\Chrome\` (unpacked) |
| `zenless-firefox-extension.xpi` | `zenless-firefox-extension` (zipped) | `Browser Extensions\zenless-firefox-extension.xpi` |

You normally don't fill this folder by hand: `cargo xtask dist` builds the sibling
repositories and puts the files here. The files are git-ignored; only this README is
tracked. `ZENLESS_PAYLOAD_DIR=<dir>` makes `build.rs` read another folder instead.
