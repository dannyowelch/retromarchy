use crate::types::{Console, EmulatorProfile, ScraperConfig};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const INPUT_MS_MAX: u32 = 60_000;

fn default_initial_delay_ms() -> u32 {
    400
}

fn default_slow_interval_ms() -> u32 {
    180
}

fn default_fast_interval_ms() -> u32 {
    50
}

fn default_ramp_ms() -> u32 {
    2000
}

/// Shared game-grid cover width. Screenshot and box-art cards both use it.
pub const COVER_WIDTH_DEFAULT: f32 = 216.0;
/// Narrowest cover that still leaves a readable caption.
pub const COVER_WIDTH_MIN: f32 = 120.0;
/// Widest cover. `columns_for` still keeps at least one column.
pub const COVER_WIDTH_MAX: f32 = 400.0;
/// Slider detents and `-` / `+` both move by this many pixels.
pub const COVER_WIDTH_STEP: f32 = 10.0;

fn default_cover_width() -> f32 {
    COVER_WIDTH_DEFAULT
}

/// Keep a cover width inside the slider range. Non-finite values become the default.
pub fn clamp_cover_width(width: f32) -> f32 {
    if !width.is_finite() {
        return COVER_WIDTH_DEFAULT;
    }
    width.round().clamp(COVER_WIDTH_MIN, COVER_WIDTH_MAX)
}

/// Move `steps` detents of [`COVER_WIDTH_STEP`] from the current width, then clamp.
pub fn step_cover_width(width: f32, steps: i32) -> f32 {
    let current = clamp_cover_width(width);
    if steps == 0 {
        return current;
    }
    clamp_cover_width(current + steps as f32 * COVER_WIDTH_STEP)
}

/// How long a direction repeats while it is held. Serialized as `[input]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSettings {
    /// ms held before the first repeat (after the initial press step)
    #[serde(default = "default_initial_delay_ms")]
    pub initial_delay_ms: u32,
    /// ms between steps during the slow phase
    #[serde(default = "default_slow_interval_ms")]
    pub slow_interval_ms: u32,
    /// ms between steps after ramp completes
    #[serde(default = "default_fast_interval_ms")]
    pub fast_interval_ms: u32,
    /// ms from first repeat until fast rate is reached (interpolate interval)
    #[serde(default = "default_ramp_ms")]
    pub ramp_ms: u32,
}

impl Default for InputSettings {
    fn default() -> Self {
        Self {
            initial_delay_ms: default_initial_delay_ms(),
            slow_interval_ms: default_slow_interval_ms(),
            fast_interval_ms: default_fast_interval_ms(),
            ramp_ms: default_ramp_ms(),
        }
    }
}

impl InputSettings {
    /// Copy with intervals that cannot stall or spin navigation.
    pub fn sanitized(self) -> Self {
        let mut settings = self;
        settings.sanitize();
        settings
    }

    /// Repeat intervals are at least 1 ms, the fast interval is not longer than the slow one, and every field is capped.
    pub fn sanitize(&mut self) {
        self.initial_delay_ms = self.initial_delay_ms.min(INPUT_MS_MAX);
        self.ramp_ms = self.ramp_ms.min(INPUT_MS_MAX);
        if self.slow_interval_ms == 0 {
            self.slow_interval_ms = default_slow_interval_ms();
        }
        if self.fast_interval_ms == 0 {
            self.fast_interval_ms = default_fast_interval_ms();
        }
        self.slow_interval_ms = self.slow_interval_ms.clamp(1, INPUT_MS_MAX);
        self.fast_interval_ms = self.fast_interval_ms.clamp(1, INPUT_MS_MAX);
        if self.fast_interval_ms > self.slow_interval_ms {
            self.fast_interval_ms = self.slow_interval_ms;
        }
    }
}

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
    /// One cover width for every system's game grid, in pixels.
    #[serde(default = "default_cover_width")]
    pub cover_width: f32,
    #[serde(default)]
    pub profiles: Vec<EmulatorProfile>,
    #[serde(default)]
    pub consoles: Vec<Console>,
    #[serde(default)]
    pub scraper: ScraperConfig,
    #[serde(default)]
    pub input: InputSettings,
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
            cover_width: default_cover_width(),
            profiles: Vec::new(),
            consoles: Vec::new(),
            scraper: ScraperConfig::default(),
            input: InputSettings::default(),
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
    config_from_toml(&content)
        .with_context(|| format!("Failed to parse config from {}", path.display()))
}

/// Parse config text. Clamps `[input]` and `cover_width`.
pub fn config_from_toml(content: &str) -> Result<Config> {
    let mut config: Config = toml::from_str(content)?;
    config.input.sanitize();
    config.cover_width = clamp_cover_width(config.cover_width);
    Ok(config)
}

