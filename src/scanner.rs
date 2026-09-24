use crate::types::{Console, Game, GameId, Media, MediaKind, Source};
use anyhow::Result;
use std::fs;
use std::io::Read;
use std::path::Path;

pub fn scan_console(console: &Console) -> Result<Vec<Game>> {
    let mut games = Vec::new();

    for rom_dir in &console.rom_dirs {
        if !rom_dir.exists() {
            continue;
        }
        scan_directory(rom_dir, console, &mut games)?;
    }

    Ok(games)
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
            if console.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext_str)) {
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
    let title = derive_title(rom_path);
    let crc32 = compute_crc32(rom_path)?;
    let media = discover_local_media(rom_path, console)?;

    Ok(Some(Game {
        id,
        console: console.id.clone(),
        rom: rom_path.to_path_buf(),
        title,
        crc32: Some(crc32),
        profile: None,
        media,
        last_played: None,
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
        (MediaKind::BoxArt, "box_art", &["png", "jpg", "jpeg"], console.media.box_art),
        (MediaKind::Screenshot, "screenshot", &["png", "jpg", "jpeg"], console.media.screenshot),
        (MediaKind::Manual, "manual", &["pdf", "txt"], console.media.manual),
        (MediaKind::Video, "video", &["mp4", "mkv", "avi"], console.media.video),
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

            let subdir_path = rom_dir.join(subdir_name).join(format!("{}.{}", rom_stem, ext));
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
            profile: "retroarch-snes9x".to_string(),
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
            profile: "retroarch-snes9x".to_string(),
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
}
