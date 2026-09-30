//! Build automation for Zenless Setup.
//!
//! `cargo xtask dist [--skip-apps] [--no-payload]`
//!
//! 1. builds `../zenless-download-manager` and `../zenless-torrent-client` in
//!    release mode (their target dirs are resolved with `cargo metadata`, so
//!    `CARGO_TARGET_DIR` and per-repo config are respected) and copies the exes
//!    into `payload/`;
//! 2. zips `../zenless-chrome-extension` and `../zenless-firefox-extension`
//!    into `payload/` (extension files only);
//! 3. builds the installer in release mode, which embeds the payload, and
//!    copies it to `dist/ZenlessSetup.exe` (+ a `.sha256` file).
//!
//! Paths can be overridden with `ZENLESS_DM_DIR`, `ZENLESS_TORRENT_DIR`,
//! `ZENLESS_CHROME_EXT_DIR`, `ZENLESS_FIREFOX_EXT_DIR` and `ZENLESS_PAYLOAD_DIR`.

use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const USAGE: &str = "\
cargo xtask dist [--skip-apps] [--no-payload]
cargo xtask pack

  dist           build the apps + extensions, embed them and write dist/ZenlessSetup.exe
  --skip-apps    don't rebuild the apps; reuse the existing release binaries
  --no-payload   build an online installer (everything is downloaded at install time)
  pack           only zip the two browser extensions into payload/

Environment overrides: ZENLESS_DM_DIR, ZENLESS_TORRENT_DIR, ZENLESS_CHROME_EXT_DIR,
ZENLESS_FIREFOX_EXT_DIR, ZENLESS_PAYLOAD_DIR.";

