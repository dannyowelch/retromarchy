use crate::catalog::{self, SystemEntry};
use crate::config::{self, Config};
use crate::database;
use crate::scanner;
use crate::types::{Console, MediaToggles};
use anyhow::Result;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FoundFolder {
    pub folder_name: String,
    pub path: PathBuf,
    pub matched_id: Option<String>,
    pub file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportChoice {
    pub system_id: String,
    pub path: PathBuf,
}

#[derive(Debug)]
pub enum ImportUpdate {
    Discover(Vec<FoundFolder>),
    Status(String),
    Done { systems: usize, games: usize },
    Failed(String),
}

pub fn discover_root(root: &Path) -> Vec<FoundFolder> {
    let mut found = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return found,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let folder_name = entry.file_name().to_string_lossy().to_string();
        let matched = catalog::match_folder(&folder_name).map(|s| s.folder_id.clone());
        let extensions = matched
            .as_deref()
            .and_then(|id| catalog::by_id().get(id).map(|s| s.extensions.clone()))
            .unwrap_or_default();
        let file_count = if extensions.is_empty() {
            count_files(&path)
        } else {
            count_matching(&path, &extensions)
        };
        found.push(FoundFolder {
            folder_name,
            path,
            matched_id: matched,
            file_count,
        });
    }
    found.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));
    found
}

fn count_files(dir: &Path) -> usize {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                count += 1;
            }
        }
    }
    count
}

fn count_matching(dir: &Path, extensions: &[String]) -> usize {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                    count += 1;
                }
            }
        }
    }
    count
}

pub fn apply_imports(config: &mut Config, conn: &Connection, choices: &[ImportChoice]) -> Result<()> {
    apply_imports_reporting(config, conn, choices, &mut |_, _, _| {}).map(|_| ())
}

/// Same write as [`apply_imports`]. `report` runs before each console scan so a
/// caller can show progress. Returns `(systems, games)` actually scanned.
pub fn apply_imports_reporting(
    config: &mut Config,
    conn: &Connection,
    choices: &[ImportChoice],
    report: &mut dyn FnMut(usize, usize, &str),
) -> Result<(usize, usize)> {
    let catalog = catalog::by_id();
    let total = choices.len();
    let mut systems = 0;
    let mut games = 0;
    for (index, choice) in choices.iter().enumerate() {
        let Some(system) = catalog.get(choice.system_id.as_str()) else {
            continue;
        };
        report(index, total, &system.display_name);
        upsert_console(config, system, &choice.path);
        let console = config
            .consoles
            .iter()
            .find(|c| c.id == choice.system_id)
            .cloned()
            .expect("console just upserted");
        let scanned = scanner::scan_console(&console)?;
        games += scanned.len();
        systems += 1;
        let ids: Vec<_> = scanned.iter().map(|g| g.id.clone()).collect();
        for game in &scanned {
            database::upsert_game(conn, game)?;
        }
        database::remove_missing_games(conn, &console.id, &ids)?;
    }
    config::save_config(config)?;
    Ok((systems, games))
}

pub fn spawn_discover(root: PathBuf) -> std::sync::mpsc::Receiver<ImportUpdate> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let found = discover_root(&root);
        let _ = tx.send(ImportUpdate::Discover(found));
    });
    rx
}

pub fn spawn_apply(choices: Vec<ImportChoice>) -> std::sync::mpsc::Receiver<ImportUpdate> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let mut config = config::load_config()?;
            let conn = database::init_db()?;
            apply_imports_reporting(&mut config, &conn, &choices, &mut |index, total, name| {
                let _ = tx.send(ImportUpdate::Status(format!(
                    "Scanning {name} ({}/{total})…",
                    index + 1
                )));
            })
        })();
        let update = match result {
            Ok((systems, games)) => ImportUpdate::Done { systems, games },
            Err(err) => ImportUpdate::Failed(err.to_string()),
        };
        let _ = tx.send(update);
    });
    rx
}

fn upsert_console(config: &mut Config, system: &SystemEntry, path: &Path) {
    if let Some(existing) = config.consoles.iter_mut().find(|c| c.id == system.folder_id) {
        existing.name = system.display_name.clone();
        existing.rom_dirs = vec![path.to_path_buf()];
        existing.extensions = system.extensions.clone();
        return;
    }
    config.consoles.push(Console {
        id: system.folder_id.clone(),
        name: system.display_name.clone(),
        rom_dirs: vec![path.to_path_buf()],
        extensions: system.extensions.clone(),
        profile: None,
        grid_art: crate::types::GridArt::default(),
        media: MediaToggles::default(),
    });
}

pub fn default_roms_root() -> PathBuf {
    dirs_home().join("ROMs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::XdgEnv;
    use crate::database;
    use std::time::Duration;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"not a rom").unwrap();
    }

    #[test]
    fn discover_matches_esde_folders_and_skips_root_files() {
        let root = tempfile::tempdir().unwrap();
        touch(&root.path().join("snes/One.sfc"));
        touch(&root.path().join("snes/nested/Two.sfc"));
        touch(&root.path().join("nes/Mario.nes"));
        touch(&root.path().join("mystery/note.txt"));
        touch(&root.path().join("readme.txt"));

        let found = discover_root(root.path());
        let names: Vec<_> = found.iter().map(|folder| folder.folder_name.as_str()).collect();
        assert_eq!(names, ["mystery", "nes", "snes"]);
        let snes = found.iter().find(|folder| folder.folder_name == "snes").unwrap();
        assert_eq!(snes.matched_id.as_deref(), Some("snes"));
        assert_eq!(snes.file_count, 2);
        let mystery = found.iter().find(|folder| folder.folder_name == "mystery").unwrap();
        assert!(mystery.matched_id.is_none());
        assert_eq!(mystery.file_count, 1);
    }

    #[test]
    fn apply_writes_config_and_sqlite_for_the_chosen_folders() {
        let root = tempfile::tempdir().unwrap();
        let _env = XdgEnv::sandbox(root.path());
        let roms = root.path().join("roms");
        touch(&roms.join("snes/One.sfc"));
        touch(&roms.join("snes/nested/Two.sfc"));
        touch(&roms.join("nes/Mario.nes"));

        let choices = vec![
            ImportChoice {
                system_id: "nes".into(),
                path: roms.join("nes"),
            },
            ImportChoice {
                system_id: "snes".into(),
                path: roms.join("snes"),
            },
        ];
        let rx = spawn_apply(choices);
        let mut saw_status = false;
        let (systems, games) = loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                ImportUpdate::Status(text) => {
                    assert!(text.starts_with("Scanning "));
                    saw_status = true;
                }
                ImportUpdate::Done { systems, games } => break (systems, games),
                other => panic!("{other:?}"),
            }
        };
        assert!(saw_status);
        assert_eq!(systems, 2);
        assert_eq!(games, 3);

        let config = config::load_config().unwrap();
        assert_eq!(config.consoles.len(), 2);
        assert_eq!(config.consoles[0].id, "nes");
        assert_eq!(config.consoles[0].rom_dirs, vec![roms.join("nes")]);
        assert!(config.consoles[0].extensions.iter().any(|ext| ext == "nes"));
        assert_eq!(config.consoles[1].id, "snes");
        assert_eq!(config.consoles[1].name, "Nintendo SNES (Super Nintendo)");
        assert!(config.consoles[1].profile.is_none());

        let conn = database::init_db().unwrap();
        let loaded = database::load_games(&conn, None).unwrap();
        let mut titles: Vec<_> = loaded.iter().map(|game| game.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["Mario", "One", "Two"]);
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}
