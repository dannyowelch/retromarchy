use crate::config::{self, ConsoleMetadata};
use crate::database::{self, LibraryStats};
use crate::gamepad::{grid_step, list_step, NavDir};
use crate::types::{Console, EmulatorProfile, Game, GridArt, Media, MediaKind, Source};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryKind {
    Disk,
    Demo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Sidebar,
    Grid,
}

#[derive(Debug, Clone)]
pub struct Shelf {
    pub console: Console,
    pub manufacturer: Option<String>,
    pub year: Option<u32>,
    pub description: Option<String>,
    pub stats: LibraryStats,
    pub games: Vec<Game>,
}

#[derive(Debug, Clone)]
pub struct Library {
    pub kind: LibraryKind,
    pub note: String,
    pub details_open: bool,
    pub shelves: Vec<Shelf>,
    pub profiles: Vec<EmulatorProfile>,
}

#[derive(Debug, Clone)]
pub struct Browse {
    pub library: Library,
    pub pane: Pane,
    pub console: usize,
    pub game: Option<usize>,
    pub details_open: bool,
    pub columns: usize,
    /// One cover width for every system, in pixels.
    pub cover_width: f32,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Arrow(NavDir),
    Grid(NavDir),
    EnterGrid,
    Clear,
    ToggleDetails,
    Launch,
    CoverSmaller,
    CoverLarger,
}

pub fn open_library() -> Library {
    match config::load_config() {
        Ok(config) if !config.consoles.is_empty() => match database::init_db() {
            Ok(conn) => from_config(&config, &conn),
            Err(err) => demo_library(&format!(
                "Demo library. Could not open the library database ({err})."
            )),
        },
        Ok(_) => demo_library("Demo library. No consoles in ~/.config/retromarchy/config.toml."),
        Err(err) => demo_library(&format!("Demo library. Could not read config ({err}).")),
    }
}

pub fn from_config(config: &config::Config, conn: &rusqlite::Connection) -> Library {
    let metadata = config::load_console_metadata().unwrap_or_default();
    let shelves = config
        .consoles
        .iter()
        .map(|console| shelf_from_disk(console, &metadata, conn))
        .collect();
    Library {
        kind: LibraryKind::Disk,
        note: String::new(),
        details_open: config.details_visible,
        shelves,
        profiles: config.profiles.clone(),
    }
}

fn shelf_from_disk(
    console: &Console,
    metadata: &[ConsoleMetadata],
    conn: &rusqlite::Connection,
) -> Shelf {
    let games = database::load_games(conn, Some(&console.id)).unwrap_or_default();
    let stats = database::get_library_stats(conn, &console.id).unwrap_or_else(|_| stats_of(&games));
    shelf(console.clone(), metadata, games, stats)
}

pub fn demo_library(note: &str) -> Library {
    let metadata = config::load_console_metadata().unwrap_or_default();
    let demo_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/demo");
    let box_art = demo_dir.join("snes-box.png");
    let shot = demo_dir.join("snes-shot.png");
    let mario = with_art(
        demo_game("snes", "Super Mario World", 12, 5400, Some(demo_played())),
        &box_art,
        &shot,
    );
    let sonic = with_art(
        demo_game("genesis", "Sonic the Hedgehog", 7, 2400, None),
        &box_art,
        &shot,
    );
    let shelves = vec![
        demo_shelf(
            "snes",
            "Super Nintendo",
            &metadata,
            GridArt::BoxArt,
            vec![
                mario,
                demo_game(
                    "snes",
                    "The Legend of Zelda: A Link to the Past",
                    4,
                    1800,
                    None,
                ),
                demo_game("snes", "Super Metroid", 2, 900, None),
                demo_game("snes", "Chrono Trigger", 0, 0, None),
            ],
        ),
        demo_shelf(
            "genesis",
            "Sega Genesis",
            &metadata,
            GridArt::Screenshot,
            vec![
                sonic,
                demo_game("genesis", "Streets of Rage 2", 1, 300, None),
                demo_game("genesis", "Gunstar Heroes", 0, 0, None),
            ],
        ),
        demo_shelf(
            "nes",
            "NES",
            &metadata,
            GridArt::BoxArt,
            vec![
                demo_game("nes", "Super Mario Bros.", 3, 600, None),
                demo_game("nes", "The Legend of Zelda", 0, 0, None),
                demo_game("nes", "Metroid", 0, 0, None),
            ],
        ),
    ];
    Library {
        kind: LibraryKind::Demo,
        note: note.to_string(),
        details_open: true,
        shelves,
        profiles: Vec::new(),
    }
}

