//! Component payloads: embedded in the exe by `build.rs`, or downloaded from
//! the GitHub releases at install time (online installer).

use crate::components::Component;
use crate::payload_format;
use crate::report::{CANCELLED, Stage};
use std::io::Read;
use std::time::Duration;

/// One payload file as embedded by `build.rs`.
pub struct Embedded {
    /// Payload / release artifact file name, e.g. `zenless-dm.exe`.
    pub file: &'static str,
    /// Raw-deflate compressed bytes, or `None` for "download at install time".
    pub data: Option<&'static [u8]>,
    /// Uncompressed size in bytes (0 when not embedded).
    pub size: u64,
    /// Lower-case hex SHA-256 of the uncompressed file.
    pub sha256: Option<&'static str>,
}

include!(concat!(env!("OUT_DIR"), "/payload.rs"));

impl Embedded {
    pub fn is_embedded(&self) -> bool {
        self.data.is_some()
    }

    /// Decompresses and verifies the embedded bytes.
    pub fn decode(&self) -> Result<Vec<u8>, String> {
        match (self.data, self.sha256) {
            (Some(data), Some(sha)) => payload_format::decode(data, self.size, sha),
            _ => Err(format!("{} is not embedded in this installer", self.file)),
        }
    }
}

/// `true` when at least one component would have to be downloaded.
pub fn needs_network(components: &[Component]) -> bool {
    components.iter().any(|c| !c.embedded().is_embedded())
}

/// Returns the bytes of a component, decoding the embedded copy or downloading it.
pub fn obtain(c: Component, stage: Stage) -> Result<Vec<u8>, String> {
    let r = stage.reporter;
    let e = c.embedded();
    let data = if e.is_embedded() {
        r.info(&format!("Unpacking {} ({})", e.file, crate::shared::kit::human_bytes(e.size)));
        let data = e.decode()?;
        stage.done();
        data
    } else {
        let url = c.download_url();
        r.info(&format!("Downloading {url}"));
        let data = download(&url, c.max_download_size(), stage)?;
        r.info(&format!(
            "Downloaded {} ({}), SHA-256 {}",
            e.file,
            crate::shared::kit::human_bytes(data.len() as u64),
            payload_format::sha256_hex(&data)
        ));
        verify_published_checksum(&url, &data, stage)?;
        data
    };
    check_file_type(c, &data)?;
    Ok(data)
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("zenless-installer/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(timeout)
        .build()
        .map_err(|e| format!("could not start the HTTP client: {e}"))
}

/// Downloads `url` into memory with progress, cancellation and size checks.
fn download(url: &str, max_size: u64, stage: Stage) -> Result<Vec<u8>, String> {
    let r = stage.reporter;
    let client = client(Duration::from_secs(30 * 60))?;
    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| format!("download failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("download failed: HTTP {status} for {url}"));
    }
    let total = resp.content_length();
    if let Some(total) = total
        && total > max_size
    {
        return Err(format!(
            "download rejected: {} is larger than expected ({})",
            url,
            crate::shared::kit::human_bytes(total)
        ));
    }
    let mut data = Vec::with_capacity(total.unwrap_or(1 << 20).min(max_size) as usize);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if r.cancelled() {
            return Err(CANCELLED.to_owned());
        }
        let n = resp.read(&mut buf).map_err(|e| format!("download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if data.len() as u64 > max_size {
            return Err(format!("download rejected: {url} is larger than expected"));
        }
        if let Some(total) = total {
            stage.set(data.len() as f32 / total.max(1) as f32);
        }
    }
    if let Some(total) = total
        && data.len() as u64 != total
    {
        return Err(format!(
            "download incomplete: got {} of {} bytes",
            data.len(),
            total
        ));
    }
    stage.done();
    Ok(data)
}

/// If the release publishes `<artifact>.sha256`, the download must match it.
fn verify_published_checksum(url: &str, data: &[u8], stage: Stage) -> Result<(), String> {
    let r = stage.reporter;
    let Ok(client) = client(Duration::from_secs(20)) else { return Ok(()) };
    let text = match client.get(format!("{url}.sha256")).send() {
        Ok(resp) if resp.status().is_success() => resp.text().unwrap_or_default(),
        _ => {
            r.info("No published checksum for this file; size and file type checks passed.");
            return Ok(());
        }
    };
    let expected: String = text.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        r.warn("Published checksum file is not in the expected format; ignored.");
        return Ok(());
    }
    let actual = payload_format::sha256_hex(data);
    if actual != expected {
        return Err(format!("checksum mismatch: expected {expected}, got {actual}"));
    }
    r.ok("Checksum matches the published SHA-256.");
    Ok(())
}