/// Load config, store one global cover width, and write it back.
pub fn save_cover_width(width: f32) -> Result<()> {
    let mut config = load_config()?;
    config.cover_width = clamp_cover_width(width);
    save_config(&config)
}

/// Load config, store hold-repeat timings, and write the file back.
pub fn save_input_settings(input: InputSettings) -> Result<()> {
    let mut config = load_config()?;
    config.input = input.sanitized();
    save_config(&config)
}

/// Load config, replace `[scraper]`, and write the file back.
/// Theme, input, consoles, and profiles stay as they were.
pub fn save_scraper_settings(scraper: ScraperConfig) -> Result<()> {
    let mut config = load_config()?;
    config.scraper = scraper;
    save_config(&config)
}

/// Load config, store `theme`, and write the file back.
pub fn save_theme_name(theme: &str) -> Result<()> {
    let mut config = load_config()?;
    config.theme = theme.to_string();
    save_config(&config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Console, GridArt, MediaToggles};
    use std::path::Path;
    use std::sync::MutexGuard;

    struct EnvLock {
        key: &'static str,
        prev: Option<String>,
        _guard: MutexGuard<'static, ()>,
    }

    impl EnvLock {
        fn set(key: &'static str, value: &Path) -> Self {
            let guard = super::XDG_LOCK
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self {
                key,
                prev,
                _guard: guard,
            }
        }
    }

    impl Drop for EnvLock {
        fn drop(&mut self) {
            match self.prev.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

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

    #[test]
    fn missing_input_section_loads_defaults() {
        let config = config_from_toml("theme = \"system\"\n").unwrap();
        assert_eq!(config.input, InputSettings::default());
        assert_eq!(config.input.initial_delay_ms, 400);
        assert_eq!(config.input.slow_interval_ms, 180);
        assert_eq!(config.input.fast_interval_ms, 50);
        assert_eq!(config.input.ramp_ms, 2000);
        assert_eq!(config.cover_width, COVER_WIDTH_DEFAULT);
    }

    #[test]
    fn cover_width_round_trips_and_clamps() {
        let mut config = Config::default();
        config.cover_width = 250.0;
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(
            text.contains("cover_width = 250") || text.contains("cover_width = 250.0"),
            "{text}"
        );
        let back = config_from_toml(&text).unwrap();
        assert_eq!(back.cover_width, 250.0);

        assert_eq!(
            config_from_toml("cover_width = 10\n").unwrap().cover_width,
            COVER_WIDTH_MIN
        );
        assert_eq!(
            config_from_toml("cover_width = 9999\n")
                .unwrap()
                .cover_width,
            COVER_WIDTH_MAX
        );
        assert_eq!(clamp_cover_width(f32::NAN), COVER_WIDTH_DEFAULT);
        assert_eq!(step_cover_width(COVER_WIDTH_DEFAULT, 1), 226.0);
        assert_eq!(step_cover_width(COVER_WIDTH_DEFAULT, -1), 206.0);
        assert_eq!(step_cover_width(COVER_WIDTH_MAX, 1), COVER_WIDTH_MAX);
        assert_eq!(step_cover_width(COVER_WIDTH_MIN, -1), COVER_WIDTH_MIN);
        assert_eq!(step_cover_width(217.4, 1), 227.0);
    }

    #[test]
    fn save_cover_width_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = EnvLock::set("XDG_CONFIG_HOME", dir.path());
        let mut config = Config::default();
        config.theme = "custom".into();
        config.details_visible = false;
        config.consoles.push(Console {
            id: "nes".into(),
            name: "NES".into(),
            rom_dirs: Vec::new(),
            extensions: vec!["nes".into()],
            profile: None,
            grid_art: GridArt::Screenshot,
            media: MediaToggles::default(),
        });
        save_config(&config).unwrap();
        save_cover_width(246.0).unwrap();
        let loaded = load_config().unwrap();
        assert_eq!(loaded.cover_width, 246.0);
        assert_eq!(loaded.theme, "custom");
        assert!(!loaded.details_visible);
        assert_eq!(loaded.consoles.len(), 1);
        assert_eq!(loaded.consoles[0].id, "nes");
        assert_eq!(loaded.consoles[0].grid_art, GridArt::Screenshot);
        // A second launch reads the same global width.
        assert_eq!(load_config().unwrap().cover_width, 246.0);
    }

    #[test]
    fn save_input_and_theme_keep_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = EnvLock::set("XDG_CONFIG_HOME", dir.path());
        let mut config = Config::default();
        config.theme = "system".into();
        config.details_visible = false;
        config.cover_width = 250.0;
        config.consoles.push(Console {
            id: "nes".into(),
            name: "NES".into(),
            rom_dirs: Vec::new(),
            extensions: vec!["nes".into()],
            profile: None,
            grid_art: GridArt::Screenshot,
            media: MediaToggles::default(),
        });
        save_config(&config).unwrap();

        save_input_settings(InputSettings {
            initial_delay_ms: 250,
            slow_interval_ms: 120,
            fast_interval_ms: 40,
            ramp_ms: 1500,
        })
        .unwrap();
        let loaded = load_config().unwrap();
        assert_eq!(loaded.input.initial_delay_ms, 250);
        assert_eq!(loaded.input.slow_interval_ms, 120);
        assert_eq!(loaded.input.fast_interval_ms, 40);
        assert_eq!(loaded.input.ramp_ms, 1500);
        assert_eq!(loaded.theme, "system");
        assert!(!loaded.details_visible);
        assert_eq!(loaded.cover_width, 250.0);
        assert_eq!(loaded.consoles[0].id, "nes");

        save_theme_name("launchbox").unwrap();
        let loaded = load_config().unwrap();
        assert_eq!(loaded.theme, "launchbox");
        assert_eq!(loaded.input.initial_delay_ms, 250);
        assert_eq!(loaded.cover_width, 250.0);
        assert_eq!(loaded.consoles[0].grid_art, GridArt::Screenshot);
    }

    #[test]
    fn input_settings_round_trip() {
        let mut config = Config::default();
        config.input.initial_delay_ms = 250;
        config.input.slow_interval_ms = 120;
        config.input.fast_interval_ms = 40;
        config.input.ramp_ms = 1500;
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(text.contains("[input]"), "{text}");
        assert!(text.contains("initial_delay_ms = 250"), "{text}");
        assert!(text.contains("slow_interval_ms = 120"), "{text}");
        assert!(text.contains("fast_interval_ms = 40"), "{text}");
        assert!(text.contains("ramp_ms = 1500"), "{text}");
        let back = config_from_toml(&text).unwrap();
        assert_eq!(back.input, config.input);
    }

    #[test]
    fn partial_input_table_fills_the_other_fields() {
        let config = config_from_toml("[input]\ninitial_delay_ms = 100\n").unwrap();
        assert_eq!(config.input.initial_delay_ms, 100);
        assert_eq!(config.input.slow_interval_ms, 180);
        assert_eq!(config.input.fast_interval_ms, 50);
        assert_eq!(config.input.ramp_ms, 2000);
    }

    #[test]
    fn illegal_input_values_cannot_stall_repeat() {
        let raw = r#"
[input]
initial_delay_ms = 999999
slow_interval_ms = 0
fast_interval_ms = 400
ramp_ms = 800000
"#;
        let config = config_from_toml(raw).unwrap();
        assert_eq!(config.input.initial_delay_ms, 60_000);
        assert_eq!(config.input.ramp_ms, 60_000);
        assert!(config.input.slow_interval_ms >= 1);
        assert!(config.input.fast_interval_ms >= 1);
        assert!(config.input.fast_interval_ms <= config.input.slow_interval_ms);

        let swapped = InputSettings {
            initial_delay_ms: 10,
            slow_interval_ms: 40,
            fast_interval_ms: 90,
            ramp_ms: 0,
        }
        .sanitized();
        assert_eq!(swapped.fast_interval_ms, 40);
        assert_eq!(swapped.slow_interval_ms, 40);
    }
}

