use crate::types::{
    GameMetadata, GridArt, Media, MediaKind, ScrapeProvider, ScraperConfig, ScraperCredentials,
    Source,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const SCRAPE_KINDS: [MediaKind; 2] = [MediaKind::BoxArt, MediaKind::Screenshot];

/// Kinds whose file is already on disk. Missing paths do not count.
pub fn present_kinds(media: &[Media]) -> Vec<MediaKind> {
    let mut kinds = Vec::new();
    for kind in SCRAPE_KINDS {
        if media
            .iter()
            .any(|item| item.kind == kind && item.path.is_file())
        {
            kinds.push(kind);
        }
    }
    kinds
}

/// Enabled kinds that do not already have a local file.
pub fn kinds_to_fetch(have_files: &[MediaKind], enabled: &[MediaKind]) -> Vec<MediaKind> {
    enabled
        .iter()
        .copied()
        .filter(|kind| SCRAPE_KINDS.contains(kind) && !have_files.contains(kind))
        .collect()
}

/// Enabled providers in list order. Later duplicates are ignored.
pub fn provider_order(providers: &[crate::types::ProviderEntry]) -> Vec<ScrapeProvider> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for entry in providers {
        if entry.enabled && seen.insert(entry.id) {
            out.push(entry.id);
        }
    }
    out
}

pub fn pick_kind(have: &[MediaKind], preferred: GridArt) -> Option<MediaKind> {
    preferred
        .fallback()
        .into_iter()
        .find(|kind| have.contains(kind))
}

pub fn credential_blocks(
    creds: &ScraperCredentials,
    providers: &[ScrapeProvider],
) -> Vec<&'static str> {
    let mut messages = Vec::new();
    for provider in providers {
        if let Some(reason) = creds.block_reason(*provider) {
            if !messages.contains(&reason) {
                messages.push(reason);
            }
        }
    }
    messages
}

pub fn ready_providers(
    creds: &ScraperCredentials,
    providers: &[ScrapeProvider],
) -> Vec<ScrapeProvider> {
    providers
        .iter()
        .copied()
        .filter(|provider| creds.block_reason(*provider).is_none())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFail {
    Failed(String),
    RateLimited(String),
}

pub struct ChainOutcome<T> {
    pub success: Option<(ScrapeProvider, T)>,
    pub errors: Vec<(ScrapeProvider, FetchFail)>,
}

/// Try providers in order. The first success wins and later providers are not called.
pub fn take_first_success<T>(
    providers: &[ScrapeProvider],
    mut fetch: impl FnMut(ScrapeProvider) -> Result<T, FetchFail>,
) -> ChainOutcome<T> {
    let mut errors = Vec::new();
    for provider in providers {
        match fetch(*provider) {
            Ok(value) => {
                return ChainOutcome {
                    success: Some((*provider, value)),
                    errors,
                };
            }
            Err(err) => errors.push((*provider, err)),
        }
    }
    ChainOutcome {
        success: None,
        errors,
    }
}

pub fn media_root() -> anyhow::Result<PathBuf> {
    let xdg = xdg::BaseDirectories::with_prefix("retromarchy")?;
    let dir = xdg.get_data_home().join("media");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn cache_relative(console_id: &str, game_id: &str, kind: MediaKind, ext: &str) -> PathBuf {
    PathBuf::from(console_id)
        .join(game_id)
        .join(format!("{}.{}", kind_file_stem(kind), ext))
}

fn kind_file_stem(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::BoxArt => "box_art",
        MediaKind::TitleScreen => "title_screen",
        MediaKind::Screenshot => "screenshot",
        MediaKind::Manual => "manual",
        MediaKind::Video => "video",
    }
}

pub fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

pub fn url_allowed(url: &str) -> bool {
    let (scheme, rest) = match url.split_once("://") {
        Some(pair) => pair,
        None => return false,
    };
    if scheme != "https" && scheme != "http" {
        return false;
    }
    let path = rest.split('?').next().unwrap_or(rest).to_ascii_lowercase();
    const REJECT: &[&str] = &[
        ".zip", ".7z", ".rar", ".iso", ".chd", ".cue", ".bin", ".nes", ".sfc", ".smc", ".md",
        ".gba", ".nds", ".rom", ".img", ".n64", ".z64", ".gb", ".gbc", ".sms", ".gg", ".pdf",
        ".mp4", ".mkv", ".avi", ".webm", ".txt",
    ];
    !REJECT.iter().any(|ext| path.ends_with(ext))
}

pub fn write_cached_image(
    root: &Path,
    console_id: &str,
    game_id: &str,
    kind: MediaKind,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let ext = image_extension(bytes)
        .ok_or_else(|| "response was not a png, jpeg, gif, or webp image".to_string())?;
    let relative = cache_relative(console_id, game_id, kind, ext);
    let path = root.join(&relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(&path, bytes).map_err(|err| err.to_string())?;
    Ok(path)
}

/// Delete the ROM path when it is a file. A directory is left in place.
pub fn delete_rom_file(path: &Path) -> std::io::Result<()> {
    if path.is_file() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Remove `root/<console>/<game id>/` and nothing above it.
/// Sidecar files next to the ROM, and any other game's cache, stay put.
pub fn delete_cached_assets(root: &Path, game: &crate::types::Game) -> std::io::Result<()> {
    let Some(dir) = game_cache_dir(root, &game.console, &game.id) else {
        return Ok(());
    };
    fs::remove_dir_all(dir)
}

fn is_single_segment(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
}

fn game_cache_dir(root: &Path, console: &str, game_id: &str) -> Option<PathBuf> {
    if !is_single_segment(console) || !is_single_segment(game_id) {
        return None;
    }
    let dir = root.join(console).join(game_id);
    if !dir.is_dir() {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let canon = dir.canonicalize().ok()?;
    let parent = canon.parent()?;
    if parent.parent()? != root {
        return None;
    }
    if parent.file_name()?.to_str()? != console || canon.file_name()?.to_str()? != game_id {
        return None;
    }
    Some(canon)
}

#[derive(Debug, Clone)]
pub struct ArtworkQuery {
    pub console_id: String,
    pub title: String,
    pub rom_name: String,
    /// CRC sent to ScreenScraper. Headered A78 files use the payload only.
    pub crc32: Option<u32>,
    /// Size sent as `romtaille`. Headered A78 files omit the 128-byte header.
    pub rom_bytes: Option<u64>,
    /// Bytes skipped before `crc32` / `rom_bytes`. Zero when the file is sent whole.
    pub header_bytes: u64,
}

impl ArtworkQuery {
    pub fn from_game(game: &crate::types::Game) -> Self {
        let rom_name = game
            .rom
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (crc32, rom_bytes, header_bytes) =
            scrape_fingerprint(&game.console, &game.rom, game.crc32);
        Self {
            console_id: game.console.clone(),
            title: game.display_title().to_string(),
            rom_name,
            crc32,
            rom_bytes,
            header_bytes,
        }
    }
}

const A78_HEADER_LEN: u64 = 128;

fn scrape_fingerprint(
    console_id: &str,
    path: &Path,
    stored_crc: Option<u32>,
) -> (Option<u32>, Option<u64>, u64) {
    let full_len = fs::metadata(path).ok().map(|meta| meta.len());
    if !may_have_a78_header(console_id, path) {
        return (stored_crc, full_len, 0);
    }
    match a78_payload_fingerprint(path) {
        Some((crc, size)) => (Some(crc), Some(size), A78_HEADER_LEN),
        None => (stored_crc, full_len, 0),
    }
}

fn may_have_a78_header(console_id: &str, path: &Path) -> bool {
    is_atari_7800(console_id) || extension_is(path, "a78")
}

fn is_atari_7800(console_id: &str) -> bool {
    matches!(
        console_id.to_ascii_lowercase().as_str(),
        "atari7800" | "a7800" | "7800"
    )
}

fn extension_is(path: &Path, want: &str) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(want))
}

/// CRC and size of an A78 ROM after its 128-byte header.
/// The header is version byte plus `ATARI7800` at offset 1. Other files stay whole.
fn a78_payload_fingerprint(path: &Path) -> Option<(u32, u64)> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len <= A78_HEADER_LEN {
        return None;
    }
    let mut header = [0u8; A78_HEADER_LEN as usize];
    file.read_exact(&mut header).ok()?;
    if &header[1..10] != b"ATARI7800" {
        return None;
    }
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = [0u8; 8192];
    loop {
        let count = file.read(&mut buf).ok()?;
        if count == 0 {
            break;
        }
        hasher.update(&buf[..count]);
    }
    Some((hasher.finalize(), len - A78_HEADER_LEN))
}

pub fn screenscraper_params(
    creds: &ScraperCredentials,
    query: &ArtworkQuery,
) -> Vec<(String, String)> {
    let mut params = vec![
        ("devid".to_string(), crate::softname::DEVID.to_string()),
        (
            "devpassword".to_string(),
            crate::softname::DEVPASSWORD.to_string(),
        ),
        (
            "softname".to_string(),
            crate::softname::SOFTNAME.to_string(),
        ),
        ("output".to_string(), "json".to_string()),
        ("ssid".to_string(), creds.screenscraper_user.clone()),
        (
            "sspassword".to_string(),
            creds.screenscraper_password.clone(),
        ),
        ("romtype".to_string(), "rom".to_string()),
        ("romnom".to_string(), query.rom_name.clone()),
    ];
    if let Some(system_id) = screenscraper_system(&query.console_id) {
        params.push(("systemeid".to_string(), system_id.to_string()));
    }
    if let Some(crc) = query.crc32 {
        params.push(("crc".to_string(), format!("{crc:08X}")));
    }
    if let Some(size) = query.rom_bytes {
        params.push(("romtaille".to_string(), size.to_string()));
    }
    params
}

const NAME_REGIONS: [&str; 3] = ["us", "wor", "eu"];

#[derive(Debug)]
struct SsGame {
    medias: Vec<(String, String, String)>,
    metadata: GameMetadata,
}

fn parse_screenscraper_game(body: &str) -> Result<SsGame, FetchFail> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| FetchFail::Failed("ScreenScraper returned invalid JSON".into()))?;
    if let Some(error) = value
        .get("header")
        .and_then(|header| header.get("success"))
        .and_then(|flag| flag.as_str())
        .filter(|flag| *flag != "true")
        .and(value.pointer("/header/error").and_then(|err| err.as_str()))
    {
        return Err(classify_provider_error(
            ScrapeProvider::ScreenScraper,
            error,
        ));
    }
    let jeu = value
        .pointer("/response/jeu")
        .filter(|jeu| jeu.is_object())
        .ok_or_else(|| FetchFail::Failed("ScreenScraper has no game match".into()))?;
    let mut medias = Vec::new();
    if let Some(items) = jeu.get("medias").and_then(|medias| medias.as_array()) {
        for media in items {
            let type_name = media.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let region = media.get("region").and_then(|v| v.as_str()).unwrap_or("");
            let url = media.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if type_name.is_empty() || url.is_empty() {
                continue;
            }
            medias.push((type_name.to_string(), region.to_string(), url.to_string()));
        }
    }
    Ok(SsGame {
        medias,
        metadata: screenscraper_metadata(jeu),
    })
}