fn demo_shelf(
    id: &str,
    name: &str,
    metadata: &[ConsoleMetadata],
    grid_art: GridArt,
    games: Vec<Game>,
) -> Shelf {
    let stats = stats_of(&games);
    shelf(
        Console {
            id: id.to_string(),
            name: name.to_string(),
            rom_dirs: Vec::new(),
            extensions: Vec::new(),
            profile: None,
            grid_art,
            media: Default::default(),
        },
        metadata,
        games,
        stats,
    )
}

fn with_art(mut game: Game, box_art: &Path, shot: &Path) -> Game {
    game.media = vec![
        Media {
            kind: MediaKind::BoxArt,
            path: box_art.to_path_buf(),
            source: Source::Local,
        },
        Media {
            kind: MediaKind::Screenshot,
            path: shot.to_path_buf(),
            source: Source::Local,
        },
    ];
    game
}

fn shelf(
    console: Console,
    metadata: &[ConsoleMetadata],
    games: Vec<Game>,
    stats: LibraryStats,
) -> Shelf {
    let meta = metadata.iter().find(|item| item.id == console.id);
    Shelf {
        manufacturer: meta.map(|item| item.manufacturer.clone()),
        year: meta.map(|item| item.year),
        description: meta.map(|item| item.description.clone()),
        console,
        stats,
        games,
    }
}

fn demo_game(
    console: &str,
    title: &str,
    play_count: u32,
    play_time: u32,
    last_played: Option<DateTime<Utc>>,
) -> Game {
    let slug = title
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    Game {
        id: format!("demo-{console}-{slug}"),
        console: console.to_string(),
        rom: PathBuf::from(format!("/demo/{console}/{slug}.rom")),
        title: title.to_string(),
        crc32: None,
        profile: None,
        media: Vec::new(),
        last_played,
        play_count,
        play_time,
        favorite: false,
    }
}

fn demo_played() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2024-11-02T18:30:00Z")
        .expect("fixed demo timestamp")
        .with_timezone(&Utc)
}

pub fn stats_of(games: &[Game]) -> LibraryStats {
    let mut last_played_date = None;
    let mut last_played_game = None;
    let mut total_play_count = 0;
    let mut total_play_time = 0;
    let mut most_played_game = None;
    let mut most_played_count = 0;
    for game in games {
        total_play_count += game.play_count;
        total_play_time += game.play_time;
        if game.play_count > most_played_count {
            most_played_count = game.play_count;
            most_played_game = Some(game.title.clone());
        }
        if let Some(played) = game.last_played {
            let newer = last_played_date
                .map(|current| played > current)
                .unwrap_or(true);
            if newer {
                last_played_date = Some(played);
                last_played_game = Some(game.title.clone());
            }
        }
    }
    LibraryStats {
        total_games: games.len() as u32,
        last_played_date,
        last_played_game,
        total_play_count,
        total_play_time,
        most_played_game: (most_played_count > 0)
            .then_some(most_played_game)
            .flatten(),
        most_played_count,
    }
}

pub fn format_play_time(seconds: u32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let remain = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{remain}s")
    }
}

pub fn cover_path(game: &Game, art: GridArt) -> Option<PathBuf> {
    for kind in art.fallback() {
        if let Some(path) = file_for(game, kind) {
            return Some(path);
        }
    }
    None
}

/// Width / height from a png, jpeg, gif, or webp header. Missing and unknown
/// files return none so the details pane can fall back to a fixed ratio.
/// Cached because the details pane asks again on every frame.
pub fn image_aspect(path: &Path) -> Option<f32> {
    static CACHE: Mutex<Option<HashMap<PathBuf, Option<f32>>>> = Mutex::new(None);
    let mut guard = CACHE.lock().unwrap_or_else(|err| err.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(ratio) = cache.get(path) {
        return *ratio;
    }
    let ratio = read_image_aspect(path);
    cache.insert(path.to_path_buf(), ratio);
    ratio
}

fn read_image_aspect(path: &Path) -> Option<f32> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(512 * 1024).read_to_end(&mut bytes).ok()?;
    let (width, height) = image_pixel_size(&bytes)?;
    if width == 0 || height == 0 {
        None
    } else {
        Some(width as f32 / height as f32)
    }
}

