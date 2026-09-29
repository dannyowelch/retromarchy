use crate::types::{Console, Game, GameId, Media, MediaKind, Source};
use anyhow::Result;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn scan_console(console: &Console) -> Result<Vec<Game>> {
    let mut games = Vec::new();

    for rom_dir in &console.rom_dirs {
        let rom_dir = expand_home(rom_dir);
        if !rom_dir.exists() {
            continue;
        }
        scan_directory(&rom_dir, console, &mut games)?;
    }

    Ok(games)
}

/// `~` and `~/...` use `HOME`. Anything else, including a missing home, stays as written.
pub fn expand_home(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if text == "~" {
        return home.unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = home {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

/// Scan every ROM folder and upsert. Rows that are still on disk keep favorites,
/// play stats, renames, and scraped metadata. Rows whose files are gone are dropped.
pub fn rescan(console: &Console, conn: &rusqlite::Connection) -> Result<Vec<Game>> {
    let scanned = scan_console(console)?;
    crate::database::replace_scanned_games(conn, &console.id, &scanned)
}

/// Games in `scanned` whose ids were not in the library before this scan.
pub fn added_games(previous_ids: &[GameId], scanned: &[Game]) -> Vec<Game> {
    let known: HashSet<&str> = previous_ids.iter().map(String::as_str).collect();
    scanned
        .iter()
        .filter(|game| !known.contains(game.id.as_str()))
        .cloned()
        .collect()
}

fn scan_directory(dir: &Path, console: &Console, games: &mut Vec<Game>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            scan_directory(&path, console, games)?;
        } else if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            if console
                .extensions
                .iter()
                .any(|e| e.eq_ignore_ascii_case(&ext_str))
                && !games.iter().any(|game| game.rom == path)
            {
                if let Some(game) = process_rom(&path, console)? {
                    games.push(game);
                }
            }
        }
    }

    Ok(())
}

fn process_rom(rom_path: &Path, console: &Console) -> Result<Option<Game>> {
    let id = compute_game_id(rom_path);
    let crc32 = compute_crc32(rom_path)?;
    let media = discover_local_media(rom_path, console)?;

    Ok(Some(Game {
        id,
        console: console.id.clone(),
        rom: rom_path.to_path_buf(),
        file_title: derive_title(rom_path),
        user_title: None,
        metadata: None,
        crc32: Some(crc32),
        profile: None,
        media,
        last_played: None,
        play_count: 0,
        play_time: 0,
        favorite: false,
    }))
}

