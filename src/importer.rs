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

#[derive(Debug, Clone)]
pub struct ImportChoice {
    pub system_id: String,
    pub path: PathBuf,
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
    let catalog = catalog::by_id();
    for choice in choices {
        let Some(system) = catalog.get(choice.system_id.as_str()) else {
            continue;
        };
        upsert_console(config, system, &choice.path);
        let console = config
            .consoles
            .iter()
            .find(|c| c.id == choice.system_id)
            .cloned()
            .expect("console just upserted");
        let scanned = scanner::scan_console(&console)?;
        let ids: Vec<_> = scanned.iter().map(|g| g.id.clone()).collect();
        for game in &scanned {
            database::upsert_game(conn, game)?;
        }
        database::remove_missing_games(conn, &console.id, &ids)?;
    }
    config::save_config(config)?;
    Ok(())
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
        media: MediaToggles::default(),
    });
}

pub fn default_roms_root() -> PathBuf {
    dirs_home().join("ROMs")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}
