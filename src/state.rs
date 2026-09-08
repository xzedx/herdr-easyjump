//! Tiny bits of durable state: the previous pane for jump-back, tracing.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn state_dir() -> PathBuf {
    let dir = std::env::var("HERDR_PLUGIN_STATE_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".local/state/herdr/plugins/zed.hop")
        });
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn remember_previous(pane_id: Option<&str>) {
    if let Some(id) = pane_id {
        let body = serde_json::json!({ "pane_id": id }).to_string();
        let _ = std::fs::write(state_dir().join("last.json"), body);
    }
}

pub fn read_previous() -> Option<String> {
    let raw = std::fs::read_to_string(state_dir().join("last.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("pane_id").and_then(|p| p.as_str()).map(String::from)
}

/// HOP_TRACE=<file> appends "<label> <epoch seconds>" lines.
pub fn trace(label: &str) {
    if let Ok(path) = std::env::var("HOP_TRACE") {
        if path.is_empty() {
            return;
        }
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(f, "{label} {t:.4}");
        }
    }
}