pub fn compute_game_id(rom_path: &Path) -> GameId {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    rom_path.to_string_lossy().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

pub fn derive_title(rom_path: &Path) -> String {
    let file_stem = rom_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown");

    clean_title(file_stem)
}

pub fn clean_title(raw: &str) -> String {
    let re_region = regex::Regex::new(r"\((?:USA|Europe|Japan|World|En|Fr|De|Es|It|Pt|Nl|Sv|No|Da|Fi|Pl|Ru|Ko|Zh|NTSC|PAL|Beta|Proto|Rev \d+|v\d+\.\d+)\)").unwrap();
    let re_brackets = regex::Regex::new(r"\[.*?\]").unwrap();
    let re_extra = regex::Regex::new(r"\(.*?\)").unwrap();

    let mut title = raw.to_string();
    title = re_region.replace_all(&title, "").to_string();
    title = re_brackets.replace_all(&title, "").to_string();
    title = re_extra.replace_all(&title, "").to_string();
    title = title.replace('_', " ");
    title = title.trim().to_string();

    if title.is_empty() {
        raw.to_string()
    } else {
        title
    }
}

fn compute_crc32(path: &Path) -> Result<u32> {
    let mut file = fs::File::open(path)?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = [0; 8192];

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(hasher.finalize())
}

fn discover_local_media(rom_path: &Path, console: &Console) -> Result<Vec<Media>> {
    let mut media = Vec::new();
    let rom_dir = rom_path.parent().unwrap_or(Path::new(""));
    let rom_stem = rom_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    let media_configs: &[(MediaKind, &str, &[&str], bool)] = &[
        (
            MediaKind::BoxArt,
            "box_art",
            &["png", "jpg", "jpeg"],
            console.media.box_art,
        ),
        (
            MediaKind::Screenshot,
            "screenshot",
            &["png", "jpg", "jpeg"],
            console.media.screenshot,
        ),
        (
            MediaKind::Manual,
            "manual",
            &["pdf", "txt"],
            console.media.manual,
        ),
        (
            MediaKind::Video,
            "video",
            &["mp4", "mkv", "avi"],
            console.media.video,
        ),
    ];

    for &(kind, subdir_name, extensions, enabled) in media_configs {
        if !enabled {
            continue;
        }

        for &ext in extensions {
            let same_dir_path = rom_dir.join(format!("{}.{}", rom_stem, ext));
            if same_dir_path.exists() {
                media.push(Media {
                    kind,
                    path: same_dir_path,
                    source: Source::Local,
                });
                break;
            }

            let subdir_path = rom_dir
                .join(subdir_name)
                .join(format!("{}.{}", rom_stem, ext));
            if subdir_path.exists() {
                media.push(Media {
                    kind,
                    path: subdir_path,
                    source: Source::Local,
                });
                break;
            }
        }
    }

    Ok(media)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MediaToggles;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_clean_title() {
        assert_eq!(clean_title("Super Mario World (USA)"), "Super Mario World");
        assert_eq!(clean_title("Zelda (Europe) [!]"), "Zelda");
        assert_eq!(clean_title("Final_Fantasy_VI"), "Final Fantasy VI");
        assert_eq!(clean_title("Metroid (Rev 1)"), "Metroid");
        assert_eq!(clean_title("Game (Beta)"), "Game");
    }

    #[test]
    fn test_idempotent_scan() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let rom_dir = temp_dir.path().join("roms");
        fs::create_dir(&rom_dir)?;

        fs::write(rom_dir.join("game1.sfc"), b"dummy rom 1")?;
        fs::write(rom_dir.join("game2.sfc"), b"dummy rom 2")?;

        let console = Console {
            id: "snes".to_string(),
            name: "Super Nintendo".to_string(),
            rom_dirs: vec![rom_dir.clone()],
            extensions: vec!["sfc".to_string()],
            emulator: None,
            core: None,
            extra_args: String::new(),
            grid_art: crate::types::GridArt::default(),
            media: MediaToggles::default(),
        };

        let games1 = scan_console(&console)?;
        assert_eq!(games1.len(), 2);

        let games2 = scan_console(&console)?;
        assert_eq!(games2.len(), 2);

        let ids1: Vec<_> = games1.iter().map(|g| g.id.clone()).collect();
        let ids2: Vec<_> = games2.iter().map(|g| g.id.clone()).collect();
        assert_eq!(ids1, ids2);

        Ok(())
    }

    #[test]
    fn test_media_discovery() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let rom_dir = temp_dir.path().join("roms");
        fs::create_dir(&rom_dir)?;

        let rom_path = rom_dir.join("game.sfc");
        fs::write(&rom_path, b"dummy")?;

        fs::write(rom_dir.join("game.png"), b"boxart")?;

        let box_art_dir = rom_dir.join("box_art");
        fs::create_dir(&box_art_dir)?;
        fs::write(box_art_dir.join("game.jpg"), b"boxart2")?;

        let console = Console {
            id: "snes".to_string(),
            name: "Super Nintendo".to_string(),
            rom_dirs: vec![rom_dir.clone()],
            extensions: vec!["sfc".to_string()],
            emulator: None,
            core: None,
            extra_args: String::new(),
            grid_art: crate::types::GridArt::default(),
            media: MediaToggles {
                box_art: true,
                screenshot: false,
                manual: false,
                video: false,
            },
        };

        let media = discover_local_media(&rom_path, &console)?;
        assert_eq!(media.len(), 1);
        assert_eq!(media[0].kind, MediaKind::BoxArt);
        assert_eq!(media[0].source, Source::Local);

        Ok(())
    }

    fn console_at(dirs: Vec<PathBuf>) -> Console {
        Console {
            id: "snes".to_string(),
            name: "Super Nintendo".to_string(),
            rom_dirs: dirs,
            extensions: vec!["sfc".to_string()],
            emulator: None,
            core: None,
            extra_args: String::new(),
            grid_art: crate::types::GridArt::default(),
            media: MediaToggles::default(),
        }
    }

    #[test]
    fn scan_reads_every_folder_and_skips_other_extensions() -> Result<()> {
        let temp = TempDir::new()?;
        let first = temp.path().join("a");
        let second = temp.path().join("b");
        fs::create_dir(&first)?;
        fs::create_dir(&second)?;
        fs::write(first.join("keep.sfc"), b"one")?;
        fs::write(first.join("notes.txt"), b"skip")?;
        fs::write(second.join("other.sfc"), b"two")?;
        fs::create_dir(second.join("nested"))?;
        fs::write(second.join("nested").join("deep.sfc"), b"three")?;

        let games = scan_console(&console_at(vec![first.clone(), second, first]))?;
        let mut titles: Vec<_> = games.iter().map(|game| game.file_title.clone()).collect();
        titles.sort();
        assert_eq!(titles, vec!["deep", "keep", "other"]);
        Ok(())
    }

    #[test]
    fn tilde_folders_are_scanned() -> Result<()> {
        let root = TempDir::new()?;
        let _env = crate::config::XdgEnv::sandbox(root.path());
        let home = root.path().join("home");
        let folder = home.join("roms");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("tilde.sfc"), b"rom")?;

        let games = scan_console(&console_at(vec![PathBuf::from("~/roms")]))?;
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].rom, folder.join("tilde.sfc"));
        Ok(())
    }

    #[test]
    fn rescan_adds_new_roms_drops_missing_and_keeps_metadata() -> Result<()> {
        let temp = TempDir::new()?;
        let first = temp.path().join("a");
        let second = temp.path().join("b");
        fs::create_dir(&first)?;
        fs::create_dir(&second)?;
        let keep_path = first.join("keep.sfc");
        let gone_path = first.join("gone.sfc");
        fs::write(&keep_path, b"keep")?;
        fs::write(&gone_path, b"gone")?;
        fs::write(second.join("other.sfc"), b"other")?;

        let db = temp.path().join("library.db");
        let conn = crate::database::open_db(&db)?;
        let console = console_at(vec![first.clone(), second.clone()]);
        let scanned = rescan(&console, &conn)?;
        let keep = scanned.iter().find(|game| game.rom == keep_path).unwrap();
        let keep_id = keep.id.clone();
        crate::database::set_favorite(&conn, &keep_id, true)?;
        crate::database::set_game_title(&conn, &keep_id, "Kept Name")?;
        crate::database::set_game_metadata(
            &conn,
            &keep_id,
            &crate::types::GameMetadata {
                title: Some("Scraped Keep".into()),
                publisher: Some("Square".into()),
                year: Some(1995),
                genre: Some("RPG".into()),
            },
        )?;
        crate::database::set_game_media(
            &conn,
            &keep_id,
            &Media {
                kind: MediaKind::BoxArt,
                path: PathBuf::from("/art/keep.png"),
                source: Source::ScreenScraper,
            },
        )?;
        crate::database::increment_play_stats(&conn, &keep_id, 12)?;

        fs::remove_file(&gone_path)?;
        fs::write(second.join("new.sfc"), b"new")?;
        let games = rescan(&console, &conn)?;
        let titles: Vec<_> = games.iter().map(|game| game.file_title.as_str()).collect();
        assert_eq!(titles, vec!["keep", "new", "other"]);
        assert!(games.iter().all(|game| game.rom != gone_path));
        assert!(keep_path.is_file());

        let keep = games.iter().find(|game| game.id == keep_id).unwrap();
        assert!(keep.favorite);
        assert_eq!(keep.user_title.as_deref(), Some("Kept Name"));
        assert_eq!(keep.display_title(), "Kept Name");
        let metadata = keep.metadata.as_ref().unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Scraped Keep"));
        assert_eq!(metadata.publisher.as_deref(), Some("Square"));
        assert_eq!(metadata.year, Some(1995));
        assert_eq!(metadata.genre.as_deref(), Some("RPG"));
        assert_eq!(keep.media.len(), 1);
        assert_eq!(keep.media[0].source, Source::ScreenScraper);
        assert_eq!(keep.play_count, 1);
        assert_eq!(keep.play_time, 12);
        Ok(())
    }

    #[test]
    fn added_games_are_ids_the_library_did_not_have() {
        let keep = Game {
            id: "keep".into(),
            console: "snes".into(),
            rom: PathBuf::from("/roms/keep.sfc"),
            file_title: "keep".into(),
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
        let mut fresh = keep.clone();
        fresh.id = "fresh".into();
        fresh.file_title = "fresh".into();
        fresh.rom = PathBuf::from("/roms/fresh.sfc");
        let scanned = vec![keep.clone(), fresh.clone()];
        assert!(added_games(&["keep".into()], &scanned)
            .iter()
            .map(|game| game.id.as_str())
            .eq(["fresh"]));
        assert!(added_games(&["keep".into(), "fresh".into()], &scanned).is_empty());
        assert_eq!(added_games(&[], &scanned).len(), 2);
        assert!(added_games(&["keep".into()], &[]).is_empty());
    }
}
