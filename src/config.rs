use serde_json::Value;
use std::fs;

use crate::paths::config_path;

pub const DEFAULT_REFRESH_SECONDS: u32 = 30;
pub const MIN_REFRESH_SECONDS: u32 = 5;
const MAX_REFRESH_SECONDS: u32 = 3600;

#[derive(Clone, Copy)]
pub struct AppConfig {
    pub party_mode: bool,
    pub show_mascot: bool,
    pub refresh_seconds: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            party_mode: false,
            show_mascot: true,
            refresh_seconds: DEFAULT_REFRESH_SECONDS,
        }
    }
}

pub fn load_config() -> AppConfig {
    let Ok(content) = fs::read_to_string(config_path()) else {
        return AppConfig::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return AppConfig::default();
    };
    let default = AppConfig::default();
    AppConfig {
        party_mode: value
            .get("party_mode")
            .and_then(Value::as_bool)
            .unwrap_or(default.party_mode),
        show_mascot: value
            .get("show_mascot")
            .and_then(Value::as_bool)
            .unwrap_or(default.show_mascot),
        refresh_seconds: value
            .get("refresh_seconds")
            .and_then(Value::as_u64)
            .map(|value| clamp_refresh_seconds(value as u32))
            .unwrap_or(default.refresh_seconds),
    }
}

pub fn save_config(config: AppConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let body = format!(
        "{{\n  \"party_mode\": {},\n  \"show_mascot\": {},\n  \"refresh_seconds\": {}\n}}\n",
        if config.party_mode { "true" } else { "false" },
        if config.show_mascot { "true" } else { "false" },
        config.refresh_seconds
    );
    let _ = fs::write(path, body);
}

pub fn party_mode_enabled() -> bool {
    load_config().party_mode
}

pub fn set_party_mode(enabled: bool) {
    let mut config = load_config();
    config.party_mode = enabled;
    save_config(config);
}

pub fn show_mascot() -> bool {
    load_config().show_mascot
}

pub fn set_show_mascot(enabled: bool) {
    let mut config = load_config();
    config.show_mascot = enabled;
    save_config(config);
}

pub fn refresh_seconds() -> u32 {
    load_config().refresh_seconds
}

pub fn set_refresh_seconds(seconds: u32) {
    let mut config = load_config();
    config.refresh_seconds = clamp_refresh_seconds(seconds);
    save_config(config);
}

pub fn refresh_interval_markup() -> String {
    format!(
        "⏱  Refresh interval:  <b>{}</b>",
        duration_label(refresh_seconds())
    )
}

pub fn party_mode_markup() -> String {
    if party_mode_enabled() {
        "🎉  Party mode:  <b>On</b>".into()
    } else {
        "🎉  Party mode:  <b>Off</b>".into()
    }
}

pub fn mascot_markup() -> String {
    if show_mascot() {
        "🦦  Mascot:  <b>On</b>".into()
    } else {
        "🦦  Mascot:  <b>Off</b>".into()
    }
}

pub fn duration_label(seconds: u32) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}m {}s", seconds / 60, seconds % 60)
    }
}

fn clamp_refresh_seconds(seconds: u32) -> u32 {
    seconds.clamp(MIN_REFRESH_SECONDS, MAX_REFRESH_SECONDS)
}