type Result<T> = std::result::Result<T, String>;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("dist") => match dist(&args[1..]) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("\nxtask dist failed: {e}");
                1
            }
        },
        Some("pack") => match pack_extensions(&Paths::resolve()) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("\nxtask pack failed: {e}");
                1
            }
        },
        None | Some("help" | "-h" | "--help") => {
            println!("{USAGE}");
            0
        }
        Some(other) => {
            eprintln!("unknown task {other:?}\n\n{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

struct Paths {
    root: PathBuf,
    dm: PathBuf,
    torrent: PathBuf,
    chrome: PathBuf,
    firefox: PathBuf,
    payload: PathBuf,
    dist: PathBuf,
}

impl Paths {
    fn resolve() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root").to_path_buf();
        let parent = root.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
        let pick = |var: &str, default: &str| env::var_os(var).map(PathBuf::from).unwrap_or_else(|| parent.join(default));
        Self {
            dm: pick("ZENLESS_DM_DIR", "zenless-download-manager"),
            torrent: pick("ZENLESS_TORRENT_DIR", "zenless-torrent-client"),
            chrome: pick("ZENLESS_CHROME_EXT_DIR", "zenless-chrome-extension"),
            firefox: pick("ZENLESS_FIREFOX_EXT_DIR", "zenless-firefox-extension"),
            payload: env::var_os("ZENLESS_PAYLOAD_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("payload")),
            dist: root.join("dist"),
            root,
        }
    }
}

fn dist(args: &[String]) -> Result<()> {
    let mut skip_apps = false;
    let mut no_payload = false;
    for a in args {
        match a.as_str() {
            "--skip-apps" => skip_apps = true,
            "--no-payload" => no_payload = true,
            other => return Err(format!("unknown flag {other:?}\n\n{USAGE}")),
        }
    }
    let paths = Paths::resolve();
    fs::create_dir_all(&paths.dist).map_err(|e| format!("create {}: {e}", paths.dist.display()))?;

    let payload_dir = if no_payload {
        step("Online installer: nothing will be embedded");
        let empty = paths.dist.join(".online-payload");
        let _ = fs::remove_dir_all(&empty);
        fs::create_dir_all(&empty).map_err(|e| e.to_string())?;
        empty
    } else {
        fs::create_dir_all(&paths.payload).map_err(|e| format!("create {}: {e}", paths.payload.display()))?;
        for (dir, bin, file) in [
            (&paths.dm, "zenless-dm", "zenless-dm.exe"),
            (&paths.torrent, "zenless-torrent", "zenless-torrent.exe"),
        ] {
            let exe = app_binary(dir, bin, skip_apps)?;
            copy(&exe, &paths.payload.join(file))?;
        }
        pack_extensions(&paths)?;
        println!();
        for file in ["zenless-dm.exe", "zenless-torrent.exe", "zenless-chrome-extension.zip", "zenless-firefox-extension.xpi"] {
            let size = fs::metadata(paths.payload.join(file)).map(|m| m.len()).unwrap_or(0);
            println!("    payload/{file:<32} {:>10}", human(size));
        }
        paths.payload.clone()
    };

    step("Building ZenlessSetup (release)");
    let mut cmd = cargo();
    cmd.args(["build", "--release", "--package", "zenless-installer", "--bin", "ZenlessSetup"])
        .current_dir(&paths.root);
    if no_payload || env::var_os("ZENLESS_PAYLOAD_DIR").is_some() {
        cmd.env("ZENLESS_PAYLOAD_DIR", &payload_dir);
    }
    run(&mut cmd)?;
    let target = target_dir(&paths.root)?;
    let exe = target.join("release").join(format!("ZenlessSetup{}", env::consts::EXE_SUFFIX));
    let out = paths.dist.join(format!("ZenlessSetup{}", env::consts::EXE_SUFFIX));
    copy(&exe, &out)?;

    let bytes = fs::read(&out).map_err(|e| e.to_string())?;
    let sha = hex(&Sha256::digest(&bytes));
    let name = out.file_name().unwrap().to_string_lossy().into_owned();
    fs::write(paths.dist.join(format!("{name}.sha256")), format!("{sha}  {name}\n")).map_err(|e| e.to_string())?;
    println!("\n  {}  {}\n  SHA-256 {sha}", out.display(), human(bytes.len() as u64));
    if no_payload {
        let _ = fs::remove_dir_all(&payload_dir);
    }
    Ok(())
}

/// Zips both browser extensions into the payload folder.
fn pack_extensions(paths: &Paths) -> Result<()> {
    fs::create_dir_all(&paths.payload).map_err(|e| format!("create {}: {e}", paths.payload.display()))?;
    for (dir, file) in [
        (&paths.chrome, "zenless-chrome-extension.zip"),
        (&paths.firefox, "zenless-firefox-extension.xpi"),
    ] {
        step(&format!("Packing {} → {file}", dir.display()));
        let n = zip_extension(dir, &paths.payload.join(file))?;
        println!("    {n} files");
    }
    Ok(())
}

fn step(text: &str) {
    println!("\n==> {text}");
}

fn cargo() -> Command {
    let mut cmd = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    // Don't leak the variables cargo set for running *this* binary into nested builds.
    for (key, _) in env::vars_os() {
        let k = key.to_string_lossy();
        if k.starts_with("CARGO_PKG_")
            || k.starts_with("CARGO_BIN_")
            || k.starts_with("CARGO_CRATE_")
            || k == "CARGO_MANIFEST_DIR"
            || k == "CARGO_MANIFEST_PATH"
            || k == "CARGO_PRIMARY_PACKAGE"
        {
            cmd.env_remove(&key);
        }
    }
    cmd
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().map_err(|e| format!("could not run {cmd:?}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{cmd:?} failed ({status})")) }
}

/// Target directory of the workspace in `dir` (respects CARGO_TARGET_DIR and config).
fn target_dir(dir: &Path) -> Result<PathBuf> {
    let out = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cargo metadata in {}: {e}", dir.display()))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    json["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| "cargo metadata did not report a target directory".into())
}

/// Builds (unless `skip`) an app repo and returns its release binary.
fn app_binary(dir: &Path, bin: &str, skip: bool) -> Result<PathBuf> {
    if !dir.join("Cargo.toml").is_file() {
        return Err(format!(
            "{} not found (clone it next to this repo, set the path via environment, or use --no-payload)",
            dir.display()
        ));
    }
    if skip {
        step(&format!("Reusing {bin} from {}", dir.display()));
    } else {
        step(&format!("Building {bin} (release) in {}", dir.display()));
        run(cargo().args(["build", "--release"]).current_dir(dir))?;
    }
    let exe = target_dir(dir)?.join("release").join(format!("{bin}{}", env::consts::EXE_SUFFIX));
    if !exe.is_file() {
        return Err(format!("{} does not exist{}", exe.display(), if skip { " (run without --skip-apps)" } else { "" }));
    }
    Ok(exe)
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to).map(|_| ()).map_err(|e| format!("copy {} → {}: {e}", from.display(), to.display()))
}