#[cfg(test)]
pub(crate) static XDG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Isolated HOME and XDG dirs for tests that read or write the real config paths.
#[cfg(test)]
pub(crate) struct XdgEnv {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl XdgEnv {
    pub(crate) fn sandbox(root: &std::path::Path) -> Self {
        let home = root.join("home");
        let config = root.join("config");
        let data = root.join("data");
        let cache = root.join("cache");
        for dir in [&home, &config, &data, &cache] {
            fs::create_dir_all(dir).unwrap();
        }
        Self::set(&[
            ("HOME", &home),
            ("XDG_CONFIG_HOME", &config),
            ("XDG_DATA_HOME", &data),
            ("XDG_CACHE_HOME", &cache),
        ])
    }

    fn set(pairs: &[(&'static str, &std::path::Path)]) -> Self {
        let guard = XDG_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let saved = pairs
            .iter()
            .map(|(key, value)| {
                let prev = std::env::var_os(key);
                std::env::set_var(key, value);
                (*key, prev)
            })
            .collect();
        Self {
            saved,
            _guard: guard,
        }
    }
}

#[cfg(test)]
impl Drop for XdgEnv {
    fn drop(&mut self) {
        for (key, prev) in self.saved.drain(..) {
            match prev {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

pub fn save_config(config: &Config) -> Result<()> {
    let path = config_path()?;
    let content = toml::to_string_pretty(config)?;
    fs::write(&path, content)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;
    Ok(())
}