fn image_pixel_size(bytes: &[u8]) -> Option<(u32, u32)> {
    png_size(bytes)
        .or_else(|| gif_size(bytes))
        .or_else(|| webp_size(bytes))
        .or_else(|| jpeg_size(bytes))
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let width = u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32;
    let height = u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32;
    Some((width, height))
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8X" => {
            let width = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]).saturating_add(1);
            let height = u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]).saturating_add(1);
            Some((width, height))
        }
        b"VP8 " => {
            if bytes.get(23..26) != Some(&[0x9d, 0x01, 0x2a]) {
                return None;
            }
            let width = u16::from_le_bytes(bytes[26..28].try_into().ok()?) as u32 & 0x3fff;
            let height = u16::from_le_bytes(bytes[28..30].try_into().ok()?) as u32 & 0x3fff;
            Some((width, height))
        }
        b"VP8L" => {
            if bytes.get(20) != Some(&0x2f) {
                return None;
            }
            let bits = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
            let width = (bits & 0x3fff) + 1;
            let height = ((bits >> 14) & 0x3fff) + 1;
            Some((width, height))
        }
        _ => None,
    }
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut index = 2;
    while index + 3 < bytes.len() {
        if bytes[index] != 0xff {
            return None;
        }
        while index < bytes.len() && bytes[index] == 0xff {
            index += 1;
        }
        if index >= bytes.len() {
            return None;
        }
        let marker = bytes[index];
        index += 1;
        if marker == 0xd8 || marker == 0xd9 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if index + 1 >= bytes.len() {
            return None;
        }
        let len = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
        if len < 2 || index + len > bytes.len() {
            return None;
        }
        let sof = matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        );
        if sof {
            if len < 7 {
                return None;
            }
            let height = u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
            return Some((width, height));
        }
        index += len;
    }
    None
}

/// Systems list. Wide enough for a name like "Nintendo Entertainment System".
pub const SIDEBAR_WIDTH: f32 = 280.0;
pub const DETAILS_WIDTH: f32 = 280.0;
pub const DETAILS_PAD: f32 = 16.0;
pub const GRID_PAD: f32 = 16.0;
pub const TILE_GAP: f32 = 12.0;

/// Shared cover width. Screenshot and box-art cards use it, so a 1280px window
/// with both panes open shows about three columns. The shell can change it;
/// this remains the default stored in config.
pub use crate::config::{
    clamp_cover_width, step_cover_width, COVER_WIDTH_DEFAULT as COVER_WIDTH, COVER_WIDTH_MAX,
    COVER_WIDTH_MIN, COVER_WIDTH_STEP,
};

/// Fixed cover slot for one console grid. Every card in that grid uses the same
/// size so keyboard columns and painted rows stay the same grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileFrame {
    pub width: f32,
    pub height: f32,
}

impl TileFrame {
    pub fn for_art(art: GridArt, width: f32) -> Self {
        let width = clamp_cover_width(width);
        match art {
            // 4:3. A screenshot fills the slot. Anything else is letterboxed inside it.
            GridArt::Screenshot => Self {
                width,
                height: width * 3.0 / 4.0,
            },
            // Same width, 3:4. A box fills the taller slot. Contain does not crop it.
            GridArt::BoxArt => Self {
                width,
                height: width * 4.0 / 3.0,
            },
        }
    }

    pub fn ratio(self) -> f32 {
        self.width / self.height
    }
}

pub fn row_of(index: usize, columns: usize) -> usize {
    index / columns.max(1)
}

pub fn file_for(game: &Game, kind: MediaKind) -> Option<PathBuf> {
    game.media
        .iter()
        .find_map(|media| (media.kind == kind && media.path.is_file()).then(|| media.path.clone()))
}

impl Browse {
    pub fn new(library: Library) -> Self {
        Self::with_cover_width(library, COVER_WIDTH)
    }

    pub fn with_cover_width(library: Library, cover_width: f32) -> Self {
        let details_open = library.details_open;
        Self {
            library,
            pane: Pane::Sidebar,
            console: 0,
            game: None,
            details_open,
            columns: 4,
            cover_width: clamp_cover_width(cover_width),
            status: String::new(),
        }
    }