fn screenscraper_metadata(jeu: &serde_json::Value) -> GameMetadata {
    let noms = json_games(jeu.get("noms"), "text");
    let title = NAME_REGIONS.iter().find_map(|region| {
        noms.iter().find_map(|nom| {
            let matches = nom.get("region").and_then(|value| value.as_str()) == Some(*region);
            matches
                .then(|| nom.get("text").and_then(|text| text.as_str()))
                .flatten()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
    });
    let publisher = jeu
        .pointer("/editeur/text")
        .and_then(|text| text.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    GameMetadata {
        title,
        publisher,
        year: year_from_dates(jeu.get("dates")),
        genre: english_genres(jeu.get("genres")),
    }
}

fn year_from_dates(dates: Option<&serde_json::Value>) -> Option<u32> {
    let dates = json_games(dates, "text");
    for region in NAME_REGIONS {
        if let Some(year) = dates.iter().find_map(|date| {
            (date.get("region").and_then(|value| value.as_str()) == Some(region))
                .then(|| date.get("text").and_then(|text| text.as_str()))
                .flatten()
                .and_then(release_year)
        }) {
            return Some(year);
        }
    }
    dates.iter().find_map(|date| {
        date.get("text")
            .and_then(|text| text.as_str())
            .and_then(release_year)
    })
}

fn release_year(text: &str) -> Option<u32> {
    let text = text.trim();
    let year_digits = match text.len() {
        4 => text,
        7 if text.as_bytes()[4] == b'-' && text[5..].bytes().all(|byte| byte.is_ascii_digit()) => {
            &text[..4]
        }
        10 if text.as_bytes()[4] == b'-'
            && text.as_bytes()[7] == b'-'
            && text[5..7].bytes().all(|byte| byte.is_ascii_digit())
            && text[8..].bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            &text[..4]
        }
        _ => return None,
    };
    if !year_digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year: u32 = year_digits.parse().ok()?;
    (1900..=2099).contains(&year).then_some(year)
}

fn english_genres(genres: Option<&serde_json::Value>) -> Option<String> {
    let mut seen = Vec::new();
    for genre in json_games(genres, "id") {
        for nom in json_games(genre.get("noms"), "text") {
            if nom.get("langue").and_then(|value| value.as_str()) != Some("en") {
                continue;
            }
            let Some(text) = nom
                .get("text")
                .and_then(|text| text.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            if !seen.iter().any(|have: &String| have == text) {
                seen.push(text.to_string());
            }
        }
    }
    if seen.is_empty() {
        None
    } else {
        Some(seen.join(", "))
    }
}

pub fn pick_screenscraper_url(
    medias: &[(String, String, String)],
    kind: MediaKind,
) -> Option<String> {
    let wanted = screenscraper_media_type(kind)?;
    const REGIONS: &[&str] = &["wor", "us", "eu", "jp", "ss"];
    let matching: Vec<_> = medias.iter().filter(|media| media.0 == wanted).collect();
    for region in REGIONS {
        if let Some(media) = matching.iter().find(|media| media.1 == *region) {
            return Some(media.2.clone());
        }
    }
    matching.first().map(|media| media.2.clone())
}

fn screenscraper_media_type(kind: MediaKind) -> Option<&'static str> {
    match kind {
        MediaKind::BoxArt => Some("box-2D"),
        MediaKind::Screenshot => Some("ss"),
        MediaKind::TitleScreen | MediaKind::Manual | MediaKind::Video => None,
    }
}

pub fn parse_thegamesdb_game_id(body: &str, title: &str) -> Result<i64, FetchFail> {
    Ok(parse_thegamesdb_game(body, title)?.id)
}

#[derive(Clone)]
struct TgGame {
    id: i64,
    exact: bool,
    title: Option<String>,
    year: Option<u32>,
    genre_ids: Vec<i64>,
    publisher_ids: Vec<i64>,
}

fn parse_thegamesdb_game(body: &str, title: &str) -> Result<TgGame, FetchFail> {
    let games = thegamesdb_games(body)?;
    let exact_at = games.iter().position(|game| {
        game.get("game_title")
            .and_then(|title_value| title_value.as_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(title))
    });
    let game = exact_at.map(|index| &games[index]).unwrap_or(&games[0]);
    tg_game_from(game, exact_at.is_some())
}

fn parse_thegamesdb_game_by_id(body: &str, id: i64) -> Result<TgGame, FetchFail> {
    let games = thegamesdb_games(body)?;
    let game = games
        .iter()
        .find(|game| game.get("id").and_then(|value| value.as_i64()) == Some(id))
        .ok_or_else(|| FetchFail::Failed("TheGamesDB game id missing".into()))?;
    tg_game_from(game, true)
}

fn thegamesdb_games(body: &str) -> Result<Vec<serde_json::Value>, FetchFail> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| FetchFail::Failed("TheGamesDB returned invalid JSON".into()))?;
    if let Some(message) = api_error_message(&value) {
        return Err(classify_provider_error(ScrapeProvider::TheGamesDb, message));
    }
    let games = value
        .pointer("/data/games")
        .and_then(|games| games.as_array())
        .cloned()
        .unwrap_or_default();
    if games.is_empty() {
        return Err(FetchFail::Failed("TheGamesDB has no game match".into()));
    }
    Ok(games)
}

fn tg_game_from(game: &serde_json::Value, exact: bool) -> Result<TgGame, FetchFail> {
    let id = game
        .get("id")
        .and_then(|id| id.as_i64())
        .ok_or_else(|| FetchFail::Failed("TheGamesDB game id missing".into()))?;
    let title = game
        .get("game_title")
        .and_then(|title| title.as_str())
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    let year = game
        .get("release_date")
        .and_then(|date| date.as_str())
        .and_then(release_year);
    Ok(TgGame {
        id,
        exact,
        title,
        year,
        genre_ids: json_id_list(game.get("genres")),
        publisher_ids: json_id_list(game.get("publishers")),
    })
}

fn json_id_list(value: Option<&serde_json::Value>) -> Vec<i64> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        serde_json::Value::Array(items) => items.iter().filter_map(json_i64).collect(),
        serde_json::Value::Null => Vec::new(),
        other => json_i64(other).into_iter().collect(),
    }
}

fn json_i64(value: &serde_json::Value) -> Option<i64> {
    if let Some(id) = value.as_i64() {
        return Some(id);
    }
    if let Some(text) = value.as_str() {
        return text.trim().parse().ok();
    }
    value.get("id").and_then(json_i64)
}

#[derive(Debug, Clone, Copy)]
enum TgList {
    Genres,
    Publishers,
}

impl TgList {
    fn path(self) -> &'static str {
        match self {
            Self::Genres => "Genres/ByGenreID",
            Self::Publishers => "Publishers/ByPublisherID",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Genres => "genres",
            Self::Publishers => "publishers",
        }
    }
}

fn parse_thegamesdb_names(body: &str, list: TgList) -> Result<Vec<(i64, String)>, FetchFail> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| FetchFail::Failed("TheGamesDB returned invalid JSON".into()))?;
    if let Some(message) = api_error_message(&value) {
        return Err(classify_provider_error(ScrapeProvider::TheGamesDb, message));
    }
    let rows = value
        .pointer(&format!("/data/{}", list.key()))
        .and_then(|rows| rows.as_object())
        .ok_or_else(|| FetchFail::Failed("TheGamesDB returned no names".into()))?;
    let mut out = Vec::new();
    for (key, item) in rows {
        let id = item
            .get("id")
            .and_then(json_i64)
            .or_else(|| key.parse().ok());
        let name = item
            .get("name")
            .and_then(|name| name.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty());
        if let (Some(id), Some(name)) = (id, name) {
            out.push((id, name.to_string()));
        }
    }
    Ok(out)
}

#[derive(Default)]
struct TgNames {
    genres: HashMap<i64, String>,
    publishers: HashMap<i64, String>,
}

impl TgNames {
    fn fill(
        &mut self,
        agent: &ureq::Agent,
        creds: &ScraperCredentials,
        game: &TgGame,
        last_http: &mut Option<Instant>,
    ) -> Result<(), FetchFail> {
        self.fill_list(agent, creds, TgList::Genres, &game.genre_ids, last_http)?;
        self.fill_list(
            agent,
            creds,
            TgList::Publishers,
            &game.publisher_ids,
            last_http,
        )?;
        Ok(())
    }

    fn fill_list(
        &mut self,
        agent: &ureq::Agent,
        creds: &ScraperCredentials,
        list: TgList,
        ids: &[i64],
        last_http: &mut Option<Instant>,
    ) -> Result<(), FetchFail> {
        let known = match list {
            TgList::Genres => &self.genres,
            TgList::Publishers => &self.publishers,
        };
        let missing: Vec<i64> = ids
            .iter()
            .copied()
            .filter(|id| !known.contains_key(id))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let params = vec![
            ("apikey".to_string(), creds.thegamesdb_api_key.clone()),
            (
                "id".to_string(),
                missing
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        ];
        let url = format!(
            "https://api.thegamesdb.net/v1/{}?{}",
            list.path(),
            encode_query(&params)
        );
        let body = download_text(agent, &url, last_http)?;
        let names = parse_thegamesdb_names(&body, list)?;
        let slot = match list {
            TgList::Genres => &mut self.genres,
            TgList::Publishers => &mut self.publishers,
        };
        for (id, name) in names {
            slot.insert(id, name);
        }
        for id in missing {
            slot.entry(id).or_default();
        }
        Ok(())
    }
}

impl TgGame {
    fn metadata(&self, names: &TgNames) -> GameMetadata {
        GameMetadata {
            title: self.title.clone(),
            publisher: joined_names(&self.publisher_ids, &names.publishers),
            year: self.year,
            genre: joined_names(&self.genre_ids, &names.genres),
        }
    }
}

fn joined_names(ids: &[i64], names: &HashMap<i64, String>) -> Option<String> {
    let mut seen = Vec::new();
    for id in ids {
        let Some(name) = names
            .get(id)
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !seen.iter().any(|have: &String| have == name) {
            seen.push(name.to_string());
        }
    }
    if seen.is_empty() {
        None
    } else {
        Some(seen.join(", "))
    }
}

fn metadata_if_matched(game: &TgGame, names: &TgNames) -> Result<GameMetadata, FetchFail> {
    if !game.exact {
        Err(FetchFail::Failed("TheGamesDB has no game match".into()))
    } else {
        Ok(game.metadata(names))
    }
}

struct TgImage {
    type_name: String,
    side: String,
    filename: String,
}

pub fn parse_thegamesdb_images(
    body: &str,
) -> Result<(String, Vec<(String, String, String)>), FetchFail> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| FetchFail::Failed("TheGamesDB returned invalid JSON".into()))?;
    if let Some(message) = api_error_message(&value) {
        return Err(classify_provider_error(ScrapeProvider::TheGamesDb, message));
    }
    let base = value
        .pointer("/data/base_url/original")
        .and_then(|url| url.as_str())
        .ok_or_else(|| FetchFail::Failed("TheGamesDB image base URL missing".into()))?
        .to_string();
    let images = value
        .pointer("/data/images")
        .and_then(|images| images.as_object())
        .ok_or_else(|| FetchFail::Failed("TheGamesDB returned no images".into()))?;
    let mut out = Vec::new();
    for group in images.values() {
        let Some(group) = group.as_array() else {
            continue;
        };
        for image in group {
            let type_name = image.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let side = image.get("side").and_then(|v| v.as_str()).unwrap_or("");
            let filename = image.get("filename").and_then(|v| v.as_str()).unwrap_or("");
            if filename.is_empty() || filename.contains("..") {
                continue;
            }
            out.push((
                type_name.to_string(),
                side.to_string(),
                filename.to_string(),
            ));
        }
    }
    Ok((base, out))
}

pub fn pick_thegamesdb_filename(
    images: &[(String, String, String)],
    kind: MediaKind,
) -> Option<String> {
    let wanted = thegamesdb_media_type(kind)?;
    let matching: Vec<_> = images.iter().filter(|image| image.0 == wanted).collect();
    if kind == MediaKind::BoxArt {
        if let Some(front) = matching.iter().find(|image| image.1 == "front") {
            return Some(front.2.clone());
        }
    }
    matching.first().map(|image| image.2.clone())
}

pub fn join_base_url(base: &str, filename: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{filename}")
    } else {
        format!("{base}/{filename}")
    }
}

fn thegamesdb_media_type(kind: MediaKind) -> Option<&'static str> {
    match kind {
        MediaKind::BoxArt => Some("boxart"),
        MediaKind::Screenshot => Some("screenshot"),
        MediaKind::TitleScreen | MediaKind::Manual | MediaKind::Video => None,
    }
}

fn api_error_message(value: &serde_json::Value) -> Option<&str> {
    value
        .pointer("/data/message")
        .and_then(|message| message.as_str())
        .filter(|message| !message.is_empty())
        .or_else(|| {
            let status = value.get("status")?.as_str()?;
            (status != "Success").then_some(status)
        })
}

fn classify_provider_error(provider: ScrapeProvider, message: &str) -> FetchFail {
    let clean = public_detail(message);
    let lower = clean.to_ascii_lowercase();
    if lower.contains("quota")
        || lower.contains("rate")
        || lower.contains("limite")
        || lower.contains("limit")
        || lower.contains("thread")
        || lower.contains("closed")
        || lower.contains("trop de")
    {
        FetchFail::RateLimited(format!("{}: {clean}", provider.label()))
    } else {
        FetchFail::Failed(format!("{}: {clean}", provider.label()))
    }
}

fn public_detail(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if lower.contains("sspassword")
        || lower.contains("devpassword")
        || lower.contains("apikey")
        || lower.contains("api_key")
    {
        return "request failed".into();
    }
    let mut clean = String::new();
    for ch in message.chars().take(180) {
        if ch.is_control() {
            clean.push(' ');
        } else {
            clean.push(ch);
        }
    }
    clean.trim().to_string()
}

fn encode_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn encode_query(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{}={}", encode_component(key), encode_component(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn screenscraper_system(console_id: &str) -> Option<u32> {
    let console_id = console_id.to_ascii_lowercase();
    Some(match console_id.as_str() {
        "megadrive" | "genesis" | "megadrivejp" => 1,
        "mastersystem" => 2,
        "nes" | "famicom" | "fds" => 3,
        "snes" | "sfc" | "snesna" => 4,
        "gb" | "sgb" => 9,
        "gbc" => 10,
        "gba" => 12,
        "gc" => 13,
        "n64" | "n64dd" => 14,
        "nds" => 15,
        "wii" => 16,
        "n3ds" => 17,
        "sega32x" | "sega32xjp" | "sega32xna" => 19,
        "segacd" | "megacd" | "megacdjp" => 20,
        "gamegear" => 21,
        "saturn" | "saturnjp" => 22,
        "dreamcast" => 23,
        "ngp" => 25,
        "atari2600" => 26,
        "atarijaguar" => 27,
        "atarilynx" => 28,
        "3do" => 29,
        "pcengine" | "tg16" => 31,
        "xbox" => 32,
        // ES-DE folder `atari7800`, plus the short aliases ScreenScraper lists.
        "atari7800" | "a7800" | "7800" => 41,
        "wonderswan" => 45,
        "wonderswancolor" => 46,
        "psx" => 57,
        "ps2" => 58,
        "psp" => 61,
        "ngpc" => 82,
        "virtualboy" => 11,
        _ => return None,
    })
}

