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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
