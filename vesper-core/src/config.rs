//! XDG path helpers via the `directories` crate. Inside Flatpak these resolve
//! through XDG automatically.

use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("io.github", "cmerk2021", "vesper")
}

/// `~/.config/vesper/`
pub fn config_dir() -> PathBuf {
    project_dirs()
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".config/vesper"))
}

/// `~/.config/vesper/themes/`
pub fn themes_dir() -> PathBuf {
    config_dir().join("themes")
}

/// `~/.local/share/vesper/`
pub fn data_dir() -> PathBuf {
    project_dirs()
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".local/share/vesper"))
}

/// Directory for persistent webview (login) data.
pub fn webview_data_dir() -> PathBuf {
    data_dir().join("webkit")
}

/// Persisted app configuration (`~/.config/vesper/config.toml`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AppConfig {
    /// Name of the active theme (built-in or user theme directory name).
    pub theme: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            theme: "vesper-dark".to_string(),
        }
    }
}

impl AppConfig {
    pub fn load() -> AppConfig {
        let path = config_dir().join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("invalid config.toml, using defaults: {e}");
                AppConfig::default()
            }),
            Err(_) => AppConfig::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = toml::to_string_pretty(self).unwrap_or_default();
        std::fs::write(dir.join("config.toml"), text)
    }
}
