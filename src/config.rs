use crate::types::{Console, EmulatorProfile, ScraperConfig};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleMetadata {
    pub id: String,
    pub manufacturer: String,
    pub year: u32,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_true")]
    pub details_visible: bool,
    #[serde(default)]
    pub profiles: Vec<EmulatorProfile>,
    #[serde(default)]
    pub consoles: Vec<Console>,
    #[serde(default)]
    pub scraper: ScraperConfig,
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            details_visible: default_true(),
            profiles: Vec::new(),
            consoles: Vec::new(),
            scraper: ScraperConfig::default(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConsoleMetadataFile {
    console: Vec<ConsoleMetadata>,
}

pub fn load_console_metadata() -> Result<Vec<ConsoleMetadata>> {
    let metadata_content = include_str!("../console_metadata.toml");
    let file: ConsoleMetadataFile = toml::from_str(metadata_content)?;
    Ok(file.console)
}

pub fn config_path() -> Result<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::with_prefix("retromarchy")?;
    Ok(xdg_dirs.place_config_file("config.toml")?)
}

pub fn load_config() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        let config = Config::default();
        save_config(&config)?;
        return Ok(config);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config from {}", path.display()))?;
    let config: Config = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config from {}", path.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Console, GridArt, MediaToggles};

    #[test]
    fn grid_art_roundtrip_saves() {
        let config = Config {
            consoles: vec![Console {
                id: "snes".into(),
                name: "Super Nintendo".into(),
                rom_dirs: vec![],
                extensions: vec!["sfc".into()],
                profile: None,
                grid_art: GridArt::Screenshot,
                media: MediaToggles::default(),
            }],
            ..Config::default()
        };
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(
            text.contains("grid_art = 'screenshot'") || text.contains("grid_art = \"screenshot\""),
            "{text}"
        );
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.consoles[0].grid_art, GridArt::Screenshot);
        assert_eq!(back.scraper.providers.len(), 2);
    }

    #[test]
    fn old_softname_fields_are_ignored_and_not_written() {
        let raw = r#"
theme = "system"
[scraper.credentials]
screenscraper_user = "member"
screenscraper_password = "secret"
screenscraper_dev_id = "leftover"
screenscraper_dev_password = "leftover-pass"
thegamesdb_api_key = "key"
"#;
        let config: Config = toml::from_str(raw).unwrap();
        assert_eq!(config.scraper.credentials.screenscraper_user, "member");
        assert!(config
            .scraper
            .credentials
            .block_reason(crate::types::ScrapeProvider::ScreenScraper)
            .is_none());
        let text = toml::to_string(&config).unwrap();
        assert!(!text.contains("screenscraper_dev_id"));
        assert!(!text.contains("screenscraper_dev_password"));
        assert!(!text.contains("leftover"));
    }

    #[test]
    fn title_screen_grid_art_loads_as_box_art() {
        let raw = r#"
[[consoles]]
id = "nes"
name = "NES"
rom_dirs = []
extensions = ["nes"]
grid_art = "title_screen"

[consoles.media]
box_art = true
screenshot = true
manual = false
video = false

[scraper]
box_art = true
title_screen = true
screenshot = false
"#;
        let config: Config = toml::from_str(raw).unwrap();
        assert_eq!(config.consoles[0].grid_art, GridArt::BoxArt);
        assert!(config.scraper.box_art);
        assert!(!config.scraper.screenshot);
        let text = toml::to_string(&config).unwrap();
        assert!(!text.contains("title_screen"));
    }
}

pub fn save_config(config: &Config) -> Result<()> {
    let path = config_path()?;
    let content = toml::to_string_pretty(config)?;
    fs::write(&path, content)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;
    Ok(())
}

