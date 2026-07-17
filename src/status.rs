use serde_json::Value;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::{debug_log, RateLimits, WindowLimit};

const LIVE_FETCH_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusSource {
    LiveAppServer,
    LocalSessionJsonl,
    Unavailable,
}

impl StatusSource {
    pub fn label(&self) -> &'static str {
        match self {
            StatusSource::LiveAppServer => "codex app-server account/rateLimits/read",
            StatusSource::LocalSessionJsonl => "local ~/.codex/sessions token_count",
            StatusSource::Unavailable => "none",
        }
    }
}

#[derive(Clone, Default)]
pub struct CodexStatus {
    pub rate_limits: Option<RateLimits>,
    pub source: StatusSource,
    pub auth_status: String,
    pub fetched_at: Option<i64>,
    pub error: Option<String>,
}

impl Default for StatusSource {
    fn default() -> Self {
        StatusSource::Unavailable
    }
}

pub fn resolve_codex_status(jsonl_limits: Option<RateLimits>) -> CodexStatus {
    let auth_status = auth_status_label();
    match fetch_live_rate_limits() {
        Ok(rate_limits) => {
            debug_log(&format!(
                "status source=live plan={} 5h={:.0}% weekly={:.0}%",
                rate_limits.plan_type,
                rate_limits.primary.used_percent,
                rate_limits.secondary.used_percent
            ));
            CodexStatus {
                rate_limits: Some(rate_limits),
                source: StatusSource::LiveAppServer,
                auth_status,
                fetched_at: Some(chrono::Utc::now().timestamp()),
                error: None,
            }
        }
        Err(err) => {
            debug_log(&format!("live rate limits failed: {err}"));
            if let Some(rate_limits) = jsonl_limits.filter(rate_limits_still_valid) {
                debug_log(&format!(
                    "status source=jsonl plan={} 5h={:.0}% weekly={:.0}%",
                    rate_limits.plan_type,
                    rate_limits.primary.used_percent,
                    rate_limits.secondary.used_percent
                ));
                CodexStatus {
                    rate_limits: Some(rate_limits),
                    source: StatusSource::LocalSessionJsonl,
                    auth_status,
                    fetched_at: Some(chrono::Utc::now().timestamp()),
                    error: Some(err),
                }
            } else {
                debug_log("status source=unavailable (no valid live or local rate limits)");
                CodexStatus {
                    rate_limits: None,
                    source: StatusSource::Unavailable,
                    auth_status,
                    fetched_at: Some(chrono::Utc::now().timestamp()),
                    error: Some(err),
                }
            }
        }
    }
}

/// Local JSONL snapshot is only usable while the 5h window is still open.
/// Expired windows must not be shown as current usage (and never as fake 0%).
pub fn rate_limits_still_valid(rate: &RateLimits) -> bool {
    let now = chrono::Utc::now().timestamp();
    let open = |w: &WindowLimit| w.resets_at.map(|reset| reset > now).unwrap_or(false);
    open(&rate.primary) || open(&rate.secondary)
}

pub fn auth_status_label() -> String {
    // Existence only — never read auth.json contents (tokens/keys).
    let path = codex_auth_path();
    if path.is_file() {
        "ChatGPT/Codex auth configured".into()
    } else {
        "Auth not found (run: codex login)".into()
    }
}

fn codex_auth_path() -> PathBuf {
    if let Some(home) = env::var_os("CODEX_HOME") {
        return PathBuf::from(home).join("auth.json");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".codex/auth.json");
    }
    PathBuf::from("auth.json")
}

fn codex_bin() -> String {
    if let Ok(path) = env::var("CODEX_BIN") {
        if !path.is_empty() {
            return path;
        }
    }
    // Desktop launches often lack nvm/npm PATH. Probe common install locations.
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            candidates.push(dir.join("codex"));
        }
    }
    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".local/bin/codex"));
        candidates.push(home.join(".cargo/bin/codex"));
        // nvm default current node bin (best-effort; no secrets)
        if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            let mut versions: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path().join("bin/codex")))
                .collect();
            versions.sort();
            versions.reverse();
            candidates.extend(versions);
        }
    }
    candidates.push(PathBuf::from("/usr/local/bin/codex"));
    candidates.push(PathBuf::from("/usr/bin/codex"));
    for path in candidates {
        if path.is_file() {
            return path.to_string_lossy().into_owned();
        }
    }
    "codex".into()
}