fn thegamesdb_platform(console_id: &str) -> Option<u32> {
    let console_id = console_id.to_ascii_lowercase();
    Some(match console_id.as_str() {
        "gc" => 2,
        "n64" | "n64dd" => 3,
        "gb" | "sgb" => 4,
        "gba" => 5,
        "snes" | "sfc" | "snesna" => 6,
        "nes" | "famicom" => 7,
        "nds" => 8,
        "wii" => 9,
        "psx" => 10,
        "ps2" => 11,
        "psp" => 13,
        "xbox" => 14,
        "dreamcast" => 16,
        "saturn" | "saturnjp" => 17,
        "genesis" => 18,
        "gamegear" => 20,
        "segacd" | "megacd" | "megacdjp" => 21,
        "atari2600" => 22,
        "neogeo" => 24,
        "3do" => 25,
        // https://thegamesdb.net/platform.php?id=27
        "atari7800" | "a7800" | "7800" => 27,
        "pcengine" | "tg16" => 34,
        "mastersystem" => 35,
        "megadrive" | "megadrivejp" => 36,
        "gbc" => 41,
        "virtualboy" => 4918,
        "ngp" => 4922,
        "ngpc" => 4923,
        "atarilynx" => 4924,
        "wonderswan" => 4925,
        "wonderswancolor" => 4926,
        "n3ds" => 4912,
        _ => return None,
    })
}

struct HttpGame {
    ss_game: Option<Result<SsGame, FetchFail>>,
    tgdb_game: Option<Result<TgGame, FetchFail>>,
    tgdb_images: Option<Result<(String, Vec<TgImage>), FetchFail>>,
}

impl HttpGame {
    fn new() -> Self {
        Self {
            ss_game: None,
            tgdb_game: None,
            tgdb_images: None,
        }
    }

    fn fetch_metadata(
        &mut self,
        agent: &ureq::Agent,
        creds: &ScraperCredentials,
        query: &ArtworkQuery,
        provider: ScrapeProvider,
        names: &mut TgNames,
        last_http: &mut Option<Instant>,
    ) -> Result<GameMetadata, FetchFail> {
        match provider {
            ScrapeProvider::ScreenScraper => Ok(self
                .ss_game(agent, creds, query, last_http)?
                .metadata
                .clone()),
            ScrapeProvider::TheGamesDb => {
                let game = self.tgdb_game(agent, creds, query, last_http)?.clone();
                if game.exact {
                    names.fill(agent, creds, &game, last_http)?;
                }
                metadata_if_matched(&game, names)
            }
        }
    }

    fn ss_game(
        &mut self,
        agent: &ureq::Agent,
        creds: &ScraperCredentials,
        query: &ArtworkQuery,
        last_http: &mut Option<Instant>,
    ) -> Result<&SsGame, FetchFail> {
        if self.ss_game.is_none() {
            let loaded = load_screenscraper(agent, creds, query, true, last_http).or_else(|err| {
                if matches!(err, FetchFail::RateLimited(_))
                    || screenscraper_system(&query.console_id).is_none()
                {
                    Err(err)
                } else {
                    load_screenscraper(agent, creds, query, false, last_http)
                }
            });
            self.ss_game = Some(loaded);
        }
        match self.ss_game.as_ref().expect("filled above") {
            Ok(game) => Ok(game),
            Err(err) => Err(err.clone()),
        }
    }

    fn tgdb_game(
        &mut self,
        agent: &ureq::Agent,
        creds: &ScraperCredentials,
        query: &ArtworkQuery,
        last_http: &mut Option<Instant>,
    ) -> Result<&TgGame, FetchFail> {
        if self.tgdb_game.is_none() {
            let with_platform = thegamesdb_platform(&query.console_id).is_some();
            let loaded =
                load_thegamesdb_id(agent, creds, query, with_platform, last_http).or_else(|err| {
                    if !with_platform || matches!(err, FetchFail::RateLimited(_)) {
                        Err(err)
                    } else {
                        load_thegamesdb_id(agent, creds, query, false, last_http)
                    }
                });
            self.tgdb_game = Some(loaded);
        }
        match self.tgdb_game.as_ref().expect("filled above") {
            Ok(game) => Ok(game),
            Err(err) => Err(err.clone()),
        }
    }

    fn fetch_kind(
        &mut self,
        agent: &ureq::Agent,
        creds: &ScraperCredentials,
        query: &ArtworkQuery,
        provider: ScrapeProvider,
        kind: MediaKind,
        last_http: &mut Option<Instant>,
    ) -> Result<Vec<u8>, FetchFail> {
        let url = match provider {
            ScrapeProvider::ScreenScraper => {
                self.screenscraper_url(agent, creds, query, kind, last_http)?
            }
            ScrapeProvider::TheGamesDb => {
                self.thegamesdb_url(agent, creds, query, kind, last_http)?
            }
        };
        if !url_allowed(&url) {
            return Err(FetchFail::Failed(format!(
                "{}: refused a non-image download",
                provider.label()
            )));
        }
        download_limited(agent, &url, last_http)
    }

    fn screenscraper_url(
        &mut self,
        agent: &ureq::Agent,
        creds: &ScraperCredentials,
        query: &ArtworkQuery,
        kind: MediaKind,
        last_http: &mut Option<Instant>,
    ) -> Result<String, FetchFail> {
        let game = self.ss_game(agent, creds, query, last_http)?;
        pick_screenscraper_url(&game.medias, kind).ok_or_else(|| {
            FetchFail::Failed(format!(
                "ScreenScraper: no {} for this game",
                kind_file_stem(kind).replace('_', " ")
            ))
        })
    }

    fn thegamesdb_url(
        &mut self,
        agent: &ureq::Agent,
        creds: &ScraperCredentials,
        query: &ArtworkQuery,
        kind: MediaKind,
        last_http: &mut Option<Instant>,
    ) -> Result<String, FetchFail> {
        let game_id = self.tgdb_game(agent, creds, query, last_http)?.id;
        if self.tgdb_images.is_none() {
            self.tgdb_images = Some(load_thegamesdb_images(agent, creds, game_id, last_http));
        }
        let (base, images) = match self.tgdb_images.as_ref().expect("filled above") {
            Ok(images) => images,
            Err(err) => return Err(err.clone()),
        };
        let tuples: Vec<_> = images
            .iter()
            .map(|image| {
                (
                    image.type_name.clone(),
                    image.side.clone(),
                    image.filename.clone(),
                )
            })
            .collect();
        let filename = pick_thegamesdb_filename(&tuples, kind).ok_or_else(|| {
            FetchFail::Failed(format!(
                "TheGamesDB: no {} for this game",
                kind_file_stem(kind).replace('_', " ")
            ))
        })?;
        Ok(join_base_url(base, &filename))
    }
}

fn load_screenscraper(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    query: &ArtworkQuery,
    with_system: bool,
    last_http: &mut Option<Instant>,
) -> Result<SsGame, FetchFail> {
    let mut params = screenscraper_params(creds, query);
    if !with_system {
        params.retain(|(key, _)| key != "systemeid");
    }
    let url = format!(
        "https://www.screenscraper.fr/api2/jeuInfos.php?{}",
        encode_query(&params)
    );
    let body = download_text(agent, &url, last_http)?;
    parse_screenscraper_game(&body)
}

fn load_thegamesdb_id(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    query: &ArtworkQuery,
    with_platform: bool,
    last_http: &mut Option<Instant>,
) -> Result<TgGame, FetchFail> {
    let mut params = vec![
        ("apikey".to_string(), creds.thegamesdb_api_key.clone()),
        ("name".to_string(), query.title.clone()),
        ("fields".to_string(), "genres,publishers".to_string()),
    ];
    if with_platform {
        if let Some(platform) = thegamesdb_platform(&query.console_id) {
            params.push(("filter[platform]".to_string(), platform.to_string()));
        }
    }
    let url = format!(
        "https://api.thegamesdb.net/v1/Games/ByGameName?{}",
        encode_query(&params)
    );
    let body = download_text(agent, &url, last_http)?;
    parse_thegamesdb_game(&body, &query.title)
}

fn load_thegamesdb_images(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    game_id: i64,
    last_http: &mut Option<Instant>,
) -> Result<(String, Vec<TgImage>), FetchFail> {
    let params = vec![
        ("apikey".to_string(), creds.thegamesdb_api_key.clone()),
        ("games_id".to_string(), game_id.to_string()),
        ("filter[type]".to_string(), "boxart,screenshot".to_string()),
    ];
    let url = format!(
        "https://api.thegamesdb.net/v1/Games/Images?{}",
        encode_query(&params)
    );
    let body = download_text(agent, &url, last_http)?;
    let (base, images) = parse_thegamesdb_images(&body)?;
    Ok((
        base,
        images
            .into_iter()
            .map(|(type_name, side, filename)| TgImage {
                type_name,
                side,
                filename,
            })
            .collect(),
    ))
}

fn download_text(
    agent: &ureq::Agent,
    url: &str,
    last_http: &mut Option<Instant>,
) -> Result<String, FetchFail> {
    let bytes = download_limited(agent, url, last_http)?;
    String::from_utf8(bytes).map_err(|_| FetchFail::Failed("response was not text".into()))
}

fn download_limited(
    agent: &ureq::Agent,
    url: &str,
    last_http: &mut Option<Instant>,
) -> Result<Vec<u8>, FetchFail> {
    pace(last_http);
    const MAX: u64 = 8 * 1024 * 1024;
    let response = match agent.get(url).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            let mut body = String::new();
            let _ = response.into_reader().take(2048).read_to_string(&mut body);
            return Err(http_status_error(code, &body));
        }
        Err(_) => return Err(FetchFail::Failed("network error".into())),
    };
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX)
        .read_to_end(&mut bytes)
        .map_err(|_| FetchFail::Failed("download failed".into()))?;
    Ok(bytes)
}

fn http_status_error(code: u16, body: &str) -> FetchFail {
    let detail = if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        value
            .pointer("/header/error")
            .and_then(|err| err.as_str())
            .or_else(|| api_error_message(&value))
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    let combined = if detail.is_empty() {
        format!("HTTP {code}")
    } else {
        format!("HTTP {code} {detail}")
    };
    let lower = combined.to_ascii_lowercase();
    let limited = code == 429
        || code == 423
        || lower.contains("quota")
        || lower.contains("rate")
        || lower.contains("limite")
        || lower.contains("limit")
        || lower.contains("thread")
        || lower.contains("closed");
    if limited {
        FetchFail::RateLimited(public_detail(&combined))
    } else {
        FetchFail::Failed(public_detail(&combined))
    }
}

