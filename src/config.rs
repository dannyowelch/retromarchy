use crate::types::{Console, EmulatorProfile};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub consoles: Vec<Console>,
    #[serde(default)]
    pub profiles: Vec<EmulatorProfile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            consoles: Vec::new(),
            profiles: Vec::new(),
        }
    }
}

pub fn config_path() -> Result<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::with_prefix("retromarchy")?;
    Ok(xdg_dirs.place_config_file("config.toml")?)
}

pub fn load_config() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config from {}", path.display()))?;
    let config: Config = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config from {}", path.display()))?;
    Ok(config)
}