fn fetch_live_rate_limits() -> Result<RateLimits, String> {
    let bin = codex_bin();
    debug_log(&format!("spawning `{bin} app-server --listen stdio://`"));
    let mut child = Command::new(&bin)
        .args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to start codex app-server: {e}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "app-server stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "app-server stdout unavailable".to_string())?;

    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("read app-server: {e}")));
                    break;
                }
            }
        }
    });

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"StatusBar-Codex-Linux","title":"Codex Usage Tray","version":"0.1.0"},"capabilities":{}}}"#;
    writeln!(stdin, "{init}").map_err(|e| format!("write initialize: {e}"))?;
    stdin
        .flush()
        .map_err(|e| format!("flush initialize: {e}"))?;

    let deadline = Instant::now() + LIVE_FETCH_TIMEOUT;
    let mut rate_limits: Option<RateLimits> = None;
    let mut init_ok = false;
    let mut requested_limits = false;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(Ok(line)) => {
                debug_log(&format!(
                    "app-server line: {}",
                    truncate_for_log(&line, 180)
                ));
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let id = msg
                    .get("id")
                    .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|i| i as u64)));
                if id == Some(1) {
                    if msg.get("error").is_some() {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("initialize error: {line}"));
                    }
                    init_ok = true;
                    debug_log("app-server initialize ok");
                    if !requested_limits {
                        let read = r#"{"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read","params":null}"#;
                        writeln!(stdin, "{read}")
                            .map_err(|e| format!("write rateLimits/read: {e}"))?;
                        stdin
                            .flush()
                            .map_err(|e| format!("flush rateLimits/read: {e}"))?;
                        requested_limits = true;
                        debug_log("requested account/rateLimits/read");
                    }
                }
                if id == Some(2) {
                    if let Some(err) = msg.get("error") {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("rateLimits/read error: {err}"));
                    }
                    let result = msg
                        .get("result")
                        .ok_or_else(|| "rateLimits/read missing result".to_string())?;
                    let snapshot = result
                        .get("rateLimits")
                        .ok_or_else(|| "rateLimits/read missing rateLimits".to_string())?;
                    rate_limits = Some(parse_rate_limit_snapshot(snapshot)?);
                    break;
                }
            }
            Ok(Err(e)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    if let Some(rate_limits) = rate_limits {
        return Ok(rate_limits);
    }
    if !init_ok {
        return Err("timed out waiting for app-server initialize".into());
    }
    if !requested_limits {
        return Err("initialize ok but rateLimits/read was not sent".into());
    }
    Err("timed out waiting for account/rateLimits/read".into())
}

fn truncate_for_log(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

fn parse_rate_limit_snapshot(raw: &Value) -> Result<RateLimits, String> {
    Ok(RateLimits {
        plan_type: raw
            .get("planType")
            .or_else(|| raw.get("plan_type"))
            .and_then(Value::as_str)
            .unwrap_or("n/a")
            .to_string(),
        primary: parse_window_fields(raw.get("primary")),
        secondary: parse_window_fields(raw.get("secondary")),
    })
}

fn parse_window_fields(value: Option<&Value>) -> WindowLimit {
    let Some(value) = value else {
        return WindowLimit::default();
    };
    WindowLimit {
        used_percent: number_field(value, "used_percent", "usedPercent").unwrap_or(0.0),
        resets_at: int_field(value, "resets_at", "resetsAt"),
        window_duration_mins: int_field(value, "window_duration_mins", "windowDurationMins"),
    }
}

fn number_field(value: &Value, snake: &str, camel: &str) -> Option<f64> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
}

fn int_field(value: &Value, snake: &str, camel: &str) -> Option<i64> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RateLimits, WindowLimit};

    #[test]
    fn expired_primary_is_not_valid() {
        let now = chrono::Utc::now().timestamp();
        let rate = RateLimits {
            plan_type: "plus".into(),
            primary: WindowLimit {
                used_percent: 25.0,
                resets_at: Some(now - 60),
                ..Default::default()
            },
            secondary: WindowLimit {
                used_percent: 40.0,
                resets_at: Some(now + 3600),
                ..Default::default()
            },
        };
        assert!(!rate_limits_still_valid(&rate));
    }

    #[test]
    fn open_primary_is_valid() {
        let now = chrono::Utc::now().timestamp();
        let rate = RateLimits {
            plan_type: "plus".into(),
            primary: WindowLimit {
                used_percent: 1.0,
                resets_at: Some(now + 3600),
                ..Default::default()
            },
            secondary: WindowLimit {
                used_percent: 67.0,
                resets_at: Some(now + 86400),
                ..Default::default()
            },
        };
        assert!(rate_limits_still_valid(&rate));
    }

    #[test]
    fn parses_camel_case_app_server_snapshot() {
        let raw = serde_json::json!({
            "planType": "plus",
            "primary": {"usedPercent": 1, "windowDurationMins": 300, "resetsAt": 1783614415},
            "secondary": {"usedPercent": 67, "windowDurationMins": 10080, "resetsAt": 1783694508}
        });
        let rate = parse_rate_limit_snapshot(&raw).unwrap();
        assert_eq!(rate.plan_type, "plus");
        assert_eq!(rate.primary.used_percent, 1.0);
        assert_eq!(rate.secondary.used_percent, 67.0);
        assert_eq!(rate.primary.resets_at, Some(1783614415));
    }
}