// ---------------------------------------------------------------------------
// Extension packaging
// ---------------------------------------------------------------------------

/// File types that belong in a browser extension package.
const EXT_ALLOWED: &[&str] = &[
    "json", "js", "mjs", "html", "htm", "css", "png", "svg", "jpg", "jpeg", "gif", "webp", "ico", "woff", "woff2",
    "ttf", "otf", "wasm",
];
/// Folders never packaged.
const EXT_SKIP_DIRS: &[&str] = &["node_modules", "tests", "test", "__tests__", "docs", "coverage", "web-ext-artifacts", "target"];
/// Tooling files that happen to have an allowed extension.
const EXT_SKIP_FILES: &[&str] = &["package.json", "package-lock.json", "tsconfig.json", "jsconfig.json", "web-ext-config.js"];

fn include_file(rel: &Path) -> bool {
    let name = rel.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name.starts_with('.') || EXT_SKIP_FILES.contains(&name.as_str()) || name.starts_with("readme") {
        return false;
    }
    if name.ends_with(".test.js") || name.ends_with(".spec.js") || name.ends_with(".config.js") || name.ends_with(".config.mjs") {
        return false;
    }
    let ext = rel.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    EXT_ALLOWED.contains(&ext.as_str())
}

/// Top-level folders that usually hold dev tooling (`scripts/check.js`…). They
/// are packaged only when `manifest.json` refers to them.
const EXT_TOOLING_DIRS: &[&str] = &["scripts", "tools"];

fn collect(dir: &Path, rel: &Path, skip_top: &[String], out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir.join(rel))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = rel.join(&name);
        if entry.file_type()?.is_dir() {
            let lower = name.to_ascii_lowercase();
            let tooling = rel.as_os_str().is_empty() && skip_top.contains(&lower);
            if !lower.starts_with('.') && !EXT_SKIP_DIRS.contains(&lower.as_str()) && !tooling {
                collect(dir, &child, skip_top, out)?;
            }
        } else if include_file(&child) {
            out.push(child);
        }
    }
    Ok(())
}

/// Zips an unpacked extension (manifest.json at the archive root).
fn zip_extension(src: &Path, out: &Path) -> Result<usize> {
    let manifest = fs::read_to_string(src.join("manifest.json"))
        .map_err(|_| format!("{} has no manifest.json", src.display()))?;
    let skip_top: Vec<String> = EXT_TOOLING_DIRS
        .iter()
        .filter(|d| !manifest.contains(&format!("{d}/")))
        .map(|d| d.to_string())
        .collect();
    let mut files = Vec::new();
    collect(src, Path::new(""), &skip_top, &mut files).map_err(|e| format!("read {}: {e}", src.display()))?;
    files.sort();
    let file = File::create(out).map_err(|e| format!("create {}: {e}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for rel in &files {
        let name = rel.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("/");
        zip.start_file(name.as_str(), opts).map_err(|e| e.to_string())?;
        let mut f = File::open(src.join(rel)).map_err(|e| format!("open {}: {e}", rel.display()))?;
        io::copy(&mut f, &mut zip).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(files.len())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn human(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.2} MB", bytes as f64 / (1 << 20) as f64)
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_file_filter() {
        for ok in ["manifest.json", "js/background.js", "icons/icon-128.png", "_locales/en/messages.json", "popup.html"] {
            assert!(include_file(Path::new(ok)), "{ok}");
        }
        for skip in ["README.md", "package.json", ".eslintrc.json", "key.pem", "notes.txt", "eslint.config.js", "a.test.js"] {
            assert!(!include_file(Path::new(skip)), "{skip}");
        }
    }
}