fn pace(last_http: &mut Option<Instant>) {
    if let Some(previous) = *last_http {
        let wait = Duration::from_millis(1100).saturating_sub(previous.elapsed());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
    *last_http = Some(Instant::now());
}

fn log_fixture(query: &ArtworkQuery, kind: &str) {
    eprintln!(
        "scrape fixture: title=\"{}\" rom=\"{}\" console={} kind={}",
        one_line(&query.title),
        one_line(&query.rom_name),
        one_line(&query.console_id),
        kind
    );
}

fn fixture_bytes(dir: &Path, kind: MediaKind) -> Result<Vec<u8>, FetchFail> {
    let name = format!("{}.png", kind_file_stem(kind));
    fs::read(dir.join(name)).map_err(|_| {
        FetchFail::Failed(format!(
            "fixture image missing for {}",
            kind_file_stem(kind).replace('_', " ")
        ))
    })
}

fn fixture_metadata(dir: &Path) -> Result<GameMetadata, FetchFail> {
    let body = fs::read_to_string(dir.join("jeuInfos.json"))
        .map_err(|_| FetchFail::Failed("fixture metadata missing".into()))?;
    Ok(parse_screenscraper_game(&body)?.metadata)
}

struct MissingWork {
    kinds: Vec<MediaKind>,
    metadata: bool,
}

fn missing_work(game: &crate::types::Game, enabled: &[MediaKind]) -> Option<MissingWork> {
    let kinds = kinds_to_fetch(&present_kinds(&game.media), enabled);
    let metadata = game.metadata.is_none();
    if kinds.is_empty() && !metadata {
        None
    } else {
        Some(MissingWork { kinds, metadata })
    }
}

fn first_uncooled<T>(
    active: &[ScrapeProvider],
    cooled: &mut Vec<ScrapeProvider>,
    reasons: &mut Vec<String>,
    prefix: &str,
    mut fetch: impl FnMut(ScrapeProvider) -> Result<T, FetchFail>,
) -> Option<T> {
    let providers: Vec<_> = active
        .iter()
        .copied()
        .filter(|provider| !cooled.contains(provider))
        .collect();
    if providers.is_empty() {
        let names = cooled
            .iter()
            .map(|provider| provider.label())
            .collect::<Vec<_>>()
            .join(", ");
        push_reason(
            reasons,
            format!("{prefix}skipped after an earlier rate limit ({names})"),
        );
        return None;
    }
    let outcome = take_first_success(&providers, &mut fetch);
    for (provider, err) in &outcome.errors {
        push_reason(
            reasons,
            format!(
                "{prefix}{}: {}",
                provider.label(),
                fail_message(err.clone())
            ),
        );
        if matches!(err, FetchFail::RateLimited(_)) && !cooled.contains(provider) {
            cooled.push(*provider);
        }
    }
    outcome.success.map(|(_, value)| value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapeCandidate {
    pub provider: ScrapeProvider,
    pub provider_game_id: String,
    pub title: String,
    pub system: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameSearch {
    pub candidates: Vec<ScrapeCandidate>,
    pub errors: Vec<String>,
}

const SEARCH_LIMIT: usize = 30;

pub fn screenscraper_search_params(
    creds: &ScraperCredentials,
    query: &str,
) -> Vec<(String, String)> {
    vec![
        ("devid".to_string(), crate::softname::DEVID.to_string()),
        (
            "devpassword".to_string(),
            crate::softname::DEVPASSWORD.to_string(),
        ),
        (
            "softname".to_string(),
            crate::softname::SOFTNAME.to_string(),
        ),
        ("output".to_string(), "json".to_string()),
        ("ssid".to_string(), creds.screenscraper_user.clone()),
        (
            "sspassword".to_string(),
            creds.screenscraper_password.clone(),
        ),
        ("recherche".to_string(), query.to_string()),
    ]
}

pub fn screenscraper_game_params(
    creds: &ScraperCredentials,
    game_id: &str,
) -> Vec<(String, String)> {
    vec![
        ("devid".to_string(), crate::softname::DEVID.to_string()),
        (
            "devpassword".to_string(),
            crate::softname::DEVPASSWORD.to_string(),
        ),
        (
            "softname".to_string(),
            crate::softname::SOFTNAME.to_string(),
        ),
        ("output".to_string(), "json".to_string()),
        ("ssid".to_string(), creds.screenscraper_user.clone()),
        (
            "sspassword".to_string(),
            creds.screenscraper_password.clone(),
        ),
        ("gameid".to_string(), game_id.to_string()),
    ]
}

pub fn thegamesdb_search_params(
    creds: &ScraperCredentials,
    query: &str,
    console_id: &str,
) -> Vec<(String, String)> {
    let mut params = vec![
        ("apikey".to_string(), creds.thegamesdb_api_key.clone()),
        ("name".to_string(), query.to_string()),
        ("include".to_string(), "platform".to_string()),
        ("fields".to_string(), "genres,publishers".to_string()),
    ];
    if let Some(platform) = thegamesdb_platform(console_id) {
        params.push(("filter[platform]".to_string(), platform.to_string()));
    }
    params
}

pub fn parse_screenscraper_search(body: &str) -> Result<Vec<ScrapeCandidate>, FetchFail> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| FetchFail::Failed("ScreenScraper returned invalid JSON".into()))?;
    if let Some(error) = value
        .get("header")
        .and_then(|header| header.get("success"))
        .and_then(|flag| flag.as_str())
        .filter(|flag| *flag != "true")
        .and(value.pointer("/header/error").and_then(|err| err.as_str()))
    {
        return Err(classify_provider_error(
            ScrapeProvider::ScreenScraper,
            error,
        ));
    }
    let mut out = Vec::new();
    for jeu in json_games(value.pointer("/response/jeux"), "id") {
        let Some(provider_game_id) = jeu.get("id").and_then(json_id) else {
            continue;
        };
        let title = screenscraper_title(&jeu);
        if title.is_empty() {
            continue;
        }
        let system = jeu
            .pointer("/systeme/text")
            .and_then(|text| text.as_str())
            .unwrap_or("")
            .to_string();
        out.push(ScrapeCandidate {
            provider: ScrapeProvider::ScreenScraper,
            provider_game_id,
            title,
            system,
        });
        if out.len() == SEARCH_LIMIT {
            break;
        }
    }
    Ok(out)
}

fn screenscraper_title(jeu: &serde_json::Value) -> String {
    let noms = json_games(jeu.get("noms"), "text");
    const REGIONS: &[&str] = &["wor", "us", "eu", "ss", "jp"];
    for region in REGIONS {
        if let Some(name) = noms.iter().find_map(|nom| {
            let matches = nom.get("region").and_then(|value| value.as_str()) == Some(*region);
            matches
                .then(|| nom.get("text").and_then(|text| text.as_str()))
                .flatten()
                .filter(|text| !text.is_empty())
        }) {
            return name.to_string();
        }
    }
    noms.iter()
        .find_map(|nom| {
            nom.get("text")
                .and_then(|text| text.as_str())
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

pub fn parse_thegamesdb_search(body: &str) -> Result<Vec<ScrapeCandidate>, FetchFail> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| FetchFail::Failed("TheGamesDB returned invalid JSON".into()))?;
    if let Some(message) = api_error_message(&value) {
        return Err(classify_provider_error(ScrapeProvider::TheGamesDb, message));
    }
    let mut out = Vec::new();
    for game in json_games(value.pointer("/data/games"), "game_title") {
        let Some(provider_game_id) = game.get("id").and_then(json_id) else {
            continue;
        };
        let title = game
            .get("game_title")
            .and_then(|title| title.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        let system = game
            .get("platform")
            .and_then(json_id)
            .map(|id| platform_name(&value, &id))
            .unwrap_or_default();
        out.push(ScrapeCandidate {
            provider: ScrapeProvider::TheGamesDb,
            provider_game_id,
            title,
            system,
        });
        if out.len() == SEARCH_LIMIT {
            break;
        }
    }
    Ok(out)
}

fn platform_name(value: &serde_json::Value, platform_id: &str) -> String {
    if !platform_id.chars().all(|ch| ch.is_ascii_digit()) {
        return String::new();
    }
    value
        .pointer(&format!("/include/platform/data/{platform_id}/name"))
        .and_then(|name| name.as_str())
        .unwrap_or("")
        .to_string()
}

fn json_id(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        let text = text.trim();
        return (!text.is_empty()).then(|| text.to_string());
    }
    value.as_i64().map(|id| id.to_string())
}

/// An array of objects, or one object that carries `marker`.
fn json_games(value: Option<&serde_json::Value>, marker: &str) -> Vec<serde_json::Value> {
    match value {
        Some(serde_json::Value::Array(items)) => items.clone(),
        Some(value @ serde_json::Value::Object(_)) if value.get(marker).is_some() => {
            vec![value.clone()]
        }
        _ => Vec::new(),
    }
}

pub fn spawn_name_search(
    query: String,
    console_id: String,
    scraper: ScraperConfig,
    fixtures: Option<PathBuf>,
) -> std::sync::mpsc::Receiver<NameSearch> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = run_name_search(query, console_id, scraper, fixtures);
        let _ = tx.send(outcome);
    });
    rx
}

fn run_name_search(
    query: String,
    console_id: String,
    scraper: ScraperConfig,
    fixtures: Option<PathBuf>,
) -> NameSearch {
    let query = query.trim().to_string();
    if query.is_empty() {
        return NameSearch {
            candidates: Vec::new(),
            errors: vec!["Enter a name to search.".into()],
        };
    }
    if fixtures.is_some() {
        return NameSearch {
            candidates: fixture_candidates(&query),
            errors: Vec::new(),
        };
    }
    let ordered = provider_order(&scraper.providers);
    let active = ready_providers(&scraper.credentials, &ordered);
    if active.is_empty() {
        return NameSearch {
            candidates: Vec::new(),
            errors: credential_blocks(&scraper.credentials, &ordered)
                .into_iter()
                .map(str::to_string)
                .collect(),
        };
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(25))
        .build();
    let mut last_http = None;
    let mut candidates = Vec::new();
    let mut errors = Vec::new();
    for provider in active {
        match search_provider(
            &agent,
            &scraper.credentials,
            provider,
            &query,
            &console_id,
            &mut last_http,
        ) {
            Ok(found) => candidates.extend(found),
            Err(err) => errors.push(fail_message(err)),
        }
    }
    NameSearch { candidates, errors }
}

fn fail_message(err: FetchFail) -> String {
    match err {
        FetchFail::Failed(message) | FetchFail::RateLimited(message) => message,
    }
}

fn fixture_candidates(query: &str) -> Vec<ScrapeCandidate> {
    vec![
        ScrapeCandidate {
            provider: ScrapeProvider::ScreenScraper,
            provider_game_id: "1".into(),
            title: query.to_string(),
            system: "Super Nintendo".into(),
        },
        ScrapeCandidate {
            provider: ScrapeProvider::TheGamesDb,
            provider_game_id: "2".into(),
            title: format!("{query} DX"),
            system: "Super Nintendo".into(),
        },
    ]
}

fn search_provider(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    provider: ScrapeProvider,
    query: &str,
    console_id: &str,
    last_http: &mut Option<Instant>,
) -> Result<Vec<ScrapeCandidate>, FetchFail> {
    match provider {
        ScrapeProvider::ScreenScraper => {
            let mut params = screenscraper_search_params(creds, query);
            if let Some(system_id) = screenscraper_system(console_id) {
                params.push(("systemeid".to_string(), system_id.to_string()));
            }
            let url = format!(
                "https://www.screenscraper.fr/api2/jeuRecherche.php?{}",
                encode_query(&params)
            );
            let body = download_text(agent, &url, last_http)?;
            parse_screenscraper_search(&body)
        }
        ScrapeProvider::TheGamesDb => {
            let params = thegamesdb_search_params(creds, query, console_id);
            let url = format!(
                "https://api.thegamesdb.net/v1/Games/ByGameName?{}",
                encode_query(&params)
            );
            let body = download_text(agent, &url, last_http)?;
            parse_thegamesdb_search(&body)
        }
    }
}

pub fn spawn_apply_candidate(
    game: crate::types::Game,
    candidate: ScrapeCandidate,
    scraper: ScraperConfig,
    fixtures: Option<PathBuf>,
) -> std::sync::mpsc::Receiver<ScrapeUpdate> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let summary = run_apply(game, candidate, scraper, fixtures, &tx);
        let _ = tx.send(ScrapeUpdate::Done(summary));
    });
    rx
}

/// Box art and screenshot for one chosen hit. Replaces cached files for those
/// kinds. Bulk scrape does not call this.
fn run_apply(
    game: crate::types::Game,
    candidate: ScrapeCandidate,
    scraper: ScraperConfig,
    fixtures: Option<PathBuf>,
    tx: &std::sync::mpsc::Sender<ScrapeUpdate>,
) -> String {
    let kinds = scraper.enabled_kinds();
    let root = match media_root() {
        Ok(root) => root,
        Err(err) => return format!("Could not create the artwork cache: {err}"),
    };
    let _ = tx.send(ScrapeUpdate::Status(format!(
        "Scraping {}…",
        game.display_title()
    )));
    let source = if fixtures.is_some() {
        Source::Local
    } else {
        candidate.provider.source()
    };
    let loaded = if let Some(dir) = &fixtures {
        CandidateLoad {
            images: Ok(kinds
                .iter()
                .map(|kind| (*kind, fixture_bytes(dir, *kind)))
                .collect()),
            metadata: fixture_metadata(dir),
        }
    } else if let Some(reason) = scraper.credentials.block_reason(candidate.provider) {
        return reason.to_string();
    } else {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(25))
            .build();
        let mut last_http = None;
        load_candidate(
            &agent,
            &scraper.credentials,
            &candidate,
            &kinds,
            &mut last_http,
        )
    };
    if let (Err(err), Err(_)) = (&loaded.images, &loaded.metadata) {
        return fail_message(err.clone());
    }
    let mut saved = 0usize;
    let mut failed = 0usize;
    match loaded.images {
        Ok(images) => {
            for (kind, bytes) in images {
                match bytes {
                    Ok(bytes) => {
                        match write_cached_image(&root, &game.console, &game.id, kind, &bytes) {
                            Ok(path) => {
                                saved += 1;
                                let _ = tx.send(ScrapeUpdate::Saved {
                                    game_id: game.id.clone(),
                                    media: Media { kind, path, source },
                                });
                            }
                            Err(err) => {
                                failed += 1;
                                let _ = tx.send(ScrapeUpdate::Status(err));
                            }
                        }
                    }
                    Err(_) => failed += 1,
                }
            }
        }
        Err(_) => failed += 1,
    }
    match loaded.metadata {
        Ok(metadata) => {
            saved += 1;
            let _ = tx.send(ScrapeUpdate::Metadata {
                game_id: game.id.clone(),
                metadata,
            });
        }
        Err(_) => failed += 1,
    }
    let mut summary = format!("Scrape finished: {saved} saved, {failed} failed.");
    if fixtures.is_some() {
        summary.push_str(" Used local fixtures; no network requests were made.");
    }
    summary
}