/// Rejects obviously wrong content (HTML error pages, truncated files).
pub fn check_file_type(c: Component, data: &[u8]) -> Result<(), String> {
    let (magic, what): (&[u8], &str) = match c {
        Component::Dm | Component::Torrent => (b"MZ", "a Windows program"),
        Component::Chrome | Component::Firefox => (b"PK\x03\x04", "a zip archive"),
    };
    if data.len() < 64 || !data.starts_with(magic) {
        return Err(format!("{} is not {what}; the download looks damaged", c.artifact()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut v = b"MZ".to_vec();
        for i in 0..200_000u32 {
            v.extend_from_slice(&(i % 251).to_le_bytes()[..1]);
            if i % 7 == 0 {
                v.extend_from_slice(b"zenless");
            }
        }
        v
    }

    #[test]
    fn round_trip_through_embed_format() {
        let data = sample();
        let compressed = payload_format::encode(&data);
        assert!(compressed.len() < data.len());
        let sha = payload_format::sha256_hex(&data);

        // What build.rs generates for this payload…
        let src = payload_format::entry_source("DM", "zenless-dm.exe", Some("zenless-dm.exe.deflate"), data.len() as u64, Some(&sha));
        assert!(src.contains("include_bytes!(concat!(env!(\"OUT_DIR\"), \"/\", \"zenless-dm.exe.deflate\"))"));
        assert!(src.contains(&format!("size: {}", data.len())));
        assert!(src.contains(&sha));

        // …and what the installer does with it at runtime.
        let leaked: &'static [u8] = Box::leak(compressed.into_boxed_slice());
        let sha_static: &'static str = Box::leak(sha.into_boxed_str());
        let e = Embedded { file: "zenless-dm.exe", data: Some(leaked), size: data.len() as u64, sha256: Some(sha_static) };
        let out = e.decode().unwrap();
        assert_eq!(out, data);
        assert_eq!(payload_format::sha256_hex(&out), sha_static);
        check_file_type(Component::Dm, &out).unwrap();
    }

    #[test]
    fn corrupt_payloads_are_rejected() {
        let data = sample();
        let compressed = payload_format::encode(&data);
        let sha = payload_format::sha256_hex(&data);
        // wrong hash
        assert!(payload_format::decode(&compressed, data.len() as u64, &"0".repeat(64)).is_err());
        // wrong size
        assert!(payload_format::decode(&compressed, data.len() as u64 - 1, &sha).is_err());
        // damaged stream
        let mut bad = compressed.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0xff;
        assert!(payload_format::decode(&bad, data.len() as u64, &sha).is_err());
    }

    #[test]
    fn missing_payload_entry() {
        let src = payload_format::entry_source("CHROME", "zenless-chrome-extension.zip", None, 0, None);
        assert!(src.contains("data: None"));
        let e = Embedded { file: "x.zip", data: None, size: 0, sha256: None };
        assert!(!e.is_embedded());
        assert!(e.decode().is_err());
    }

    #[test]
    fn file_type_checks() {
        assert!(check_file_type(Component::Chrome, b"<html>not found</html>").is_err());
        let mut zip = b"PK\x03\x04".to_vec();
        zip.resize(100, 0);
        assert!(check_file_type(Component::Firefox, &zip).is_ok());
        assert!(check_file_type(Component::Torrent, &zip).is_err());
    }
}
