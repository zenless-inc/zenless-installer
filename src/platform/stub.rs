//! Non-Windows stand-ins so the installer compiles (and its pure logic can be
//! tested) everywhere. Registry and shortcut operations report "unsupported".

use super::{Browser, RawValue};
use std::ffi::OsStr;
use std::io;
use std::path::Path;

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "only supported on Windows")
}

pub fn reg_get_string(_path: &str, _name: &str) -> Option<String> {
    None
}
pub fn reg_get_dword(_path: &str, _name: &str) -> Option<u32> {
    None
}
pub fn reg_get_raw(_path: &str, _name: &str) -> Option<RawValue> {
    None
}
pub fn reg_set_raw(_path: &str, _name: &str, _value: &RawValue) -> io::Result<()> {
    Err(unsupported())
}
pub fn reg_set_string(_path: &str, _name: &str, _value: &str) -> io::Result<()> {
    Err(unsupported())
}
pub fn reg_set_dword(_path: &str, _name: &str, _value: u32) -> io::Result<()> {
    Err(unsupported())
}
pub fn reg_delete_value(_path: &str, _name: &str) -> io::Result<()> {
    Ok(())
}
pub fn reg_delete_tree(_path: &str) -> io::Result<()> {
    Ok(())
}
pub fn reg_key_exists(_path: &str) -> bool {
    false
}
pub fn reg_key_is_empty(_path: &str) -> bool {
    false
}

pub fn create_shortcut(_lnk: &Path, _target: &Path, _args: Option<&str>, _description: &str) -> io::Result<()> {
    Err(unsupported())
}

pub fn detect_browsers() -> Vec<Browser> {
    Vec::new()
}

pub fn launch(exe: &Path, args: &[&OsStr]) -> io::Result<()> {
    std::process::Command::new(exe).args(args).spawn().map(|_| ())
}

pub fn shell_open(_target: &OsStr) -> io::Result<()> {
    Err(unsupported())
}

pub fn spawn_hidden_cmd(_command: &str) -> io::Result<()> {
    Err(unsupported())
}

pub fn free_space(_path: &Path) -> Option<u64> {
    None
}

pub fn notify_assoc_changed() {}

pub fn attach_parent_console() -> bool {
    true
}