struct CandidateLoad {
    images: Result<Vec<(MediaKind, Result<Vec<u8>, FetchFail>)>, FetchFail>,
    metadata: Result<GameMetadata, FetchFail>,
}

fn load_candidate(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    candidate: &ScrapeCandidate,
    kinds: &[MediaKind],
    last_http: &mut Option<Instant>,
) -> CandidateLoad {
    match candidate.provider {
        ScrapeProvider::ScreenScraper => {
            match load_screenscraper_by_id(agent, creds, &candidate.provider_game_id, last_http) {
                Ok(game) => CandidateLoad {
                    images: Ok(kind_bytes(kinds, |kind| {
                        match pick_screenscraper_url(&game.medias, kind) {
                            Some(url) if url_allowed(&url) => {
                                download_limited(agent, &url, last_http)
                            }
                            Some(_) => Err(FetchFail::Failed(
                                "ScreenScraper: refused a non-image download".into(),
                            )),
                            None => Err(FetchFail::Failed(format!(
                                "ScreenScraper: no {} for this game",
                                kind_file_stem(kind).replace('_', " ")
                            ))),
                        }
                    })),
                    metadata: Ok(game.metadata),
                },
                Err(err) => CandidateLoad {
                    images: Err(err.clone()),
                    metadata: Err(err),
                },
            }
        }
        ScrapeProvider::TheGamesDb => {
            let game_id = match candidate.provider_game_id.parse::<i64>() {
                Ok(id) => id,
                Err(_) => {
                    let err = FetchFail::Failed("TheGamesDB game id was not a number.".into());
                    return CandidateLoad {
                        images: Err(err.clone()),
                        metadata: Err(err),
                    };
                }
            };
            let metadata = match load_thegamesdb_game_by_id(agent, creds, game_id, last_http) {
                Ok(game) => {
                    let mut names = TgNames::default();
                    match names.fill(agent, creds, &game, last_http) {
                        Ok(()) => Ok(game.metadata(&names)),
                        Err(err) => Err(err),
                    }
                }
                Err(err) => Err(err),
            };
            let images = if kinds.is_empty() {
                Ok(Vec::new())
            } else {
                match load_thegamesdb_images(agent, creds, game_id, last_http) {
                    Ok((base, images)) => {
                        let tuples: Vec<_> = images
                            .iter()
                            .map(|image| {
                                (
                                    image.type_name.clone(),
                                    image.side.clone(),
                                    image.filename.clone(),
                                )
                            })
                            .collect();
                        Ok(kind_bytes(kinds, |kind| {
                            match pick_thegamesdb_filename(&tuples, kind) {
                                Some(filename) => {
                                    let url = join_base_url(&base, &filename);
                                    if url_allowed(&url) {
                                        download_limited(agent, &url, last_http)
                                    } else {
                                        Err(FetchFail::Failed(
                                            "TheGamesDB: refused a non-image download".into(),
                                        ))
                                    }
                                }
                                None => Err(FetchFail::Failed(format!(
                                    "TheGamesDB: no {} for this game",
                                    kind_file_stem(kind).replace('_', " ")
                                ))),
                            }
                        }))
                    }
                    Err(err) => Err(err),
                }
            };
            CandidateLoad { images, metadata }
        }
    }
}

fn kind_bytes(
    kinds: &[MediaKind],
    mut fetch: impl FnMut(MediaKind) -> Result<Vec<u8>, FetchFail>,
) -> Vec<(MediaKind, Result<Vec<u8>, FetchFail>)> {
    kinds
        .iter()
        .copied()
        .map(|kind| (kind, fetch(kind)))
        .collect()
}

fn load_screenscraper_by_id(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    game_id: &str,
    last_http: &mut Option<Instant>,
) -> Result<SsGame, FetchFail> {
    let params = screenscraper_game_params(creds, game_id);
    let url = format!(
        "https://www.screenscraper.fr/api2/jeuInfos.php?{}",
        encode_query(&params)
    );
    let body = download_text(agent, &url, last_http)?;
    parse_screenscraper_game(&body)
}

fn load_thegamesdb_game_by_id(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    game_id: i64,
    last_http: &mut Option<Instant>,
) -> Result<TgGame, FetchFail> {
    let params = vec![
        ("apikey".to_string(), creds.thegamesdb_api_key.clone()),
        ("id".to_string(), game_id.to_string()),
        ("fields".to_string(), "genres,publishers".to_string()),
    ];
    let url = format!(
        "https://api.thegamesdb.net/v1/Games/ByGameID?{}",
        encode_query(&params)
    );
    let body = download_text(agent, &url, last_http)?;
    parse_thegamesdb_game_by_id(&body, game_id)
}

#[derive(Debug)]
pub enum ScrapeUpdate {
    Status(String),
    Saved {
        game_id: String,
        media: Media,
    },
    Metadata {
        game_id: String,
        metadata: GameMetadata,
    },
    Done(String),
}

pub fn spawn_scrape(
    games: Vec<crate::types::Game>,
    scraper: ScraperConfig,
    fixtures: Option<PathBuf>,
) -> std::sync::mpsc::Receiver<ScrapeUpdate> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let summary = run_scrape(games, scraper, fixtures, &tx);
        let _ = tx.send(ScrapeUpdate::Done(summary));
    });
    rx
}

/// Games handed to one background scrape.
#[derive(Debug)]
pub struct ScrapeBatch {
    pub console_id: String,
    pub games: Vec<crate::types::Game>,
}

#[derive(Debug)]
pub enum Admit {
    /// Nothing else is running. Start this batch now.
    Start(ScrapeBatch),
    /// It will run after the scrape already in flight.
    Queued,
    /// This system already has a scrape running or waiting.
    Busy,
}

/// One artwork scrape at a time. A second Scrape Missing for the same system
/// waits instead of running beside the first. A scan can still append games
/// the in-flight scrape did not have.
#[derive(Debug, Default)]
pub struct ScrapeRuns {
    active: Option<String>,
    queued: VecDeque<ScrapeBatch>,
}

impl ScrapeRuns {
    pub fn tracks(&self, console_id: &str) -> bool {
        self.active.as_deref() == Some(console_id)
            || self
                .queued
                .iter()
                .any(|batch| batch.console_id == console_id)
    }

    /// Scrape Missing for one system. A system already running or waiting is [`Admit::Busy`].
    pub fn request_missing(&mut self, console_id: &str, games: Vec<crate::types::Game>) -> Admit {
        if self.tracks(console_id) {
            return Admit::Busy;
        }
        self.enqueue(console_id, games)
    }

    /// Games a scan just added. Merged into a waiting batch for that system,
    /// or run after the one already scraping it.
    pub fn request_added(&mut self, console_id: &str, games: Vec<crate::types::Game>) -> Admit {
        if games.is_empty() {
            return Admit::Busy;
        }
        if let Some(batch) = self
            .queued
            .iter_mut()
            .find(|batch| batch.console_id == console_id)
        {
            push_new_games(&mut batch.games, games);
            return Admit::Queued;
        }
        if self.active.as_deref() == Some(console_id) {
            self.queued.push_back(ScrapeBatch {
                console_id: console_id.to_string(),
                games,
            });
            return Admit::Queued;
        }
        self.enqueue(console_id, games)
    }

    /// One game from the scrape dialog. Does not queue behind another scrape.
    pub fn request_one(&mut self, console_id: &str) -> bool {
        if self.active.is_some() || !self.queued.is_empty() {
            return false;
        }
        self.active = Some(console_id.to_string());
        true
    }

    pub fn finish(&mut self) -> Option<ScrapeBatch> {
        self.active = None;
        let next = self.queued.pop_front()?;
        self.active = Some(next.console_id.clone());
        Some(next)
    }

    pub fn clear(&mut self) {
        self.active = None;
        self.queued.clear();
    }

    fn enqueue(&mut self, console_id: &str, games: Vec<crate::types::Game>) -> Admit {
        let batch = ScrapeBatch {
            console_id: console_id.to_string(),
            games,
        };
        if self.active.is_some() {
            self.queued.push_back(batch);
            Admit::Queued
        } else {
            self.active = Some(batch.console_id.clone());
            Admit::Start(batch)
        }
    }
}

fn push_new_games(into: &mut Vec<crate::types::Game>, extra: Vec<crate::types::Game>) {
    for game in extra {
        if !into.iter().any(|have| have.id == game.id) {
            into.push(game);
        }
    }
}

pub fn fixture_dir_from_env() -> Option<PathBuf> {
    std::env::var_os("RETROMARCHY_SCRAPER_FIXTURES")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

fn push_reason(reasons: &mut Vec<String>, reason: String) {
    if !reasons.iter().any(|have| have == &reason) {
        reasons.push(reason);
    }
}

/// One stderr line for a game counted as failed. Also appends to an existing
/// `*.log` file in the retromarchy data or cache directory. Does not create one.
fn log_scrape_failure(query: &ArtworkQuery, reasons: &[String]) {
    let line = scrape_failure_line(query, reasons);
    eprintln!("{line}");
    for path in existing_scrape_logs() {
        append_log_line(&path, &line);
    }
}

fn scrape_failure_line(query: &ArtworkQuery, reasons: &[String]) -> String {
    let system = match screenscraper_system(&query.console_id) {
        Some(id) => id.to_string(),
        None => "none".to_string(),
    };
    let platform = match thegamesdb_platform(&query.console_id) {
        Some(id) => id.to_string(),
        None => "none".to_string(),
    };
    let crc = match query.crc32 {
        Some(crc) => format!("{crc:08X}"),
        None => "none".to_string(),
    };
    let size = match query.rom_bytes {
        Some(size) => size.to_string(),
        None => "none".to_string(),
    };
    let detail = if reasons.is_empty() {
        "no artwork".to_string()
    } else {
        reasons
            .iter()
            .map(|reason| public_detail(reason))
            .collect::<Vec<_>>()
            .join("; ")
    };
    scrub_secrets(format!(
        "scrape failed: title=\"{}\" rom=\"{}\" console={} screenscraper_system={} thegamesdb_platform={} crc={} size={} header={} {detail}",
        one_line(&query.title),
        one_line(&query.rom_name),
        one_line(&query.console_id),
        system,
        platform,
        crc,
        size,
        query.header_bytes,
    ))
}

fn scrub_secrets(mut line: String) -> String {
    for secret in [crate::softname::DEVPASSWORD, crate::softname::DEVID] {
        if !secret.is_empty() {
            line = line.replace(secret, "redacted");
        }
    }
    line
}

fn one_line(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars().take(180) {
        if ch.is_control() || ch == '"' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn existing_scrape_logs() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(xdg) = xdg::BaseDirectories::with_prefix("retromarchy") else {
        return files;
    };
    for dir in [xdg.get_data_home(), xdg.get_cache_home()] {
        collect_log_files(&dir, &mut files);
    }
    files.sort();
    files
}

fn collect_log_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_log = path.extension().and_then(|ext| ext.to_str()) == Some("log");
        if path.is_file() && is_log {
            files.push(path);
        }
    }
}

