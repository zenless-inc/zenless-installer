//! Asking running Zenless apps to exit before their files are replaced,
//! through the apps' local API (`POST http://127.0.0.1:<port>/quit`).

use crate::components::Component;
use crate::platform::{self, Env};
use crate::report::Reporter;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

/// Value of the `X-Zenless-Client` header sent to the apps.
pub const CLIENT: &str = concat!("zenless-installer/", env!("CARGO_PKG_VERSION"));

/// How long to wait for an app to exit after asking it to quit.
const QUIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Builds a minimal HTTP/1.1 request. No `Origin` header: `/quit` is only
/// accepted from local, non-browser callers.
pub fn build_request(method: &str, path: &str, port: u16) -> String {
    let body = if method == "POST" { "{}" } else { "" };
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Zenless-Client: {CLIENT}\r\nConnection: close\r\n"
    );
    if method == "POST" {
        req += &format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len());
    }
    req += "\r\n";
    req += body;
    req
}

/// Sends a request and returns the HTTP status code, if anything answered.
fn http(port: u16, method: &str, path: &str) -> Option<u16> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
    let _ = s.set_write_timeout(Some(Duration::from_millis(1500)));
    s.write_all(build_request(method, path, port).as_bytes()).ok()?;
    let mut buf = [0u8; 512];
    let n = s.read(&mut buf).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    head.split_whitespace().nth(1)?.parse().ok()
}

/// `true` when the app answers `GET /ping`.
pub fn is_running(port: u16) -> bool {
    http(port, "GET", "/ping").is_some()
}

/// Asks the app owning `exe` to quit and waits (up to 5 s) until it is gone.
/// Returns `false` if it is apparently still running (its exe is then renamed
/// to `.old` by the caller instead of being overwritten).
pub fn quit_and_wait(env: &Env, c: Component, exe: &Path, r: &dyn Reporter) -> bool {
    let Some(port) = c.api_port() else { return true };
    if env.is_sandbox() {
        r.info(&format!("Sandbox mode: not asking {} to quit.", c.name()));
        return true;
    }
    let api_up = is_running(port);
    let locked = exe.exists() && platform::is_file_locked(exe);
    if !api_up && !locked {
        return true;
    }
    if api_up {
        r.info(&format!("{} is running; asking it to exit…", c.name()));
        match http(port, "POST", "/quit") {
            Some(code) if (200..300).contains(&code) => {}
            Some(code) => r.warn(&format!("{} answered /quit with HTTP {code}.", c.name())),
            None => r.warn(&format!("{} did not answer the quit request.", c.name())),
        }
    }
    let deadline = Instant::now() + QUIT_TIMEOUT;
    loop {
        let still = is_running(port) || (exe.exists() && platform::is_file_locked(exe));
        if !still {
            r.ok(&format!("{} has exited.", c.name()));
            return true;
        }
        if Instant::now() >= deadline {
            r.warn(&format!(
                "{} is still running; its program file will be swapped out and replaced on its next start.",
                c.name()
            ));
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_request_shape() {
        let req = build_request("POST", "/quit", 6812);
        assert!(req.starts_with("POST /quit HTTP/1.1\r\n"));
        assert!(req.contains("\r\nX-Zenless-Client: zenless-installer/0.1.0\r\n"));
        assert!(req.contains("Host: 127.0.0.1:6812"));
        assert!(!req.to_ascii_lowercase().contains("origin:"));
        assert!(req.ends_with("\r\n\r\n{}"));
        let ping = build_request("GET", "/ping", 6813);
        assert!(!ping.contains("Content-Length"));
    }
}
