//! Minimal client for the Herdr socket API: one JSON request per line,
//! one JSON response per line, over a Unix socket.
//!
//! The server closes the connection after each plain request, so every
//! call opens a fresh socket. That costs well under a millisecond.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

pub fn socket_path() -> String {
    if let Ok(p) = std::env::var("HERDR_SOCKET_PATH") {
        if !p.is_empty() {
            return p;
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/.config/herdr/herdr.sock")
}

pub struct Client {
    path: String,
}

impl Client {
    /// Resolve the socket path and check the server answers at all.
    pub fn connect() -> Result<Self, String> {
        let path = socket_path();
        UnixStream::connect(&path).map_err(|e| format!("connect {path}: {e}"))?;
        Ok(Client { path })
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let stream = UnixStream::connect(&self.path).map_err(|e| format!("connect: {e}"))?;
        let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        let req = json!({"id": "easyjump", "method": method, "params": params});
        let mut line = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        line.push('\n');
        writer
            .write_all(line.as_bytes())
            .map_err(|e| format!("{method}: write: {e}"))?;
        let mut resp = String::new();
        let n = reader
            .read_line(&mut resp)
            .map_err(|e| format!("{method}: read: {e}"))?;
        if n == 0 {
            return Err(format!("{method}: connection closed"));
        }
        let v: Value = serde_json::from_str(&resp).map_err(|e| format!("{method}: {e}"))?;
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("error");
            return Err(format!("{method}: {msg}"));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn snapshot(&mut self) -> Result<Value, String> {
        let r = self.call("session.snapshot", json!({}))?;
        r.get("snapshot")
            .cloned()
            .ok_or_else(|| "snapshot missing".to_string())
    }
}