fn append_log_line(path: &Path, line: &str) {
    if !path.is_file() {
        return;
    }
    let Ok(mut file) = fs::OpenOptions::new().append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

fn run_scrape(
    games: Vec<crate::types::Game>,
    scraper: ScraperConfig,
    fixtures: Option<PathBuf>,
    tx: &std::sync::mpsc::Sender<ScrapeUpdate>,
) -> String {
    let enabled = scraper.enabled_kinds();
    if games.is_empty() {
        return "No games to scrape.".into();
    }
    let ordered = provider_order(&scraper.providers);
    if ordered.is_empty() && fixtures.is_none() {
        return "Enable ScreenScraper or TheGamesDB in Scraper settings.".into();
    }
    let blocked = credential_blocks(&scraper.credentials, &ordered);
    let active = if fixtures.is_some() {
        ordered.clone()
    } else {
        ready_providers(&scraper.credentials, &ordered)
    };
    if active.is_empty() && fixtures.is_none() {
        return blocked.join(" ");
    }

    let root = match media_root() {
        Ok(root) => root,
        Err(err) => return format!("Could not create the artwork cache: {err}"),
    };
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(25))
        .build();
    let mut last_http = None;
    let mut saved = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut cooled = Vec::new();
    let mut names = TgNames::default();

    for (index, game) in games.iter().enumerate() {
        let _ = tx.send(ScrapeUpdate::Status(format!(
            "Scraping {}/{}: {}",
            index + 1,
            games.len(),
            game.display_title()
        )));
        let Some(work) = missing_work(game, &enabled) else {
            skipped += 1;
            continue;
        };
        let query = ArtworkQuery::from_game(game);
        let mut http = HttpGame::new();
        let mut game_failed = false;
        let mut reasons = Vec::new();
        for kind in work.kinds {
            if fixtures.is_some() {
                log_fixture(&query, kind_file_stem(kind));
            }
            let found = first_uncooled(&active, &mut cooled, &mut reasons, "", |provider| {
                let bytes = if let Some(dir) = &fixtures {
                    fixture_bytes(dir, kind)?
                } else {
                    http.fetch_kind(
                        &agent,
                        &scraper.credentials,
                        &query,
                        provider,
                        kind,
                        &mut last_http,
                    )?
                };
                let source = if fixtures.is_some() {
                    Source::Local
                } else {
                    provider.source()
                };
                let path = write_cached_image(&root, &game.console, &game.id, kind, &bytes)
                    .map_err(FetchFail::Failed)?;
                Ok(Media { kind, path, source })
            });
            if let Some(media) = found {
                saved += 1;
                let _ = tx.send(ScrapeUpdate::Saved {
                    game_id: game.id.clone(),
                    media,
                });
            } else {
                game_failed = true;
            }
        }
        if work.metadata {
            if fixtures.is_some() {
                log_fixture(&query, "metadata");
            }
            let found = first_uncooled(
                &active,
                &mut cooled,
                &mut reasons,
                "metadata ",
                |provider| match &fixtures {
                    Some(dir) => fixture_metadata(dir),
                    None => http.fetch_metadata(
                        &agent,
                        &scraper.credentials,
                        &query,
                        provider,
                        &mut names,
                        &mut last_http,
                    ),
                },
            );
            if let Some(metadata) = found {
                saved += 1;
                let _ = tx.send(ScrapeUpdate::Metadata {
                    game_id: game.id.clone(),
                    metadata,
                });
            } else {
                game_failed = true;
            }
        }
        if game_failed {
            failed += 1;
            log_scrape_failure(&query, &reasons);
        }
    }

    let mut summary =
        format!("Scrape finished: {saved} saved, {skipped} already complete, {failed} failed.");
    if fixtures.is_some() {
        summary.push_str(" Used local fixtures; no network requests were made.");
    }
    if !blocked.is_empty() && fixtures.is_none() {
        summary.push(' ');
        summary.push_str(&blocked.join(" "));
    }
    if !cooled.is_empty() {
        let names = cooled
            .iter()
            .map(|provider| provider.label())
            .collect::<Vec<_>>()
            .join(", ");
        summary.push_str(&format!(" Rate limited: {names}."));
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GameMetadata, ProviderEntry, Source};
    use std::fs;

    fn provider(id: ScrapeProvider, enabled: bool) -> ProviderEntry {
        ProviderEntry { id, enabled }
    }

    #[test]
    fn skips_kind_when_file_is_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("box.png");
        fs::write(&path, b"present").unwrap();
        let media = vec![Media {
            kind: MediaKind::BoxArt,
            path,
            source: Source::Local,
        }];
        let have = present_kinds(&media);
        assert_eq!(have, vec![MediaKind::BoxArt]);
        let fetch = kinds_to_fetch(&have, &ScraperConfig::default().enabled_kinds());
        assert_eq!(fetch, vec![MediaKind::Screenshot]);
    }

    #[test]
    fn missing_file_is_fetched_again() {
        let media = vec![Media {
            kind: MediaKind::BoxArt,
            path: PathBuf::from("/no/such/retromarchy-box.png"),
            source: Source::ScreenScraper,
        }];
        let have = present_kinds(&media);
        let fetch = kinds_to_fetch(&have, &[MediaKind::BoxArt]);
        assert_eq!(fetch, vec![MediaKind::BoxArt]);
    }

    #[test]
    fn disabled_kind_and_provider_are_skipped() {
        let mut config = ScraperConfig::default();
        config.box_art = false;
        config.providers = vec![
            provider(ScrapeProvider::ScreenScraper, false),
            provider(ScrapeProvider::TheGamesDb, true),
            provider(ScrapeProvider::TheGamesDb, true),
        ];
        let fetch = kinds_to_fetch(&[], &config.enabled_kinds());
        assert_eq!(fetch, vec![MediaKind::Screenshot]);
        assert_eq!(
            provider_order(&config.providers),
            vec![ScrapeProvider::TheGamesDb]
        );
    }

    #[test]
    fn provider_priority_keeps_user_order() {
        let providers = vec![
            provider(ScrapeProvider::TheGamesDb, true),
            provider(ScrapeProvider::ScreenScraper, true),
        ];
        assert_eq!(
            provider_order(&providers),
            vec![ScrapeProvider::TheGamesDb, ScrapeProvider::ScreenScraper]
        );
    }

    #[test]
    fn first_success_stops_the_chain() {
        let providers = [ScrapeProvider::ScreenScraper, ScrapeProvider::TheGamesDb];
        let mut calls = Vec::new();
        let outcome = take_first_success(&providers, |provider| {
            calls.push(provider);
            if provider == ScrapeProvider::ScreenScraper {
                Err(FetchFail::Failed("miss".into()))
            } else {
                Ok("image")
            }
        });
        assert_eq!(outcome.success.unwrap().0, ScrapeProvider::TheGamesDb);
        assert_eq!(calls, providers);
    }

    #[test]
    fn success_does_not_call_the_next_provider() {
        let providers = [ScrapeProvider::ScreenScraper, ScrapeProvider::TheGamesDb];
        let mut calls = Vec::new();
        let outcome = take_first_success(&providers, |provider| {
            calls.push(provider);
            Ok::<_, FetchFail>(())
        });
        assert!(outcome.success.is_some());
        assert_eq!(calls, vec![ScrapeProvider::ScreenScraper]);
        assert!(outcome.errors.is_empty());
    }

    #[test]
    fn rate_limit_still_tries_the_next_provider() {
        let providers = [ScrapeProvider::ScreenScraper, ScrapeProvider::TheGamesDb];
        let outcome = take_first_success(&providers, |provider| {
            if provider == ScrapeProvider::ScreenScraper {
                Err(FetchFail::RateLimited("slow down".into()))
            } else {
                Ok("image")
            }
        });
        assert_eq!(outcome.success.unwrap().1, "image");
        assert!(matches!(outcome.errors[0].1, FetchFail::RateLimited(_)));
    }

    #[test]
    fn missing_credentials_are_explained() {
        let creds = ScraperCredentials::default();
        let providers = [ScrapeProvider::ScreenScraper, ScrapeProvider::TheGamesDb];
        let blocked = credential_blocks(&creds, &providers);
        assert_eq!(blocked.len(), 2);
        assert!(blocked[0].contains("ScreenScraper"));
        assert!(blocked[1].contains("TheGamesDB"));
        assert!(ready_providers(&creds, &providers).is_empty());

        assert!(!blocked[0].to_ascii_lowercase().contains("developer"));
        assert!(blocked[0].contains("username"));
        assert!(blocked[0].contains("password"));

        let mut creds = ScraperCredentials::default();
        creds.screenscraper_user = "member".into();
        creds.screenscraper_password = "secret".into();
        assert!(creds.block_reason(ScrapeProvider::ScreenScraper).is_none());
        assert_eq!(
            ready_providers(&creds, &providers),
            vec![ScrapeProvider::ScreenScraper]
        );

        creds.thegamesdb_api_key = "secret-key".into();
        assert_eq!(
            ready_providers(&creds, &providers),
            vec![ScrapeProvider::ScreenScraper, ScrapeProvider::TheGamesDb]
        );
    }

    #[test]
    fn grid_falls_back_when_preferred_kind_is_missing() {
        let have = [MediaKind::Screenshot];
        assert_eq!(
            pick_kind(&have, GridArt::BoxArt),
            Some(MediaKind::Screenshot)
        );
        assert_eq!(pick_kind(&[], GridArt::BoxArt), None);
    }

    #[test]
    fn screenscraper_query_is_rom_only_and_hides_secrets_from_parser() {
        let creds = ScraperCredentials {
            screenscraper_user: "user".into(),
            screenscraper_password: "secret-pass".into(),
            thegamesdb_api_key: String::new(),
        };
        let query = ArtworkQuery {
            console_id: "snes".into(),
            title: "Chrono Trigger".into(),
            rom_name: "Chrono Trigger.sfc".into(),
            crc32: Some(0x50AB_C90A),
            rom_bytes: Some(1024),
            header_bytes: 0,
        };
        let params = screenscraper_params(&creds, &query);
        let value = |key: &str| {
            params
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, item)| item.as_str())
                .unwrap()
        };
        assert_eq!(value("softname"), crate::softname::SOFTNAME);
        assert_eq!(crate::softname::SOFTNAME, "retromarchy");
        assert_eq!(value("devid"), crate::softname::DEVID);
        assert_eq!(value("devpassword"), crate::softname::DEVPASSWORD);
        assert_eq!(value("ssid"), "user");
        assert_eq!(value("sspassword"), "secret-pass");
        let romtype = params.iter().find(|(key, _)| key == "romtype").unwrap();
        assert_eq!(romtype.1, "rom");
        assert!(params.iter().all(|(key, _)| key != "bios"));
        assert_eq!(
            params.iter().find(|(key, _)| key == "systemeid").unwrap().1,
            "4"
        );
        let fail =
            classify_provider_error(ScrapeProvider::ScreenScraper, "bad sspassword=secret-pass");
        let FetchFail::Failed(message) = fail else {
            panic!("expected failed");
        };
        assert!(!message.contains("secret-pass"));
    }

    #[test]
    fn screenscraper_parser_picks_region_and_ignores_manuals() {
        let body = r#"{
            "header": {"success": "true"},
            "response": {"jeu": {"medias": [
                {"type": "box-2D", "region": "jp", "url": "https://example.test/jp.png"},
                {"type": "box-2D", "region": "us", "url": "https://example.test/us.png"},
                {"type": "sstitle", "region": "wor", "url": "https://example.test/title.png"},
                {"type": "ss", "region": "eu", "url": "https://example.test/shot.png"},
                {"type": "manuel", "region": "us", "url": "https://example.test/manual.pdf"}
            ]}}
        }"#;
        let parsed = parse_screenscraper_game(body).unwrap();
        let medias = parsed.medias;
        assert_eq!(
            pick_screenscraper_url(&medias, MediaKind::BoxArt).as_deref(),
            Some("https://example.test/us.png")
        );
        assert!(pick_screenscraper_url(&medias, MediaKind::TitleScreen).is_none());
        assert_eq!(
            pick_screenscraper_url(&medias, MediaKind::Screenshot).as_deref(),
            Some("https://example.test/shot.png")
        );
        assert!(pick_screenscraper_url(&medias, MediaKind::Manual).is_none());
    }

    #[test]
    fn thegamesdb_parser_prefers_front_boxart() {
        let games = r#"{"status":"Success","data":{"games":[
            {"id": 9, "game_title": "Other"},
            {"id": 42, "game_title": "Chrono Trigger"}
        ]}}"#;
        assert_eq!(
            parse_thegamesdb_game_id(games, "chrono trigger").unwrap(),
            42
        );
        let images = r#"{
            "status": "Success",
            "data": {
                "base_url": {"original": "https://cdn.thegamesdb.net/images/original/"},
                "images": {"42": [
                    {"type": "boxart", "side": "back", "filename": "boxart/back/42-1.jpg"},
                    {"type": "boxart", "side": "front", "filename": "boxart/front/42-1.jpg"},
                    {"type": "titlescreen", "filename": "titlescreens/42-1.jpg"},
                    {"type": "screenshot", "filename": "screenshots/42-1.jpg"},
                    {"type": "fanart", "filename": "fanart/42-1.jpg"}
                ]}
            }
        }"#;
        let (base, parsed) = parse_thegamesdb_images(images).unwrap();
        let file = pick_thegamesdb_filename(&parsed, MediaKind::BoxArt).unwrap();
        assert_eq!(file, "boxart/front/42-1.jpg");
        assert_eq!(
            join_base_url(&base, &file),
            "https://cdn.thegamesdb.net/images/original/boxart/front/42-1.jpg"
        );
        assert!(pick_thegamesdb_filename(&parsed, MediaKind::Video).is_none());
    }

    #[test]
    fn rejects_archives_and_accepts_png() {
        assert!(image_extension(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A]).is_some());
        assert!(image_extension(b"PK\x03\x04rom").is_none());
        assert!(!url_allowed("https://example.test/game.zip"));
        assert!(!url_allowed("file:///tmp/game.png"));
        assert!(url_allowed(
            "https://www.screenscraper.fr/api2/mediaJeu.php?id=1"
        ));
    }

    #[test]
    fn cached_image_is_one_file_per_kind() {
        let dir = tempfile::tempdir().unwrap();
        let png = tiny_png();
        let path =
            write_cached_image(dir.path(), "snes", "abc", MediaKind::Screenshot, &png).unwrap();
        assert!(path.ends_with("snes/abc/screenshot.png"));
        assert_eq!(fs::read(&path).unwrap(), png);
        assert!(write_cached_image(
            dir.path(),
            "snes",
            "abc",
            MediaKind::Screenshot,
            b"not-an-image"
        )
        .is_err());
    }

    fn tiny_png() -> Vec<u8> {
        let raw = b"\x89PNG\r\n\x1a\n";
        raw.to_vec()
    }

    #[test]
    fn name_search_lists_hits_and_does_not_query_by_hash() {
        let creds = ScraperCredentials {
            screenscraper_user: "user".into(),
            screenscraper_password: "secret-pass".into(),
            thegamesdb_api_key: "key".into(),
        };
        let params = screenscraper_search_params(&creds, "Chrono Trigger");
        assert!(params
            .iter()
            .any(|(key, value)| key == "recherche" && value == "Chrono Trigger"));
        assert!(params
            .iter()
            .all(|(key, _)| key != "crc" && key != "romnom"));
        let game_params = screenscraper_game_params(&creds, "7708");
        assert!(game_params
            .iter()
            .any(|(key, value)| key == "gameid" && value == "7708"));
        assert!(game_params
            .iter()
            .all(|(key, _)| key != "crc" && key != "romnom"));
        let tg = thegamesdb_search_params(&creds, "Chrono Trigger", "snes");
        assert!(tg
            .iter()
            .any(|(key, value)| key == "name" && value == "Chrono Trigger"));
        assert!(tg
            .iter()
            .any(|(key, value)| key == "filter[platform]" && value == "6"));

        let ss = r#"{
            "header": {"success": "true"},
            "response": {"jeux": [
                {
                    "id": "3",
                    "noms": [
                        {"region": "jp", "text": "クロノ・トリガー"},
                        {"region": "us", "text": "Chrono Trigger"}
                    ],
                    "systeme": {"id": "4", "text": "Super Nintendo"}
                },
                {
                    "id": 9,
                    "noms": {"region": "eu", "text": "Chrono Trigger DX"},
                    "systeme": {"id": "4", "text": "Super Nintendo"}
                }
            ]}
        }"#;
        let hits = parse_screenscraper_search(ss).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].provider, ScrapeProvider::ScreenScraper);
        assert_eq!(hits[0].provider_game_id, "3");
        assert_eq!(hits[0].title, "Chrono Trigger");
        assert_eq!(hits[0].system, "Super Nintendo");
        assert_eq!(hits[1].title, "Chrono Trigger DX");
        assert!(
            parse_screenscraper_search(r#"{"header":{"success":"true"},"response":{}}"#)
                .unwrap()
                .is_empty()
        );

        let tg_body = r#"{
            "status": "Success",
            "data": {"games": [
                {"id": 111, "game_title": "Chrono Trigger", "platform": 6},
                {"id": 222, "game_title": "Radical Psycho Machine Racing", "platform": 6}
            ]},
            "include": {"platform": {"data": {"6": {"name": "Super Nintendo (SNES)"}}}}
        }"#;
        let hits = parse_thegamesdb_search(tg_body).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].provider_game_id, "111");
        assert_eq!(hits[0].system, "Super Nintendo (SNES)");
        assert_eq!(hits[1].title, "Radical Psycho Machine Racing");
        assert!(
            parse_thegamesdb_search(r#"{"status":"Success","data":{"games":[]}}"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cached_assets_delete_only_that_games_folder() {
        let root = tempfile::tempdir().unwrap();
        let game_dir = root.path().join("snes").join("abc");
        let other_dir = root.path().join("snes").join("other");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();
        fs::write(game_dir.join("box_art.png"), b"art").unwrap();
        fs::write(other_dir.join("box_art.png"), b"keep").unwrap();
        fs::write(root.path().join("snes").join("note.txt"), b"keep").unwrap();
        let rom_dir = tempfile::tempdir().unwrap();
        let rom = rom_dir.path().join("Chrono.sfc");
        fs::write(&rom, b"rom").unwrap();
        fs::write(rom_dir.path().join("other.sfc"), b"keep").unwrap();

        let game = crate::types::Game {
            id: "abc".into(),
            console: "snes".into(),
            rom: rom.clone(),
            file_title: "Chrono".into(),
            user_title: None,
            metadata: None,
            crc32: None,
            profile: None,
            media: Vec::new(),
            last_played: None,
            play_count: 0,
            play_time: 0,
            favorite: false,
        };
        delete_cached_assets(root.path(), &game).unwrap();
        assert!(!game_dir.exists());
        assert_eq!(fs::read(other_dir.join("box_art.png")).unwrap(), b"keep");
        assert_eq!(
            fs::read(root.path().join("snes").join("note.txt")).unwrap(),
            b"keep"
        );

        let mut escape = game.clone();
        escape.id = "..".into();
        delete_cached_assets(root.path(), &escape).unwrap();
        assert!(other_dir.join("box_art.png").is_file());

        delete_rom_file(&rom).unwrap();
        assert!(!rom.exists());
        assert!(rom_dir.path().join("other.sfc").is_file());
        delete_rom_file(rom_dir.path()).unwrap();
        assert!(rom_dir.path().is_dir());
    }

    fn creds() -> ScraperCredentials {
        ScraperCredentials {
            screenscraper_user: "user".into(),
            screenscraper_password: "secret-pass".into(),
            thegamesdb_api_key: "key".into(),
        }
    }

    fn param<'a>(params: &'a [(String, String)], key: &str) -> &'a str {
        params
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .unwrap()
    }

    fn dummy_game(rom: PathBuf, console: &str, crc32: u32) -> crate::types::Game {
        crate::types::Game {
            id: "asteroids".into(),
            console: console.into(),
            rom,
            file_title: "Asteroids".into(),
            user_title: None,
            metadata: None,
            crc32: Some(crc32),
            profile: None,
            media: Vec::new(),
            last_played: None,
            play_count: 0,
            play_time: 0,
            favorite: false,
        }
    }

    /// Headered `.a78` bytes: version byte, `ATARI7800` at offset 1, then a payload.
    fn headered_a78() -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[0] = 1;
        bytes[1..10].copy_from_slice(b"ATARI7800");
        bytes.extend_from_slice(b"ROM-BYTES");
        bytes
    }

    #[test]
    fn atari_7800_batch_query_skips_a78_header_and_sends_system_ids() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = headered_a78();
        let payload = b"ROM-BYTES";
        let rom = dir.path().join("Asteroids (USA).a78");
        fs::write(&rom, &bytes).unwrap();
        let file_crc = crc32fast::hash(&bytes);
        let payload_crc = crc32fast::hash(payload);
        let query = ArtworkQuery::from_game(&dummy_game(rom, "atari7800", file_crc));

        assert_eq!(query.rom_name, "Asteroids (USA).a78");
        assert_eq!(query.title, "Asteroids");
        assert_eq!(crate::scanner::clean_title("Asteroids (USA)"), "Asteroids");
        assert_eq!(query.header_bytes, 128);
        assert_eq!(query.rom_bytes, Some(payload.len() as u64));
        assert_eq!(query.crc32, Some(payload_crc));
        assert_ne!(query.crc32, Some(file_crc));

        let params = screenscraper_params(&creds(), &query);
        assert_eq!(param(&params, "systemeid"), "41");
        assert_eq!(param(&params, "romnom"), "Asteroids (USA).a78");
        assert_ne!(param(&params, "romnom"), "Asteroids");
        assert_eq!(param(&params, "crc"), format!("{payload_crc:08X}"));
        assert_eq!(param(&params, "romtaille"), payload.len().to_string());
        assert!(params.iter().all(|(key, _)| key != "gameid"));

        let raw_a78 = dir.path().join("Food Fight (USA).a78");
        fs::write(&raw_a78, b"NOHEADER").unwrap();
        let raw_crc = crc32fast::hash(b"NOHEADER");
        let raw_query = ArtworkQuery::from_game(&dummy_game(raw_a78, "atari7800", raw_crc));
        assert_eq!(raw_query.header_bytes, 0);
        assert_eq!(raw_query.crc32, Some(raw_crc));
        assert_eq!(raw_query.rom_bytes, Some(8));

        let padded = dir.path().join("Meltdown.a78");
        let padded_bytes = vec![0u8; 200];
        fs::write(&padded, &padded_bytes).unwrap();
        let padded_crc = crc32fast::hash(&padded_bytes);
        let padded_query = ArtworkQuery::from_game(&dummy_game(padded, "atari7800", padded_crc));
        assert_eq!(padded_query.header_bytes, 0);
        assert_eq!(padded_query.crc32, Some(padded_crc));
        assert_eq!(padded_query.rom_bytes, Some(200));

        let headered_bin = headered_a78();
        let bin = dir.path().join("Centipede.bin");
        fs::write(&bin, &headered_bin).unwrap();
        let bin_query =
            ArtworkQuery::from_game(&dummy_game(bin, "A7800", crc32fast::hash(&headered_bin)));
        let bin_params = screenscraper_params(&creds(), &bin_query);
        assert_eq!(param(&bin_params, "systemeid"), "41");
        assert_eq!(param(&bin_params, "romnom"), "Centipede.bin");
        assert_eq!(bin_query.header_bytes, 128);
        assert_eq!(param(&bin_params, "romtaille"), payload.len().to_string());
        assert_eq!(param(&bin_params, "crc"), format!("{payload_crc:08X}"));

        let plain = dir.path().join("Joust.bin");
        fs::write(&plain, b"RAW-BIN").unwrap();
        let plain_crc = crc32fast::hash(b"RAW-BIN");
        let plain_query = ArtworkQuery::from_game(&dummy_game(plain, "atari7800", plain_crc));
        assert_eq!(plain_query.header_bytes, 0);
        assert_eq!(plain_query.crc32, Some(plain_crc));
        assert_eq!(plain_query.rom_bytes, Some(7));

        let zip_bytes = b"not-a-real-zip";
        let zip = dir.path().join("Food Fight.zip");
        fs::write(&zip, zip_bytes).unwrap();
        let zip_query =
            ArtworkQuery::from_game(&dummy_game(zip, "7800", crc32fast::hash(zip_bytes)));
        let zip_params = screenscraper_params(&creds(), &zip_query);
        assert_eq!(zip_query.header_bytes, 0);
        assert_eq!(param(&zip_params, "systemeid"), "41");
        assert_eq!(param(&zip_params, "romnom"), "Food Fight.zip");
        assert_eq!(param(&zip_params, "romtaille"), zip_bytes.len().to_string());
        assert_eq!(
            param(&zip_params, "crc"),
            format!("{:08X}", crc32fast::hash(zip_bytes))
        );

        let lynx = dir.path().join("California Games.lnx");
        let mut lynx_bytes = vec![0u8; 64];
        lynx_bytes[0..4].copy_from_slice(b"LYNX");
        lynx_bytes.extend_from_slice(b"HANDHELD");
        fs::write(&lynx, &lynx_bytes).unwrap();
        let lynx_crc = crc32fast::hash(&lynx_bytes);
        let lynx_query = ArtworkQuery::from_game(&dummy_game(lynx, "atarilynx", lynx_crc));
        assert_eq!(lynx_query.header_bytes, 0);
        assert_eq!(lynx_query.crc32, Some(lynx_crc));
        assert_eq!(lynx_query.rom_bytes, Some(lynx_bytes.len() as u64));

        assert_eq!(screenscraper_system("atari7800"), Some(41));
        assert_eq!(screenscraper_system("a7800"), Some(41));
        assert_eq!(screenscraper_system("atarilynx"), Some(28));
        assert_eq!(thegamesdb_platform("atari7800"), Some(27));
        assert_eq!(thegamesdb_platform("a7800"), Some(27));
        assert_eq!(thegamesdb_platform("atarilynx"), Some(4924));

        let tg = thegamesdb_search_params(&creds(), "Asteroids", "atari7800");
        assert_eq!(param(&tg, "filter[platform]"), "27");
        assert_eq!(param(&tg, "name"), "Asteroids");

        let search = screenscraper_search_params(&creds(), "Asteroids");
        assert!(search
            .iter()
            .any(|(key, value)| key == "recherche" && value == "Asteroids"));
        assert!(search.iter().all(|(key, _)| {
            key != "crc"
                && key != "romnom"
                && key != "romtaille"
                && key != "systemeid"
                && key != "gameid"
        }));
        let game_params = screenscraper_game_params(&creds(), "1234");
        assert_eq!(param(&game_params, "gameid"), "1234");
        assert!(game_params
            .iter()
            .all(|(key, _)| key != "crc" && key != "romnom" && key != "systemeid"));
    }

    #[test]
    fn scrape_failure_line_names_the_rom_and_hides_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = headered_a78();
        let rom = dir.path().join("Asteroids (USA).a78");
        fs::write(&rom, &bytes).unwrap();
        let file_crc = crc32fast::hash(&bytes);
        let payload_crc = crc32fast::hash(b"ROM-BYTES");
        let query = ArtworkQuery::from_game(&dummy_game(rom, "atari7800", file_crc));
        let line = scrape_failure_line(
            &query,
            &[
                "ScreenScraper: HTTP 404 Jeu non trouve".into(),
                "ScreenScraper: bad sspassword=secret-pass".into(),
                format!("echo {}", crate::softname::DEVPASSWORD),
                format!("id {}", crate::softname::DEVID),
            ],
        );
        assert!(line.starts_with("scrape failed:"));
        assert!(line.contains("rom=\"Asteroids (USA).a78\""));
        assert!(line.contains("console=atari7800"));
        assert!(line.contains("screenscraper_system=41"));
        assert!(line.contains("thegamesdb_platform=27"));
        assert!(line.contains(&format!("crc={payload_crc:08X}")));
        assert!(line.contains("size=9"));
        assert!(line.contains("header=128"));
        assert!(!line.contains(&format!("crc={file_crc:08X}")));
        assert!(line.contains("HTTP 404 Jeu non trouve"));
        assert!(!line.contains("secret-pass"));
        assert!(!line.contains("sspassword"));
        assert!(!line.contains("devpassword"));
        assert!(!line.contains(crate::softname::DEVPASSWORD));
        assert!(!line.contains(crate::softname::DEVID));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn scrape_failure_appends_only_when_a_log_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("retromarchy.log");
        let missing = dir.path().join("absent.log");
        fs::write(&log, "boot\n").unwrap();
        let line = "scrape failed: rom=\"Asteroids (USA).a78\" screenscraper_system=41";
        append_log_line(&log, line);
        append_log_line(&missing, "scrape failed: should not create a file");
        let text = fs::read_to_string(&log).unwrap();
        assert_eq!(
            text,
            "boot\nscrape failed: rom=\"Asteroids (USA).a78\" screenscraper_system=41\n"
        );
        assert!(!missing.exists());
    }

    #[test]
    fn screenscraper_metadata_prefers_us_then_wor_then_eu() {
        let body = r#"{"header":{"success":"true"},"response":{"jeu":{"noms":[{"region":"jp","text":"クロノ・トリガー"},{"region":"eu","text":"Chrono Trigger EU"},{"region":"us","text":"Chrono Trigger"}],"editeur":{"id":"38","text":"Square"},"dates":[{"region":"us","text":"1995-08-22"},{"region":"jp","text":"1995-03-11"}],"genres":[{"id":"7","noms":[{"langue":"fr","text":"Jeu de rôles"},{"langue":"en","text":"Role Playing Game"}]}],"medias":[{"type":"box-2D","region":"us","url":"https://example.test/us.png"}]}}}"#;
        let game = parse_screenscraper_game(body).unwrap();
        assert_eq!(game.metadata.title.as_deref(), Some("Chrono Trigger"));
        assert_eq!(game.metadata.publisher.as_deref(), Some("Square"));
        assert_eq!(game.metadata.year, Some(1995));
        assert_eq!(game.metadata.genre.as_deref(), Some("Role Playing Game"));
        assert_eq!(
            pick_screenscraper_url(&game.medias, MediaKind::BoxArt).as_deref(),
            Some("https://example.test/us.png")
        );

        let wor = parse_screenscraper_game(
            r#"{"header":{"success":"true"},"response":{"jeu":{"noms":[{"region":"eu","text":"EU Name"},{"region":"wor","text":"World Name"}]}}}"#,
        )
        .unwrap();
        assert_eq!(wor.metadata.title.as_deref(), Some("World Name"));

        let jp = parse_screenscraper_game(
            r#"{"header":{"success":"true"},"response":{"jeu":{"noms":[{"region":"jp","text":"クロノ"}]}}}"#,
        )
        .unwrap();
        assert!(jp.metadata.title.is_none());

        let us_year = parse_screenscraper_game(
            r#"{"header":{"success":"true"},"response":{"jeu":{"dates":[{"region":"jp","text":"1990"},{"region":"us","text":"2001-01-01"}]}}}"#,
        )
        .unwrap();
        assert_eq!(us_year.metadata.year, Some(2001));
        assert_eq!(
            parse_screenscraper_game(
                r#"{"header":{"success":"true"},"response":{"jeu":{"dates":{"region":"us","text":"1995-08"}}}}"#
            )
            .unwrap()
            .metadata
            .year,
            Some(1995)
        );
        assert_eq!(
            parse_screenscraper_game(
                r#"{"header":{"success":"true"},"response":{"jeu":{"dates":{"region":"eu","text":"1899"}}}}"#
            )
            .unwrap()
            .metadata
            .year,
            None
        );
        assert_eq!(
            parse_screenscraper_game(
                r#"{"header":{"success":"true"},"response":{"jeu":{"dates":{"region":"wor","text":"2100"}}}}"#
            )
            .unwrap()
            .metadata
            .year,
            None
        );
    }

    #[test]
    fn screenscraper_empty_medias_still_returns_metadata() {
        let game = parse_screenscraper_game(
            r#"{"header":{"success":"true"},"response":{"jeu":{"noms":{"region":"wor","text":"Tetris"}}}}"#,
        )
        .unwrap();
        assert_eq!(game.metadata.title.as_deref(), Some("Tetris"));
        assert!(game.metadata.publisher.is_none());
        assert!(game.metadata.year.is_none());
        assert!(game.metadata.genre.is_none());
        assert!(game.medias.is_empty());
    }

    #[test]
    fn screenscraper_header_error_is_err() {
        let err =
            parse_screenscraper_game(r#"{"header":{"success":"false","error":"Jeu non trouve"}}"#)
                .unwrap_err();
        let FetchFail::Failed(message) = err else {
            panic!("expected failed");
        };
        assert!(message.contains("Jeu non trouve"));
    }

    #[test]
    fn thegamesdb_metadata_requires_an_exact_title() {
        let body = r#"{"status":"Success","data":{"games":[{"id":9,"game_title":"Other"},{"id":42,"game_title":"Chrono Trigger","release_date":"1995-08-22","genres":[4],"publishers":[7]}]}}"#;
        let matched = parse_thegamesdb_game(body, "chrono trigger").unwrap();
        assert_eq!(matched.id, 42);
        assert!(matched.exact);
        assert_eq!(matched.year, Some(1995));
        assert_eq!(matched.genre_ids, vec![4]);
        assert_eq!(matched.publisher_ids, vec![7]);
        let miss = parse_thegamesdb_game(body, "no such game").unwrap();
        assert_eq!(miss.id, 9);
        assert!(!miss.exact);
        let err = metadata_if_matched(&miss, &TgNames::default()).unwrap_err();
        let FetchFail::Failed(message) = err else {
            panic!("expected failed");
        };
        assert_eq!(message, "TheGamesDB has no game match");
        assert_eq!(
            parse_thegamesdb_game_id(body, "chrono trigger").unwrap(),
            42
        );
        assert_eq!(parse_thegamesdb_game_id(body, "no such game").unwrap(), 9);
        let by_id = parse_thegamesdb_game_by_id(body, 42).unwrap();
        assert_eq!(by_id.id, 42);
        assert!(by_id.exact);
        assert_eq!(by_id.title.as_deref(), Some("Chrono Trigger"));
        assert!(parse_thegamesdb_game_by_id(body, 7).is_err());
    }

    #[test]
    fn thegamesdb_names_become_a_genre() {
        let names = parse_thegamesdb_names(
            r#"{"status":"Success","data":{"count":1,"genres":{"4":{"id":4,"name":"Role-Playing"}}}}"#,
            TgList::Genres,
        )
        .unwrap();
        assert_eq!(names, vec![(4, "Role-Playing".into())]);
        let mut cache = TgNames::default();
        for (id, name) in names {
            cache.genres.insert(id, name);
        }
        let game = TgGame {
            id: 42,
            exact: true,
            title: Some("Chrono Trigger".into()),
            year: Some(1995),
            genre_ids: vec![4],
            publisher_ids: vec![],
        };
        assert_eq!(game.metadata(&cache).genre.as_deref(), Some("Role-Playing"));
    }

    #[test]
    fn missing_work_splits_art_from_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let box_path = dir.path().join("box.png");
        let shot_path = dir.path().join("shot.png");
        fs::write(&box_path, b"box").unwrap();
        fs::write(&shot_path, b"shot").unwrap();
        let mut game = crate::types::Game {
            id: "chrono".into(),
            console: "snes".into(),
            rom: PathBuf::from("/tmp/chrono.sfc"),
            file_title: "chrono trigger".into(),
            user_title: None,
            metadata: None,
            crc32: None,
            profile: None,
            media: vec![
                Media {
                    kind: MediaKind::BoxArt,
                    path: box_path,
                    source: Source::Local,
                },
                Media {
                    kind: MediaKind::Screenshot,
                    path: shot_path,
                    source: Source::Local,
                },
            ],
            last_played: None,
            play_count: 0,
            play_time: 0,
            favorite: false,
        };
        let enabled = ScraperConfig::default().enabled_kinds();
        let work = missing_work(&game, &enabled).unwrap();
        assert!(work.kinds.is_empty());
        assert!(work.metadata);

        game.metadata = Some(GameMetadata {
            title: Some("Chrono Trigger".into()),
            ..GameMetadata::default()
        });
        assert!(missing_work(&game, &enabled).is_none());

        game.media[0].path = PathBuf::from("/no/such/retromarchy-box.png");
        let work = missing_work(&game, &enabled).unwrap();
        assert_eq!(work.kinds, vec![MediaKind::BoxArt]);
        assert!(!work.metadata);
    }

    #[test]
    fn scrape_failure_line_keeps_a_metadata_reason() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("chrono_trigger.sfc");
        fs::write(&rom, b"rom-bytes!").unwrap();
        let query = ArtworkQuery::from_game(&dummy_game(rom, "snes", 0x1A2B3C4D));
        let line = scrape_failure_line(
            &query,
            &[
                "metadata ScreenScraper: HTTP 404 Jeu non trouve".into(),
                format!("metadata {}", crate::softname::DEVPASSWORD),
            ],
        );
        assert!(line.starts_with("scrape failed:"));
        assert!(line.contains("metadata ScreenScraper: HTTP 404 Jeu non trouve"));
        assert!(line.contains("title=\"Asteroids\""));
        assert!(!line.contains(crate::softname::DEVPASSWORD));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn fixture_metadata_reads_jeu_infos() {
        let dir = tempfile::tempdir().unwrap();
        let missing = fixture_metadata(dir.path()).unwrap_err();
        let FetchFail::Failed(message) = missing else {
            panic!("expected failed");
        };
        assert_eq!(message, "fixture metadata missing");
        fs::write(
            dir.path().join("jeuInfos.json"),
            r#"{"header":{"success":"true"},"response":{"jeu":{"noms":{"region":"wor","text":"Tetris"}}}}"#,
        )
        .unwrap();
        let metadata = fixture_metadata(dir.path()).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Tetris"));
        assert!(metadata.publisher.is_none());
    }

    fn named(id: &str) -> crate::types::Game {
        let mut game = dummy_game(PathBuf::from(format!("/roms/{id}.sfc")), "snes", 1);
        game.id = id.into();
        game.file_title = id.into();
        game
    }

    #[test]
    fn scrape_runs_refuse_a_second_pass_and_keep_games_a_scan_adds() {
        let mut runs = ScrapeRuns::default();
        match runs.request_missing("snes", vec![named("a")]) {
            Admit::Start(batch) => assert_eq!(batch.games[0].id, "a"),
            other => panic!("expected start, got {other:?}"),
        }
        assert!(runs.tracks("snes"));
        assert!(matches!(
            runs.request_missing("snes", vec![named("a"), named("b")]),
            Admit::Busy
        ));
        assert!(!runs.request_one("snes"));

        assert!(matches!(
            runs.request_added("snes", vec![named("b")]),
            Admit::Queued
        ));
        assert!(matches!(
            runs.request_added("snes", vec![named("b"), named("c")]),
            Admit::Queued
        ));
        assert!(matches!(
            runs.request_missing("nes", vec![named("d")]),
            Admit::Queued
        ));

        let next = runs.finish().unwrap();
        assert_eq!(next.console_id, "snes");
        let ids: Vec<_> = next.games.iter().map(|game| game.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c"]);

        let nes = runs.finish().unwrap();
        assert_eq!(nes.console_id, "nes");
        assert_eq!(nes.games[0].id, "d");
        assert!(runs.finish().is_none());
        assert!(!runs.tracks("snes"));
        assert!(!runs.tracks("nes"));

        assert!(runs.request_one("genesis"));
        assert!(matches!(
            runs.request_missing("genesis", vec![named("e")]),
            Admit::Busy
        ));
        assert!(matches!(
            runs.request_added("snes", vec![named("f")]),
            Admit::Queued
        ));
        let added = runs.finish().unwrap();
        assert_eq!(added.console_id, "snes");
        assert_eq!(added.games[0].id, "f");
        assert!(runs.finish().is_none());

        match runs.request_added("snes", vec![named("g")]) {
            Admit::Start(batch) => assert_eq!(batch.games[0].id, "g"),
            other => panic!("expected an idle scan to start, got {other:?}"),
        }
    }
}