    /// Cover width from disk. A missing or unreadable config keeps [`COVER_WIDTH`].
    pub fn saved_cover_width() -> f32 {
        config::load_config()
            .map(|config| config.cover_width)
            .unwrap_or(COVER_WIDTH)
    }

    pub fn shelf(&self) -> Option<&Shelf> {
        self.library.shelves.get(self.console)
    }

    pub fn selected_game(&self) -> Option<&Game> {
        let index = self.game?;
        self.shelf()?.games.get(index)
    }

    pub fn apply(&mut self, key: Key) {
        match key {
            Key::ToggleDetails => self.details_open = !self.details_open,
            Key::CoverSmaller => self.cover_width = step_cover_width(self.cover_width, -1),
            Key::CoverLarger => self.cover_width = step_cover_width(self.cover_width, 1),
            Key::Clear => {
                self.game = None;
                self.pane = Pane::Sidebar;
            }
            Key::EnterGrid => self.enter_grid(),
            Key::Launch => {}
            Key::Arrow(dir) => self.move_arrow(dir),
            Key::Grid(dir) => self.step_grid(dir),
        }
    }

    pub fn set_columns(&mut self, columns: usize) {
        self.columns = columns.max(1);
    }

    pub fn tile_frame(&self) -> TileFrame {
        self.shelf()
            .map(|shelf| TileFrame::for_art(shelf.console.grid_art, self.cover_width))
            .unwrap_or_else(|| TileFrame::for_art(GridArt::BoxArt, self.cover_width))
    }

    pub fn select_console(&mut self, index: usize) {
        if index >= self.library.shelves.len() {
            return;
        }
        self.console = index;
        self.game = None;
        self.pane = Pane::Sidebar;
    }

    pub fn select_game(&mut self, index: usize) {
        let len = self.shelf().map(|shelf| shelf.games.len()).unwrap_or(0);
        if index >= len {
            return;
        }
        self.game = Some(index);
        self.pane = Pane::Grid;
    }

    fn move_arrow(&mut self, dir: NavDir) {
        match self.pane {
            Pane::Sidebar => match dir {
                NavDir::Up | NavDir::Down => self.step_sidebar(dir),
                NavDir::Right => self.enter_grid(),
                NavDir::Left => {}
            },
            Pane::Grid => {
                if dir == NavDir::Left && self.at_row_start() {
                    self.pane = Pane::Sidebar;
                    self.game = None;
                    return;
                }
                self.step_grid(dir);
            }
        }
    }

    fn step_sidebar(&mut self, dir: NavDir) {
        let len = self.library.shelves.len() as i32;
        let Some(next) = list_step(Some(self.console as i32), dir, len) else {
            return;
        };
        if next as usize != self.console {
            self.console = next as usize;
            self.game = None;
        }
    }

    fn enter_grid(&mut self) {
        self.pane = Pane::Grid;
        let len = self.shelf().map(|shelf| shelf.games.len()).unwrap_or(0);
        if len == 0 {
            self.game = None;
            return;
        }
        if self.game.is_none() {
            self.game = Some(0);
        }
    }

    fn step_grid(&mut self, dir: NavDir) {
        let len = self.shelf().map(|shelf| shelf.games.len()).unwrap_or(0) as i32;
        let columns = self.columns.max(1) as i32;
        let index = self.game.map(|index| index as i32);
        if let Some(next) = grid_step(index, dir, len, columns) {
            self.game = Some(next as usize);
            self.pane = Pane::Grid;
        }
    }

    fn at_row_start(&self) -> bool {
        match self.game {
            Some(index) => index % self.columns.max(1) == 0,
            None => true,
        }
    }
}

pub fn resolve_profile(library: &Library, game: &Game) -> Option<EmulatorProfile> {
    let from_game = game.profile.as_deref().filter(|id| !id.is_empty());
    let id = from_game.or_else(|| {
        library
            .shelves
            .iter()
            .find(|shelf| shelf.console.id == game.console)
            .and_then(|shelf| shelf.console.profile.as_deref())
            .filter(|id| !id.is_empty())
    })?;
    library
        .profiles
        .iter()
        .find(|profile| profile.id() == id)
        .cloned()
}

