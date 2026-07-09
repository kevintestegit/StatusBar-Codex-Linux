use std::env;
use std::fs;
use std::path::PathBuf;

pub fn codex_home() -> PathBuf {
    if let Some(path) = env::var_os("CODEX_HOME") {
        return PathBuf::from(path);
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".codex");
    }
    PathBuf::from(".codex")
}

pub fn details_html_path() -> PathBuf {
    let user = env::var("USER").unwrap_or_else(|_| "user".into());
    let dir = env::temp_dir().join(format!("codex-usage-tray-{user}"));
    let _ = fs::create_dir_all(&dir);
    dir.join("details.html")
}

pub fn config_path() -> PathBuf {
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME")
        && !config_home.is_empty()
    {
        return PathBuf::from(config_home).join("codex-usage-tray/config.json");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".config/codex-usage-tray/config.json");
    }
    PathBuf::from("codex-usage-tray-config.json")
}
