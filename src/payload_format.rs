//! On-disk / in-binary format of embedded payloads.
//!
//! This module is compiled twice: by `build.rs` (to compress the files found in
//! `payload/` and generate `payload.rs`) and by the installer itself (to
//! decompress and verify them). Keeping both sides in one file guarantees they
//! agree. Format: raw DEFLATE stream (miniz_oxide) + uncompressed size + SHA-256.
#![allow(dead_code)] // each side uses a different half of this module

use sha2::{Digest, Sha256};

/// miniz compression level (0-10). 9 is "best" without the very slow level 10.
pub const LEVEL: u8 = 9;

/// The payload files the installer knows about: (constant name, file name).
pub const FILES: [(&str, &str); 4] = [
    ("DM", "zenless-dm.exe"),
    ("TORRENT", "zenless-torrent.exe"),
    ("CHROME", "zenless-chrome-extension.zip"),
    ("FIREFOX", "zenless-firefox-extension.xpi"),
];

/// Lower-case hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Compresses a payload for embedding.
pub fn encode(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(data, LEVEL)
}

/// Decompresses an embedded payload and checks its size and hash.
pub fn decode(compressed: &[u8], size: u64, sha256: &str) -> Result<Vec<u8>, String> {
    let limit = usize::try_from(size).map_err(|_| "payload too large".to_owned())?;
    let data = miniz_oxide::inflate::decompress_to_vec_with_limit(compressed, limit)
        .map_err(|e| format!("embedded payload is corrupt ({:?})", e.status))?;
    if data.len() as u64 != size {
        return Err(format!(
            "embedded payload has the wrong size ({} bytes, expected {size})",
            data.len()
        ));
    }
    let actual = sha256_hex(&data);
    if !actual.eq_ignore_ascii_case(sha256) {
        return Err(format!("embedded payload failed its SHA-256 check ({actual})"));
    }
    Ok(data)
}

/// Rust source for one entry of the generated `payload.rs`.
///
/// `blob` is the file name of the compressed blob inside `OUT_DIR`, or `None`
/// when the payload is not embedded (downloaded at install time instead).
pub fn entry_source(ident: &str, file: &str, blob: Option<&str>, size: u64, sha256: Option<&str>) -> String {
    match (blob, sha256) {
        (Some(blob), Some(sha)) => format!(
            "pub const {ident}: Embedded = Embedded {{ file: {file:?}, \
             data: Some(include_bytes!(concat!(env!(\"OUT_DIR\"), \"/\", {blob:?}))), \
             size: {size}, sha256: Some({sha:?}) }};\n"
        ),
        _ => format!(
            "pub const {ident}: Embedded = Embedded {{ file: {file:?}, data: None, size: 0, sha256: None }};\n"
        ),
    }
}