pub fn columns_for(width: f32, details_open: bool, tile_width: f32) -> usize {
    let tile_width = tile_width.max(1.0);
    let reserved =
        SIDEBAR_WIDTH + if details_open { DETAILS_WIDTH } else { 0.0 } + GRID_PAD * 2.0 + 16.0;
    let grid = (width - reserved).max(tile_width);
    let columns = ((grid + TILE_GAP) / (tile_width + TILE_GAP)).floor() as usize;
    columns.clamp(1, 8)
}

pub fn key_from_name(name: &str, modified: bool) -> Option<Key> {
    key_from_parts(name, None, modified)
}

/// `name` is the GPUI key. `key_char` is the typed character, used when a
/// layout reports numpad `-` / `+` only there.
pub fn key_from_parts(name: &str, key_char: Option<&str>, modified: bool) -> Option<Key> {
    if modified {
        return None;
    }
    map_key(name).or_else(|| key_char.and_then(map_key))
}

fn map_key(name: &str) -> Option<Key> {
    Some(match name {
        "up" => Key::Arrow(NavDir::Up),
        "down" => Key::Arrow(NavDir::Down),
        "left" => Key::Arrow(NavDir::Left),
        "right" => Key::Arrow(NavDir::Right),
        "h" => Key::Grid(NavDir::Left),
        "j" => Key::Grid(NavDir::Down),
        "k" => Key::Grid(NavDir::Up),
        "l" => Key::Grid(NavDir::Right),
        "tab" => Key::EnterGrid,
        "escape" => Key::Clear,
        "d" => Key::ToggleDetails,
        "enter" => Key::Launch,
        "-" | "minus" | "subtract" | "kp_subtract" | "numpadsubtract" => Key::CoverSmaller,
        "+" | "plus" | "=" | "equal" | "equals" | "add" | "kp_add" | "numpadadd" => {
            Key::CoverLarger
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Media, MediaToggles, Source};
    use std::fs;

    fn sample() -> Browse {
        Browse::new(demo_library("Demo library."))
    }

    #[test]
    fn arrows_move_sidebar_and_grid() {
        let mut browse = sample();
        browse.apply(Key::Arrow(NavDir::Down));
        assert_eq!(browse.console, 1);
        assert_eq!(browse.shelf().unwrap().console.id, "genesis");
        assert_eq!(browse.game, None);

        browse.apply(Key::Arrow(NavDir::Right));
        assert_eq!(browse.pane, Pane::Grid);
        assert_eq!(browse.selected_game().unwrap().title, "Sonic the Hedgehog");

        browse.set_columns(2);
        browse.apply(Key::Arrow(NavDir::Right));
        assert_eq!(browse.selected_game().unwrap().title, "Streets of Rage 2");
        browse.apply(Key::Arrow(NavDir::Left));
        assert_eq!(browse.selected_game().unwrap().title, "Sonic the Hedgehog");
        browse.apply(Key::Arrow(NavDir::Left));
        assert_eq!(browse.pane, Pane::Sidebar);
        assert_eq!(browse.game, None);

        browse.apply(Key::Arrow(NavDir::Right));
        browse.apply(Key::Arrow(NavDir::Down));
        assert_eq!(browse.selected_game().unwrap().title, "Gunstar Heroes");
    }

    #[test]
    fn vim_keys_enter_the_grid_and_escape_clears() {
        let mut browse = sample();
        browse.apply(Key::Grid(NavDir::Down));
        assert_eq!(browse.pane, Pane::Grid);
        assert_eq!(browse.game, Some(0));
        browse.set_columns(2);
        browse.apply(Key::Grid(NavDir::Down));
        assert_eq!(browse.game, Some(2));
        browse.apply(Key::Clear);
        assert_eq!(browse.pane, Pane::Sidebar);
        assert_eq!(browse.game, None);
        assert!(browse.details_open);
        browse.apply(Key::ToggleDetails);
        assert!(!browse.details_open);
    }

    #[test]
    fn cover_uses_grid_art_then_the_other_file() {
        let dir = tempfile::tempdir().unwrap();
        let shot = dir.path().join("shot.png");
        fs::write(&shot, b"png").unwrap();
        let mut game = demo_game("snes", "Example", 0, 0, None);
        game.media.push(Media {
            kind: MediaKind::Screenshot,
            path: shot.clone(),
            source: Source::Local,
        });
        assert_eq!(cover_path(&game, GridArt::BoxArt), Some(shot));
        let missing = PathBuf::from("/no/such/box.png");
        game.media.insert(
            0,
            Media {
                kind: MediaKind::BoxArt,
                path: missing,
                source: Source::Local,
            },
        );
        assert!(cover_path(&game, GridArt::BoxArt)
            .unwrap()
            .ends_with("shot.png"));
    }

    #[test]
    fn disk_library_reads_the_same_database() {
        let dir = tempfile::tempdir().unwrap();
        let conn = database::open_db(&dir.path().join("library.db")).unwrap();
        let mut game = demo_game("snes", "Chrono Trigger", 2, 90, None);
        game.rom = PathBuf::from("/roms/snes/chrono.sfc");
        database::upsert_game(&conn, &game).unwrap();
        let config = config::Config {
            consoles: vec![Console {
                id: "snes".into(),
                name: "Super Nintendo".into(),
                rom_dirs: Vec::new(),
                extensions: vec!["sfc".into()],
                profile: None,
                grid_art: GridArt::BoxArt,
                media: MediaToggles::default(),
            }],
            details_visible: false,
            ..config::Config::default()
        };
        let library = from_config(&config, &conn);
        assert_eq!(library.kind, LibraryKind::Disk);
        assert!(!library.details_open);
        assert_eq!(library.shelves[0].games[0].title, "Chrono Trigger");
        assert_eq!(library.shelves[0].stats.total_games, 1);
        assert_eq!(library.shelves[0].manufacturer.as_deref(), Some("Nintendo"));
    }

    #[test]
    fn play_time_and_columns() {
        assert_eq!(format_play_time(45), "45s");
        assert_eq!(format_play_time(120), "2m");
        assert_eq!(format_play_time(5400), "1h 30m");
        let shot = TileFrame::for_art(GridArt::Screenshot, COVER_WIDTH);
        let box_art = TileFrame::for_art(GridArt::BoxArt, COVER_WIDTH);
        assert_eq!(shot.width, box_art.width);
        assert_eq!(shot.width, COVER_WIDTH);
        assert!(box_art.height > shot.height);
        assert!((shot.ratio() - 4.0 / 3.0).abs() < 0.001);
        assert!((box_art.ratio() - 3.0 / 4.0).abs() < 0.001);
        let wide = TileFrame::for_art(GridArt::Screenshot, 320.0);
        let tall = TileFrame::for_art(GridArt::BoxArt, 320.0);
        assert_eq!(wide.width, 320.0);
        assert_eq!(tall.width, 320.0);
        assert!((wide.ratio() - 4.0 / 3.0).abs() < 0.001);
        assert!((tall.ratio() - 3.0 / 4.0).abs() < 0.001);
        assert!(tall.height > wide.height);
        assert_eq!(columns_for(1280.0, true, shot.width), 3);
        assert_eq!(columns_for(1280.0, true, box_art.width), 3);
        assert!(columns_for(1600.0, false, shot.width) > columns_for(1280.0, true, shot.width));
        assert!(columns_for(1400.0, true, COVER_WIDTH) > columns_for(1400.0, true, 320.0));
        assert!(columns_for(1280.0, true, COVER_WIDTH_MAX) >= 1);
        assert!(columns_for(700.0, true, COVER_WIDTH_MAX) >= 1);
        assert!(columns_for(1280.0, true, COVER_WIDTH_MIN) >= 1);
        assert_eq!(row_of(0, 3), 0);
        assert_eq!(row_of(5, 3), 1);
    }

    #[test]
    fn click_selects_console_and_game() {
        let mut browse = sample();
        browse.select_game(99);
        assert_eq!(browse.game, None);
        browse.select_console(1);
        assert_eq!(browse.console, 1);
        assert_eq!(browse.pane, Pane::Sidebar);
        assert_eq!(browse.game, None);
        browse.select_game(2);
        assert_eq!(browse.pane, Pane::Grid);
        assert_eq!(browse.selected_game().unwrap().title, "Gunstar Heroes");
        browse.select_console(0);
        assert_eq!(browse.console, 0);
        assert_eq!(browse.game, None);
        assert_eq!(browse.pane, Pane::Sidebar);
    }

    #[test]
    fn cover_width_steps_and_stays_global() {
        let mut browse = sample();
        assert_eq!(browse.cover_width, COVER_WIDTH);
        browse.apply(Key::CoverLarger);
        assert_eq!(browse.cover_width, COVER_WIDTH + COVER_WIDTH_STEP);
        browse.apply(Key::CoverSmaller);
        assert_eq!(browse.cover_width, COVER_WIDTH);

        browse.cover_width = 250.0;
        let before = browse.tile_frame();
        browse.select_console(1);
        assert_eq!(browse.shelf().unwrap().console.id, "genesis");
        assert_eq!(browse.cover_width, 250.0);
        assert_eq!(browse.tile_frame().width, before.width);
        browse.select_console(2);
        assert_eq!(browse.cover_width, 250.0);
        assert_eq!(browse.tile_frame().width, 250.0);
        // Box art is taller than a screenshot at the same width.
        browse.select_console(0);
        assert!(browse.tile_frame().height > TileFrame::for_art(GridArt::Screenshot, 250.0).height);

        browse.cover_width = COVER_WIDTH_MAX;
        browse.apply(Key::CoverLarger);
        assert_eq!(browse.cover_width, COVER_WIDTH_MAX);
        browse.cover_width = COVER_WIDTH_MIN;
        browse.apply(Key::CoverSmaller);
        assert_eq!(browse.cover_width, COVER_WIDTH_MIN);
    }

    #[test]
    fn minus_and_plus_resize_covers() {
        assert_eq!(key_from_name("-", false), Some(Key::CoverSmaller));
        assert_eq!(key_from_name("+", false), Some(Key::CoverLarger));
        assert_eq!(key_from_name("=", false), Some(Key::CoverLarger));
        assert_eq!(key_from_name("subtract", false), Some(Key::CoverSmaller));
        assert_eq!(key_from_name("add", false), Some(Key::CoverLarger));
        assert_eq!(key_from_name("kp_subtract", false), Some(Key::CoverSmaller));
        assert_eq!(key_from_name("kp_add", false), Some(Key::CoverLarger));
        assert_eq!(
            key_from_parts("unknown", Some("-"), false),
            Some(Key::CoverSmaller)
        );
        assert_eq!(
            key_from_parts("unknown", Some("+"), false),
            Some(Key::CoverLarger)
        );
        assert_eq!(key_from_name("-", true), None);
        assert_eq!(key_from_name("d", false), Some(Key::ToggleDetails));
    }

    #[test]
    fn image_aspect_reads_headers() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("box.png");
        let mut bytes = vec![
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 13, b'I', b'H', b'D', b'R',
        ];
        bytes.extend(150u32.to_be_bytes());
        bytes.extend(200u32.to_be_bytes());
        fs::write(&png, &bytes).unwrap();
        assert!((image_aspect(&png).unwrap() - 0.75).abs() < 0.001);

        let gif = dir.path().join("shot.gif");
        let mut gif_bytes = b"GIF89a".to_vec();
        gif_bytes.extend(320u16.to_le_bytes());
        gif_bytes.extend(240u16.to_le_bytes());
        fs::write(&gif, gif_bytes).unwrap();
        assert!((image_aspect(&gif).unwrap() - 320.0 / 240.0).abs() < 0.001);

        let jpeg = dir.path().join("shot.jpg");
        let jpeg_bytes = [
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x64, 0x00, 0xc8, 0x01, 0x01, 0x11,
            0x00,
        ];
        fs::write(&jpeg, jpeg_bytes).unwrap();
        assert!((image_aspect(&jpeg).unwrap() - 2.0).abs() < 0.001);

        let webp = dir.path().join("box.webp");
        let mut webp_bytes = b"RIFF".to_vec();
        webp_bytes.extend(0u32.to_le_bytes());
        webp_bytes.extend(b"WEBPVP8X");
        webp_bytes.extend(10u32.to_le_bytes());
        webp_bytes.extend([0, 0, 0, 0]);
        webp_bytes.extend((216u32 - 1).to_le_bytes()[..3].to_vec());
        webp_bytes.extend((288u32 - 1).to_le_bytes()[..3].to_vec());
        fs::write(&webp, webp_bytes).unwrap();
        assert!((image_aspect(&webp).unwrap() - 216.0 / 288.0).abs() < 0.001);

        assert!(image_aspect(Path::new("/no/such/image.png")).is_none());
    }
}
