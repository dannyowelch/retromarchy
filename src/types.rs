use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type ConsoleId = String;
pub type ProfileId = String;
pub type GameId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Console {
    pub id: ConsoleId,
    pub name: String,
    pub rom_dirs: Vec<PathBuf>,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub profile: Option<ProfileId>,
    /// Which cached artwork the game grid prefers for this console.
    /// Serialized before `media` so it stays a key on the console table.
    #[serde(default)]
    pub grid_art: GridArt,
    pub media: MediaToggles,
}

#[derive(Debug, Clone)]
pub struct Game {
    pub id: GameId,
    pub console: ConsoleId,
    pub rom: PathBuf,
    pub title: String,
    pub crc32: Option<u32>,
    pub profile: Option<ProfileId>,
    pub media: Vec<Media>,
    pub last_played: Option<DateTime<Utc>>,
    pub play_count: u32,
    pub play_time: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    BoxArt,
    Screenshot,
    Manual,
    Video,
    TitleScreen,
}

/// Artwork the grid shows for a console. Stored on the console, not the scraper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridArt {
    BoxArt,
    TitleScreen,
    Screenshot,
}

impl Default for GridArt {
    fn default() -> Self {
        Self::BoxArt
    }
}

impl GridArt {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BoxArt => "box_art",
            Self::TitleScreen => "title_screen",
            Self::Screenshot => "screenshot",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BoxArt => "Box art",
            Self::TitleScreen => "Title screen",
            Self::Screenshot => "Screenshot",
        }
    }

    /// Preferred kind, then the other two v1 kinds.
    pub fn fallback(self) -> [MediaKind; 3] {
        match self {
            Self::BoxArt => [MediaKind::BoxArt, MediaKind::TitleScreen, MediaKind::Screenshot],
            Self::TitleScreen => [MediaKind::TitleScreen, MediaKind::BoxArt, MediaKind::Screenshot],
            Self::Screenshot => [MediaKind::Screenshot, MediaKind::BoxArt, MediaKind::TitleScreen],
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "box_art" => Some(Self::BoxArt),
            "title_screen" => Some(Self::TitleScreen),
            "screenshot" => Some(Self::Screenshot),
            _ => None,
        }
    }
}

/// Global artwork download settings. Separate from per-console local [`MediaToggles`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScraperConfig {
    #[serde(default = "default_true")]
    pub box_art: bool,
    #[serde(default = "default_true")]
    pub title_screen: bool,
    #[serde(default = "default_true")]
    pub screenshot: bool,
    #[serde(default = "default_providers")]
    pub providers: Vec<ProviderEntry>,
    #[serde(default)]
    pub credentials: ScraperCredentials,
}

fn default_true() -> bool {
    true
}

fn default_providers() -> Vec<ProviderEntry> {
    vec![
        ProviderEntry {
            id: ScrapeProvider::ScreenScraper,
            enabled: true,
        },
        ProviderEntry {
            id: ScrapeProvider::TheGamesDb,
            enabled: true,
        },
    ]
}

impl Default for ScraperConfig {
    fn default() -> Self {
        Self {
            box_art: true,
            title_screen: true,
            screenshot: true,
            providers: default_providers(),
            credentials: ScraperCredentials::default(),
        }
    }
}

impl ScraperConfig {
    pub fn enabled_kinds(&self) -> Vec<MediaKind> {
        let mut kinds = Vec::new();
        if self.box_art {
            kinds.push(MediaKind::BoxArt);
        }
        if self.title_screen {
            kinds.push(MediaKind::TitleScreen);
        }
        if self.screenshot {
            kinds.push(MediaKind::Screenshot);
        }
        kinds
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScrapeProvider {
    #[serde(rename = "screenscraper")]
    ScreenScraper,
    #[serde(rename = "thegamesdb")]
    TheGamesDb,
}

impl ScrapeProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::ScreenScraper => "ScreenScraper",
            Self::TheGamesDb => "TheGamesDB",
        }
    }

    pub fn source(self) -> Source {
        match self {
            Self::ScreenScraper => Source::ScreenScraper,
            Self::TheGamesDb => Source::TheGamesDb,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub id: ScrapeProvider,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScraperCredentials {
    #[serde(default)]
    pub screenscraper_user: String,
    #[serde(default)]
    pub screenscraper_password: String,
    #[serde(default)]
    pub thegamesdb_api_key: String,
}

impl Default for ScraperCredentials {
    fn default() -> Self {
        Self {
            screenscraper_user: String::new(),
            screenscraper_password: String::new(),
            thegamesdb_api_key: String::new(),
        }
    }
}

impl ScraperCredentials {
    pub fn block_reason(&self, provider: ScrapeProvider) -> Option<&'static str> {
        match provider {
            ScrapeProvider::ScreenScraper => {
                let missing = self.screenscraper_user.trim().is_empty()
                    || self.screenscraper_password.is_empty();
                missing.then_some(
                    "ScreenScraper needs a username and password in Scraper settings.",
                )
            }
            ScrapeProvider::TheGamesDb => self.thegamesdb_api_key.trim().is_empty().then_some(
                "TheGamesDB needs an API key in Scraper settings.",
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaToggles {
    pub box_art: bool,
    pub screenshot: bool,
    pub manual: bool,
    pub video: bool,
}

impl Default for MediaToggles {
    fn default() -> Self {
        Self {
            box_art: true,
            screenshot: true,
            manual: true,
            video: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Media {
    pub kind: MediaKind,
    pub path: PathBuf,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    ScreenScraper,
    TheGamesDb,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EmulatorProfile {
    RetroArch {
        id: ProfileId,
        core: PathBuf,
        config: Option<PathBuf>,
    },
    Standalone {
        id: ProfileId,
        command: String,
    },
}

impl EmulatorProfile {
    pub fn id(&self) -> &ProfileId {
        match self {
            EmulatorProfile::RetroArch { id, .. } => id,
            EmulatorProfile::Standalone { id, .. } => id,
        }
    }
}
