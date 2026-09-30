//! Windows implementation of the platform layer (HKCU registry, shortcuts,
//! browser detection, process helpers). Never requires administrator rights.

use super::{Browser, BrowserKind, RawValue};
use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use winreg::enums::*;
use winreg::{RegKey, RegValue};

fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

fn not_found_ok(r: io::Result<()>) -> io::Result<()> {
    match r {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Registry (HKCU, paths already mapped by `Reg`)
// ---------------------------------------------------------------------------

fn reg_type_from_u32(v: u32) -> RegType {
    match v {
        1 => REG_SZ,
        2 => REG_EXPAND_SZ,
        3 => REG_BINARY,
        4 => REG_DWORD,
        5 => REG_DWORD_BIG_ENDIAN,
        6 => REG_LINK,
        7 => REG_MULTI_SZ,
        11 => REG_QWORD,
        _ => REG_NONE,
    }
}

pub fn reg_get_string(path: &str, name: &str) -> Option<String> {
    let key = hkcu().open_subkey_with_flags(path, KEY_READ).ok()?;
    key.get_value::<String, _>(name).ok()
}

pub fn reg_get_dword(path: &str, name: &str) -> Option<u32> {
    let key = hkcu().open_subkey_with_flags(path, KEY_READ).ok()?;
    key.get_value::<u32, _>(name).ok()
}

pub fn reg_get_raw(path: &str, name: &str) -> Option<RawValue> {
    let key = hkcu().open_subkey_with_flags(path, KEY_READ).ok()?;
    let v = key.get_raw_value(name).ok()?;
    Some(RawValue { vtype: v.vtype as u32, bytes: v.bytes.into_owned() })
}

pub fn reg_set_raw(path: &str, name: &str, value: &RawValue) -> io::Result<()> {
    let (key, _) = hkcu().create_subkey(path)?;
    let v = RegValue { bytes: value.bytes.clone().into(), vtype: reg_type_from_u32(value.vtype) };
    key.set_raw_value(name, &v)
}

pub fn reg_set_string(path: &str, name: &str, value: &str) -> io::Result<()> {
    let (key, _) = hkcu().create_subkey(path)?;
    key.set_value(name, &value)
}

pub fn reg_set_dword(path: &str, name: &str, value: u32) -> io::Result<()> {
    let (key, _) = hkcu().create_subkey(path)?;
    key.set_value(name, &value)
}

pub fn reg_delete_value(path: &str, name: &str) -> io::Result<()> {
    let key = match hkcu().open_subkey_with_flags(path, KEY_SET_VALUE) {
        Ok(k) => k,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    not_found_ok(key.delete_value(name))
}

pub fn reg_delete_tree(path: &str) -> io::Result<()> {
    if path.trim_matches('\\').is_empty() {
        return Err(io::Error::other("refusing to delete the HKCU root"));
    }
    not_found_ok(hkcu().delete_subkey_all(path))
}

pub fn reg_key_exists(path: &str) -> bool {
    hkcu().open_subkey_with_flags(path, KEY_READ).is_ok()
}

pub fn reg_key_is_empty(path: &str) -> bool {
    hkcu()
        .open_subkey_with_flags(path, KEY_READ)
        .and_then(|k| k.query_info())
        .is_ok_and(|info| info.sub_keys == 0 && info.values == 0)
}

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

pub fn create_shortcut(lnk: &Path, target: &Path, args: Option<&str>, description: &str) -> io::Result<()> {
    let target_str = target
        .to_str()
        .ok_or_else(|| io::Error::other("shortcut target is not valid Unicode"))?;
    // mslnk builds the target ID list from a drive letter path.
    let b = target_str.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return Err(io::Error::other("shortcut targets must be on a local drive (C:\\...)"));
    }
    let mut sl = mslnk::ShellLink::new(target).map_err(io::Error::other)?;
    sl.set_arguments(args.map(str::to_owned));
    sl.set_name(Some(description.to_owned()));
    sl.set_icon_location(Some(target_str.to_owned()));
    if let Some(dir) = target.parent().and_then(Path::to_str) {
        sl.set_working_dir(Some(dir.to_owned()));
    }
    sl.create_lnk(lnk).map_err(io::Error::other)
}

// ---------------------------------------------------------------------------
// Browsers
// ---------------------------------------------------------------------------

pub fn detect_browsers() -> Vec<Browser> {
    BrowserKind::ALL
        .iter()
        .filter_map(|&kind| find_browser(kind).map(|exe| Browser { kind, exe }))
        .collect()
}

fn find_browser(kind: BrowserKind) -> Option<PathBuf> {
    let app_paths = format!(r"Software\Microsoft\Windows\CurrentVersion\App Paths\{}", kind.exe_name());
    let roots = [
        (HKEY_CURRENT_USER, 0),
        (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY),
        (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY),
    ];
    for (root, flags) in roots {
        if let Ok(key) = RegKey::predef(root).open_subkey_with_flags(&app_paths, KEY_READ | flags)
            && let Ok(value) = key.get_value::<String, _>("")
        {
            let p = PathBuf::from(value.trim().trim_matches('"'));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let bases: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .collect();
    for base in &bases {
        for rel in kind.known_paths() {
            let p = base.join(rel);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Processes & shell
// ---------------------------------------------------------------------------

/// Starts a program without waiting for it (and without inheriting our stdio).
pub fn launch(exe: &Path, args: &[&OsStr]) -> io::Result<()> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn().map(|_| ())
}

/// Opens a folder, file or URL with its default handler (ShellExecute "open").
pub fn shell_open(target: &OsStr) -> io::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let op = wide("open");
    let file = wide(target);
    // SW_SHOWNORMAL = 1
    let h = unsafe {
        ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1)
    };
    // Values <= 32 are errors.
    if h as usize > 32 { Ok(()) } else { Err(io::Error::other(format!("ShellExecute failed ({})", h as usize))) }
}

/// Runs `cmd.exe /c <command>` hidden and detached (used for self-deletion).
pub fn spawn_hidden_cmd(command: &str) -> io::Result<()> {
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    let cmd_exe = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join("cmd.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("cmd.exe"));
    Command::new(cmd_exe)
        .raw_arg(format!("/c {command}"))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

/// Free bytes available to the user on the volume holding `path` (or its
/// nearest existing parent).
pub fn free_space(path: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut p = Some(path);
    while let Some(cur) = p {
        if cur.exists() {
            let w = wide(cur);
            let mut avail = 0u64;
            let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, std::ptr::null_mut(), std::ptr::null_mut()) };
            return (ok != 0).then_some(avail);
        }
        p = cur.parent();
    }
    None
}

/// Tells Explorer that file associations changed.
pub fn notify_assoc_changed() {
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
}

/// Attaches to the console of the parent process so silent mode can print.
pub fn attach_parent_console() -> bool {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}
