use crate::types::{
    GridArt, Media, MediaKind, ScrapeProvider, ScraperConfig, ScraperCredentials, Source,
};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
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
    pub crc32: Option<u32>,
    pub rom_bytes: Option<u64>,
}

impl ArtworkQuery {
    pub fn from_game(game: &crate::types::Game) -> Self {
        let rom_bytes = fs::metadata(&game.rom).ok().map(|meta| meta.len());
        let rom_name = game
            .rom
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            console_id: game.console.clone(),
            title: game.title.clone(),
            rom_name,
            crc32: game.crc32,
            rom_bytes,
        }
    }
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

struct SsMedia {
    type_name: String,
    region: String,
    url: String,
}

pub fn parse_screenscraper_medias(body: &str) -> Result<Vec<(String, String, String)>, FetchFail> {
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
    let medias = value
        .pointer("/response/jeu/medias")
        .and_then(|medias| medias.as_array())
        .ok_or_else(|| FetchFail::Failed("ScreenScraper has no game match".into()))?;
    let mut out = Vec::new();
    for media in medias {
        let type_name = media.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let region = media.get("region").and_then(|v| v.as_str()).unwrap_or("");
        let url = media.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if type_name.is_empty() || url.is_empty() {
            continue;
        }
        out.push((type_name.to_string(), region.to_string(), url.to_string()));
    }
    Ok(out)
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
    let exact = games.iter().find(|game| {
        game.get("game_title")
            .and_then(|title_value| title_value.as_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(title))
    });
    let game = exact.unwrap_or(&games[0]);
    game.get("id")
        .and_then(|id| id.as_i64())
        .ok_or_else(|| FetchFail::Failed("TheGamesDB game id missing".into()))
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
    Some(match console_id {
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
    Some(match console_id {
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
    ss_medias: Option<Result<Vec<SsMedia>, FetchFail>>,
    tgdb_id: Option<Result<i64, FetchFail>>,
    tgdb_images: Option<Result<(String, Vec<TgImage>), FetchFail>>,
}

impl HttpGame {
    fn new() -> Self {
        Self {
            ss_medias: None,
            tgdb_id: None,
            tgdb_images: None,
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
        if self.ss_medias.is_none() {
            let loaded = load_screenscraper(agent, creds, query, true, last_http).or_else(|err| {
                if matches!(err, FetchFail::RateLimited(_))
                    || screenscraper_system(&query.console_id).is_none()
                {
                    Err(err)
                } else {
                    load_screenscraper(agent, creds, query, false, last_http)
                }
            });
            self.ss_medias = Some(loaded);
        }
        let medias = match self.ss_medias.as_ref().expect("filled above") {
            Ok(medias) => medias,
            Err(err) => return Err(err.clone()),
        };
        let tuples: Vec<_> = medias
            .iter()
            .map(|media| {
                (
                    media.type_name.clone(),
                    media.region.clone(),
                    media.url.clone(),
                )
            })
            .collect();
        pick_screenscraper_url(&tuples, kind).ok_or_else(|| {
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
        if self.tgdb_id.is_none() {
            let with_platform = thegamesdb_platform(&query.console_id).is_some();
            let loaded =
                load_thegamesdb_id(agent, creds, query, with_platform, last_http).or_else(|err| {
                    if !with_platform || matches!(err, FetchFail::RateLimited(_)) {
                        Err(err)
                    } else {
                        load_thegamesdb_id(agent, creds, query, false, last_http)
                    }
                });
            self.tgdb_id = Some(loaded);
        }
        let game_id = match self.tgdb_id.as_ref().expect("filled above") {
            Ok(id) => *id,
            Err(err) => return Err(err.clone()),
        };
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
) -> Result<Vec<SsMedia>, FetchFail> {
    let mut params = screenscraper_params(creds, query);
    if !with_system {
        params.retain(|(key, _)| key != "systemeid");
    }
    let url = format!(
        "https://www.screenscraper.fr/api2/jeuInfos.php?{}",
        encode_query(&params)
    );
    let body = download_text(agent, &url, last_http)?;
    let medias = parse_screenscraper_medias(&body)?;
    Ok(medias
        .into_iter()
        .map(|(type_name, region, url)| SsMedia {
            type_name,
            region,
            url,
        })
        .collect())
}

fn load_thegamesdb_id(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    query: &ArtworkQuery,
    with_platform: bool,
    last_http: &mut Option<Instant>,
) -> Result<i64, FetchFail> {
    let mut params = vec![
        ("apikey".to_string(), creds.thegamesdb_api_key.clone()),
        ("name".to_string(), query.title.clone()),
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
    parse_thegamesdb_game_id(&body, &query.title)
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

fn fixture_bytes(dir: &Path, kind: MediaKind) -> Result<Vec<u8>, FetchFail> {
    let name = format!("{}.png", kind_file_stem(kind));
    fs::read(dir.join(name)).map_err(|_| {
        FetchFail::Failed(format!(
            "fixture image missing for {}",
            kind_file_stem(kind).replace('_', " ")
        ))
    })
}

/// One name-search hit. `remote_id` is the provider game id, not the library id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapeCandidate {
    pub provider: ScrapeProvider,
    pub remote_id: String,
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
        let Some(remote_id) = jeu.get("id").and_then(json_id) else {
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
            remote_id,
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
        let Some(remote_id) = game.get("id").and_then(json_id) else {
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
            remote_id,
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
            remote_id: "1".into(),
            title: query.to_string(),
            system: "Super Nintendo".into(),
        },
        ScrapeCandidate {
            provider: ScrapeProvider::TheGamesDb,
            remote_id: "2".into(),
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
    if kinds.is_empty() {
        return "Enable box art or screenshot in Scraper settings.".into();
    }
    let root = match media_root() {
        Ok(root) => root,
        Err(err) => return format!("Could not create the artwork cache: {err}"),
    };
    let _ = tx.send(ScrapeUpdate::Status(format!("Scraping {}…", game.title)));
    let source = if fixtures.is_some() {
        Source::Local
    } else {
        candidate.provider.source()
    };
    let loaded: Vec<(MediaKind, Result<Vec<u8>, FetchFail>)> = if let Some(dir) = &fixtures {
        kinds
            .iter()
            .map(|kind| (*kind, fixture_bytes(dir, *kind)))
            .collect()
    } else if let Some(reason) = scraper.credentials.block_reason(candidate.provider) {
        return reason.to_string();
    } else {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(25))
            .build();
        let mut last_http = None;
        match load_candidate_images(
            &agent,
            &scraper.credentials,
            &candidate,
            &kinds,
            &mut last_http,
        ) {
            Ok(images) => images,
            Err(err) => return fail_message(err),
        }
    };
    let mut saved = 0usize;
    let mut failed = 0usize;
    for (kind, bytes) in loaded {
        match bytes {
            Ok(bytes) => match write_cached_image(&root, &game.console, &game.id, kind, &bytes) {
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
            },
            Err(_) => failed += 1,
        }
    }
    let mut summary = format!("Scrape finished: {saved} saved, {failed} failed.");
    if fixtures.is_some() {
        summary.push_str(" Used local fixtures; no network requests were made.");
    }
    summary
}

fn load_candidate_images(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    candidate: &ScrapeCandidate,
    kinds: &[MediaKind],
    last_http: &mut Option<Instant>,
) -> Result<Vec<(MediaKind, Result<Vec<u8>, FetchFail>)>, FetchFail> {
    match candidate.provider {
        ScrapeProvider::ScreenScraper => {
            let medias = load_screenscraper_by_id(agent, creds, &candidate.remote_id, last_http)?;
            Ok(kinds
                .iter()
                .copied()
                .map(|kind| {
                    let bytes = match pick_screenscraper_url(&medias, kind) {
                        Some(url) if url_allowed(&url) => download_limited(agent, &url, last_http),
                        Some(_) => Err(FetchFail::Failed(
                            "ScreenScraper: refused a non-image download".into(),
                        )),
                        None => Err(FetchFail::Failed(format!(
                            "ScreenScraper: no {} for this game",
                            kind_file_stem(kind).replace('_', " ")
                        ))),
                    };
                    (kind, bytes)
                })
                .collect())
        }
        ScrapeProvider::TheGamesDb => {
            let game_id: i64 = candidate
                .remote_id
                .parse()
                .map_err(|_| FetchFail::Failed("TheGamesDB game id was not a number.".into()))?;
            let (base, images) = load_thegamesdb_images(agent, creds, game_id, last_http)?;
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
            Ok(kinds
                .iter()
                .copied()
                .map(|kind| {
                    let bytes = match pick_thegamesdb_filename(&tuples, kind) {
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
                    };
                    (kind, bytes)
                })
                .collect())
        }
    }
}

fn load_screenscraper_by_id(
    agent: &ureq::Agent,
    creds: &ScraperCredentials,
    game_id: &str,
    last_http: &mut Option<Instant>,
) -> Result<Vec<(String, String, String)>, FetchFail> {
    let params = screenscraper_game_params(creds, game_id);
    let url = format!(
        "https://www.screenscraper.fr/api2/jeuInfos.php?{}",
        encode_query(&params)
    );
    let body = download_text(agent, &url, last_http)?;
    parse_screenscraper_medias(&body)
}

#[derive(Debug)]
pub enum ScrapeUpdate {
    Status(String),
    Saved { game_id: String, media: Media },
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

pub fn fixture_dir_from_env() -> Option<PathBuf> {
    std::env::var_os("RETROMARCHY_SCRAPER_FIXTURES")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

fn run_scrape(
    games: Vec<crate::types::Game>,
    scraper: ScraperConfig,
    fixtures: Option<PathBuf>,
    tx: &std::sync::mpsc::Sender<ScrapeUpdate>,
) -> String {
    let enabled = scraper.enabled_kinds();
    if enabled.is_empty() {
        return "Enable box art or screenshot in Scraper settings.".into();
    }
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

    for (index, game) in games.iter().enumerate() {
        let _ = tx.send(ScrapeUpdate::Status(format!(
            "Scraping {}/{}: {}",
            index + 1,
            games.len(),
            game.title
        )));
        let have = present_kinds(&game.media);
        let todo = kinds_to_fetch(&have, &enabled);
        if todo.is_empty() {
            skipped += 1;
            continue;
        }
        let query = ArtworkQuery::from_game(game);
        let mut http = HttpGame::new();
        let mut game_failed = false;
        for kind in todo {
            let providers: Vec<_> = active
                .iter()
                .copied()
                .filter(|provider| !cooled.contains(provider))
                .collect();
            if providers.is_empty() {
                game_failed = true;
                continue;
            }
            let outcome = take_first_success(&providers, |provider| {
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
            for (provider, err) in &outcome.errors {
                if matches!(err, FetchFail::RateLimited(_)) && !cooled.contains(provider) {
                    cooled.push(*provider);
                }
            }
            if let Some((_provider, media)) = outcome.success {
                saved += 1;
                let _ = tx.send(ScrapeUpdate::Saved {
                    game_id: game.id.clone(),
                    media,
                });
            } else {
                game_failed = true;
            }
        }
        if game_failed {
            failed += 1;
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
    use crate::types::{ProviderEntry, Source};
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
        let medias = parse_screenscraper_medias(body).unwrap();
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
        assert_eq!(hits[0].remote_id, "3");
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
        assert_eq!(hits[0].remote_id, "111");
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
            title: "Chrono".into(),
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
}
