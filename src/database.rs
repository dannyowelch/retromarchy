use crate::types::{ConsoleId, Game, GameId, GameMetadata, Media, MediaKind, Source};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub fn db_path() -> Result<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::with_prefix("retromarchy")?;
    Ok(xdg_dirs.place_data_file("library.db")?)
}

pub fn init_db() -> Result<Connection> {
    let path = db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    open_db(&path)
}

pub fn open_db(path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open database at {}", path.display()))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS games (
            id TEXT PRIMARY KEY,
            console TEXT NOT NULL,
            rom TEXT NOT NULL,
            title TEXT NOT NULL,
            crc32 INTEGER,
            profile TEXT,
            last_played TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS media (
            game_id TEXT NOT NULL,
            kind INTEGER NOT NULL,
            path TEXT NOT NULL,
            source INTEGER NOT NULL,
            FOREIGN KEY(game_id) REFERENCES games(id) ON DELETE CASCADE
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_games_console ON games(console)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_media_game ON media(game_id)",
        [],
    )?;

    run_migrations(&conn)?;
    fold_custom_titles(&conn)?;

    Ok(conn)
}

fn run_migrations(conn: &Connection) -> Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version < 1 {
        conn.execute(
            "ALTER TABLE games ADD COLUMN play_count INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        conn.execute(
            "ALTER TABLE games ADD COLUMN play_time INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        conn.execute("PRAGMA user_version = 1", [])?;
    }

    if version < 2 {
        conn.execute(
            "ALTER TABLE games ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        conn.execute("PRAGMA user_version = 2", [])?;
    }

    if version < 3 {
        conn.execute(
            "ALTER TABLE games ADD COLUMN title_custom INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        conn.execute("PRAGMA user_version = 3", [])?;
    }

    if version < 4 {
        let tx = conn.unchecked_transaction()?;
        tx.execute("ALTER TABLE games ADD COLUMN user_title TEXT", [])?;
        tx.execute("ALTER TABLE games ADD COLUMN scraped_title TEXT", [])?;
        tx.execute("ALTER TABLE games ADD COLUMN publisher TEXT", [])?;
        tx.execute("ALTER TABLE games ADD COLUMN year INTEGER", [])?;
        tx.execute("ALTER TABLE games ADD COLUMN genre TEXT", [])?;
        tx.execute("ALTER TABLE games ADD COLUMN metadata_scraped_at TEXT", [])?;
        tx.execute("PRAGMA user_version = 4", [])?;
        tx.commit()?;
    }

    Ok(())
}

fn fold_custom_titles(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id, rom, title FROM games WHERE title_custom != 0")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for (id, rom, title) in rows {
        let file_title = crate::scanner::derive_title(Path::new(&rom));
        let trimmed = title.trim();
        if trimmed.is_empty() {
            conn.execute(
                "UPDATE games SET title = ?1, title_custom = 0 WHERE id = ?2",
                params![file_title, id],
            )?;
        } else {
            conn.execute(
                "UPDATE games SET user_title = ?1, title = ?2, title_custom = 0 WHERE id = ?3",
                params![trimmed, file_title, id],
            )?;
        }
    }
    Ok(())
}

pub fn upsert_game(conn: &Connection, game: &Game) -> Result<()> {
    conn.execute(
        "INSERT INTO games (id, console, rom, title, crc32, profile, last_played, play_count, play_time, favorite)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
            console = excluded.console,
            rom = excluded.rom,
            user_title = CASE
                WHEN games.title_custom != 0 AND trim(games.title) != '' THEN trim(games.title)
                ELSE games.user_title
            END,
            title = excluded.title,
            title_custom = 0,
            crc32 = COALESCE(games.crc32, excluded.crc32),
            profile = COALESCE(games.profile, excluded.profile),
            last_played = COALESCE(games.last_played, excluded.last_played),
            play_count = games.play_count,
            play_time = games.play_time",
        params![
            game.id,
            game.console,
            game.rom.to_string_lossy().to_string(),
            game.file_title,
            game.crc32,
            game.profile.as_ref(),
            game.last_played.map(|dt| dt.to_rfc3339()),
            game.play_count,
            game.play_time,
            game.favorite,
        ],
    )?;

    conn.execute(
        "DELETE FROM media WHERE game_id = ?1 AND source = ?2",
        params![game.id, source_to_i32(Source::Local)],
    )?;

    for media in &game.media {
        if media.source == Source::Local {
            conn.execute(
                "DELETE FROM media WHERE game_id = ?1 AND kind = ?2",
                params![game.id, media_kind_to_i32(media.kind)],
            )?;
            conn.execute(
                "INSERT INTO media (game_id, kind, path, source) VALUES (?1, ?2, ?3, ?4)",
                params![
                    game.id,
                    media_kind_to_i32(media.kind),
                    media.path.to_string_lossy().to_string(),
                    source_to_i32(media.source),
                ],
            )?;
        }
    }

    Ok(())
}

/// Upsert a console scan and drop rows whose ROM is gone.
/// Play counts, favorites, and custom titles stay on the rows that remain.
pub fn replace_scanned_games(
    conn: &Connection,
    console: &ConsoleId,
    scanned: &[Game],
) -> Result<Vec<Game>> {
    let ids: Vec<_> = scanned.iter().map(|game| game.id.clone()).collect();
    for game in scanned {
        upsert_game(conn, game)?;
    }
    remove_missing_games(conn, console, &ids)?;
    load_games(conn, Some(console))
}

pub fn remove_missing_games(
    conn: &Connection,
    console: &ConsoleId,
    existing_ids: &[GameId],
) -> Result<usize> {
    if existing_ids.is_empty() {
        let count = conn.execute("DELETE FROM games WHERE console = ?1", params![console])?;
        return Ok(count);
    }

    let placeholders = existing_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let query = format!(
        "DELETE FROM games WHERE console = ?1 AND id NOT IN ({})",
        placeholders
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = vec![console];
    for id in existing_ids {
        params.push(id);
    }
    let count = conn.execute(&query, params.as_slice())?;
    Ok(count)
}

pub fn load_games(conn: &Connection, console: Option<&ConsoleId>) -> Result<Vec<Game>> {
    let mut stmt = if let Some(_console_id) = console {
        conn.prepare(
            "SELECT id, console, rom, title, crc32, profile, last_played, play_count, play_time, favorite,
                    user_title, scraped_title, publisher, year, genre, metadata_scraped_at
             FROM games WHERE console = ?1",
        )?
    } else {
        conn.prepare(
            "SELECT id, console, rom, title, crc32, profile, last_played, play_count, play_time, favorite,
                    user_title, scraped_title, publisher, year, genre, metadata_scraped_at
             FROM games",
        )?
    };

    let game_mapper = |row: &rusqlite::Row| {
        let scraped_at: Option<String> = row.get(15)?;
        let metadata = if scraped_at.is_some() {
            Some(GameMetadata {
                title: nonempty(row.get(11)?),
                publisher: nonempty(row.get(12)?),
                year: row
                    .get::<_, Option<i64>>(13)?
                    .and_then(|year| u32::try_from(year).ok()),
                genre: nonempty(row.get(14)?),
            })
        } else {
            None
        };
        Ok(Game {
            id: row.get(0)?,
            console: row.get(1)?,
            rom: PathBuf::from(row.get::<_, String>(2)?),
            file_title: row.get(3)?,
            user_title: nonempty(row.get(10)?),
            metadata,
            crc32: row.get(4)?,
            profile: row.get(5)?,
            last_played: row
                .get::<_, Option<String>>(6)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            media: Vec::new(),
            play_count: row.get::<_, Option<u32>>(7)?.unwrap_or(0),
            play_time: row.get::<_, Option<u32>>(8)?.unwrap_or(0),
            favorite: row.get::<_, i64>(9)? != 0,
        })
    };

    let mut games = if let Some(console_id) = console {
        stmt.query_map([console_id], game_mapper)?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], game_mapper)?
            .collect::<Result<Vec<_>, _>>()?
    };

    for game in &mut games {
        game.media = load_media(conn, &game.id)?;
    }

    games.sort_by(|a, b| {
        a.display_title()
            .cmp(b.display_title())
            .then_with(|| a.id.cmp(&b.id))
    });

    Ok(games)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn load_media(conn: &Connection, game_id: &GameId) -> Result<Vec<Media>> {
    let mut stmt = conn.prepare("SELECT kind, path, source FROM media WHERE game_id = ?1")?;

    let media = stmt
        .query_map([game_id], |row| {
            Ok(Media {
                kind: i32_to_media_kind(row.get(0)?),
                path: PathBuf::from(row.get::<_, String>(1)?),
                source: i32_to_source(row.get(2)?),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(media)
}

pub fn set_game_media(conn: &Connection, game_id: &GameId, media: &Media) -> Result<()> {
    conn.execute(
        "DELETE FROM media WHERE game_id = ?1 AND kind = ?2",
        params![game_id, media_kind_to_i32(media.kind)],
    )?;
    conn.execute(
        "INSERT INTO media (game_id, kind, path, source) VALUES (?1, ?2, ?3, ?4)",
        params![
            game_id,
            media_kind_to_i32(media.kind),
            media.path.to_string_lossy().to_string(),
            source_to_i32(media.source),
        ],
    )?;
    Ok(())
}

pub fn update_last_played(conn: &Connection, game_id: &GameId) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE games SET last_played = ?1 WHERE id = ?2",
        params![now, game_id],
    )?;
    Ok(())
}

pub fn set_favorite(conn: &Connection, game_id: &GameId, favorite: bool) -> Result<()> {
    conn.execute(
        "UPDATE games SET favorite = ?1 WHERE id = ?2",
        params![favorite, game_id],
    )?;
    Ok(())
}

pub fn set_game_title(conn: &Connection, game_id: &GameId, title: &str) -> Result<Option<String>> {
    let stored = nonempty(Some(title.to_string()));
    conn.execute(
        "UPDATE games SET user_title = ?1, title_custom = 0 WHERE id = ?2",
        params![stored, game_id],
    )?;
    Ok(stored)
}

pub fn set_game_metadata(
    conn: &Connection,
    game_id: &GameId,
    metadata: &GameMetadata,
) -> Result<()> {
    let scraped_at = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE games SET scraped_title = ?1, publisher = ?2, year = ?3, genre = ?4, metadata_scraped_at = ?5 WHERE id = ?6",
        params![
            nonempty(metadata.title.clone()),
            nonempty(metadata.publisher.clone()),
            metadata.year.map(i64::from),
            nonempty(metadata.genre.clone()),
            scraped_at,
            game_id,
        ],
    )?;
    Ok(())
}

/// Drop the game and its media rows. Does not touch files on disk.
pub fn delete_game(conn: &Connection, game_id: &GameId) -> Result<()> {
    conn.execute("DELETE FROM media WHERE game_id = ?1", params![game_id])?;
    conn.execute("DELETE FROM games WHERE id = ?1", params![game_id])?;
    Ok(())
}

/// Drop every game for a system, and the media rows. Does not touch files on disk.
pub fn delete_console_library(conn: &Connection, console: &ConsoleId) -> Result<usize> {
    let mut stmt = conn.prepare("SELECT id FROM games WHERE console = ?1")?;
    let ids = stmt
        .query_map(params![console], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for id in &ids {
        conn.execute("DELETE FROM media WHERE game_id = ?1", params![id])?;
    }
    let count = conn.execute("DELETE FROM games WHERE console = ?1", params![console])?;
    Ok(count)
}

pub fn increment_play_stats(
    conn: &Connection,
    game_id: &GameId,
    play_time_seconds: u32,
) -> Result<()> {
    conn.execute(
        "UPDATE games SET play_count = play_count + 1, play_time = play_time + ?1 WHERE id = ?2",
        params![play_time_seconds, game_id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStats {
    pub total_games: u32,
    pub last_played_date: Option<DateTime<Utc>>,
    pub last_played_game: Option<String>,
    pub total_play_count: u32,
    pub total_play_time: u32,
    pub most_played_game: Option<String>,
    pub most_played_count: u32,
}

fn media_kind_to_i32(kind: MediaKind) -> i32 {
    match kind {
        MediaKind::BoxArt => 0,
        MediaKind::Screenshot => 1,
        MediaKind::Manual => 2,
        MediaKind::Video => 3,
        MediaKind::TitleScreen => 4,
    }
}

fn i32_to_media_kind(val: i32) -> MediaKind {
    match val {
        0 => MediaKind::BoxArt,
        1 => MediaKind::Screenshot,
        2 => MediaKind::Manual,
        3 => MediaKind::Video,
        4 => MediaKind::TitleScreen,
        _ => MediaKind::BoxArt,
    }
}

fn source_to_i32(source: Source) -> i32 {
    match source {
        Source::ScreenScraper => 0,
        Source::TheGamesDb => 1,
        Source::Local => 2,
    }
}

fn i32_to_source(val: i32) -> Source {
    match val {
        0 => Source::ScreenScraper,
        1 => Source::TheGamesDb,
        2 => Source::Local,
        _ => Source::Local,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn sample(id: &str, title: &str, favorite: bool) -> Game {
        Game {
            id: id.to_string(),
            console: "snes".to_string(),
            rom: PathBuf::from(format!("/tmp/{id}.sfc")),
            file_title: title.to_string(),
            user_title: None,
            metadata: None,
            crc32: None,
            profile: None,
            media: Vec::new(),
            last_played: None,
            play_count: 0,
            play_time: 0,
            favorite,
        }
    }

    #[test]
    fn play_stats_survive_a_rescan_and_missing_roms_drop() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("library.db")).unwrap();
        upsert_game(&conn, &sample("keep", "Kept", false)).unwrap();
        upsert_game(&conn, &sample("gone", "Gone", false)).unwrap();
        increment_play_stats(&conn, &"keep".to_string(), 9).unwrap();
        update_last_played(&conn, &"keep".to_string()).unwrap();

        let games = replace_scanned_games(
            &conn,
            &"snes".to_string(),
            &[
                sample("keep", "Kept", false),
                sample("new", "New Game", false),
            ],
        )
        .unwrap();
        assert_eq!(
            games
                .iter()
                .map(|game| game.id.as_str())
                .collect::<Vec<_>>(),
            vec!["keep", "new"]
        );
        let keep = games.iter().find(|game| game.id == "keep").unwrap();
        assert_eq!(keep.play_count, 1);
        assert_eq!(keep.play_time, 9);
        assert!(keep.last_played.is_some());
        assert!(games.iter().all(|game| game.id != "gone"));
    }

    #[test]
    fn favorite_survives_rescan_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("library.db")).unwrap();
        upsert_game(&conn, &sample("a", "Alpha", false)).unwrap();
        set_favorite(&conn, &"a".to_string(), true).unwrap();
        upsert_game(&conn, &sample("a", "Alpha", false)).unwrap();
        let games = load_games(&conn, None).unwrap();
        assert!(games[0].favorite);
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
                .unwrap(),
            4
        );
    }

    #[test]
    fn renamed_title_survives_rescan_and_delete_drops_media() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("library.db")).unwrap();
        upsert_game(&conn, &sample("a", "Alpha", false)).unwrap();
        upsert_game(&conn, &sample("b", "Beta", false)).unwrap();
        set_game_title(&conn, &"a".to_string(), "Alpha Renamed").unwrap();
        set_game_media(
            &conn,
            &"a".to_string(),
            &Media {
                kind: MediaKind::BoxArt,
                path: PathBuf::from("/tmp/a/box_art.png"),
                source: Source::ScreenScraper,
            },
        )
        .unwrap();

        upsert_game(&conn, &sample("a", "Alpha From Scan", false)).unwrap();
        upsert_game(&conn, &sample("b", "Beta Cleaned", false)).unwrap();
        let games = load_games(&conn, None).unwrap();
        let alpha = games.iter().find(|game| game.id == "a").unwrap();
        let beta = games.iter().find(|game| game.id == "b").unwrap();
        assert_eq!(alpha.display_title(), "Alpha Renamed");
        assert_eq!(alpha.file_title, "Alpha From Scan");
        assert_eq!(alpha.media.len(), 1);
        assert_eq!(beta.display_title(), "Beta Cleaned");
        assert_eq!(beta.file_title, "Beta Cleaned");

        let rom = dir.path().join("keep.sfc");
        fs::write(&rom, b"stay").unwrap();
        delete_console_library(&conn, &"snes".to_string()).unwrap();
        assert!(rom.is_file());
        let left = load_games(&conn, None).unwrap();
        assert!(left.is_empty());
        let media_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM media", [], |row| row.get(0))
            .unwrap();
        assert_eq!(media_rows, 0);

        delete_game(&conn, &"a".to_string()).unwrap();
        let left = load_games(&conn, None).unwrap();
        assert!(left.is_empty());
        let media_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM media WHERE game_id = 'a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(media_rows, 0);
    }

    #[test]
    fn version_one_library_gains_favorite_default_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "CREATE TABLE games (
                    id TEXT PRIMARY KEY,
                    console TEXT NOT NULL,
                    rom TEXT NOT NULL,
                    title TEXT NOT NULL,
                    crc32 INTEGER,
                    profile TEXT,
                    last_played TEXT,
                    play_count INTEGER NOT NULL DEFAULT 0,
                    play_time INTEGER NOT NULL DEFAULT 0
                )",
                [],
            )
            .unwrap();
            conn.execute("PRAGMA user_version = 1", []).unwrap();
            conn.execute(
                "INSERT INTO games (id, console, rom, title) VALUES ('g', 'snes', '/r.sfc', 'Chrono')",
                [],
            )
            .unwrap();
        }
        let conn = open_db(&path).unwrap();
        let games = load_games(&conn, Some(&"snes".to_string())).unwrap();
        assert_eq!(games.len(), 1);
        assert!(!games[0].favorite);
        set_favorite(&conn, &games[0].id, true).unwrap();
        assert!(load_games(&conn, None).unwrap()[0].favorite);
    }

    fn v3_library(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE games (
                id TEXT PRIMARY KEY,
                console TEXT NOT NULL,
                rom TEXT NOT NULL,
                title TEXT NOT NULL,
                crc32 INTEGER,
                profile TEXT,
                last_played TEXT,
                play_count INTEGER NOT NULL DEFAULT 0,
                play_time INTEGER NOT NULL DEFAULT 0,
                favorite INTEGER NOT NULL DEFAULT 0,
                title_custom INTEGER NOT NULL DEFAULT 0
            );
            PRAGMA user_version = 3;",
        )
        .unwrap();
    }

    #[test]
    fn version_three_library_folds_custom_titles_and_keeps_play_stats() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        {
            v3_library(&path);
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "INSERT INTO games (id, console, rom, title, title_custom, favorite, play_count, play_time, last_played)
                 VALUES ('chrono', 'snes', '/r/chrono_trigger_(USA).sfc', 'My Chrono', 1, 1, 4, 90, '2020-05-06T07:08:09Z'),
                        ('plain', 'snes', '/r/other.sfc', 'Plain', 0, 0, 1, 2, NULL)",
                [],
            )
            .unwrap();
        }
        let conn = open_db(&path).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
                .unwrap(),
            4
        );
        let games = load_games(&conn, None).unwrap();
        let chrono = games.iter().find(|game| game.id == "chrono").unwrap();
        let plain = games.iter().find(|game| game.id == "plain").unwrap();
        assert_eq!(chrono.display_title(), "My Chrono");
        assert_eq!(chrono.file_title, "chrono trigger");
        assert_eq!(chrono.user_title.as_deref(), Some("My Chrono"));
        assert!(chrono.favorite);
        assert_eq!(chrono.play_count, 4);
        assert_eq!(chrono.play_time, 90);
        assert!(chrono.last_played.is_some());
        assert!(plain.user_title.is_none());
        assert_eq!(plain.file_title, "Plain");
        assert_eq!(plain.play_count, 1);
        assert_eq!(
            set_game_title(&conn, &"chrono".to_string(), "").unwrap(),
            None
        );
        let cleared = load_games(&conn, None)
            .unwrap()
            .into_iter()
            .find(|game| game.id == "chrono")
            .unwrap();
        assert_eq!(cleared.display_title(), "chrono trigger");
        assert!(cleared.user_title.is_none());
        assert!(cleared.favorite);
        assert_eq!(cleared.play_count, 4);
        assert_eq!(cleared.play_time, 90);
        drop(conn);
        let conn = open_db(&path).unwrap();
        let again = load_games(&conn, None)
            .unwrap()
            .into_iter()
            .find(|game| game.id == "chrono")
            .unwrap();
        assert_eq!(again.display_title(), "chrono trigger");
        assert_eq!(again.file_title, "chrono trigger");
        assert!(again.user_title.is_none());
        assert!(again.favorite);
        assert_eq!(again.play_count, 4);
        assert_eq!(again.play_time, 90);
        assert!(again.last_played.is_some());
    }

    #[test]
    fn failed_v4_transaction_leaves_version_three() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        v3_library(&path);
        {
            let conn = Connection::open(&path).unwrap();
            let failed = (|| -> Result<()> {
                let tx = conn.unchecked_transaction()?;
                tx.execute("ALTER TABLE games ADD COLUMN scraped_title TEXT", [])?;
                anyhow::bail!("stopped before commit");
            })();
            assert!(failed.is_err());
        }
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
                .unwrap(),
            3
        );
        let mut stmt = conn.prepare("PRAGMA table_info(games)").unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!names.iter().any(|name| name == "scraped_title"));
    }

    #[test]
    fn upsert_keeps_user_title_and_metadata_and_updates_file_title() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("library.db")).unwrap();
        upsert_game(&conn, &sample("a", "Old File", false)).unwrap();
        assert_eq!(
            set_game_title(&conn, &"a".to_string(), "Mine").unwrap(),
            Some("Mine".into())
        );
        set_game_metadata(
            &conn,
            &"a".to_string(),
            &GameMetadata {
                title: Some("Scraped".into()),
                publisher: Some("Square".into()),
                year: Some(1995),
                genre: Some("RPG".into()),
            },
        )
        .unwrap();
        upsert_game(&conn, &sample("a", "New File", false)).unwrap();
        let game = load_games(&conn, None).unwrap().pop().unwrap();
        assert_eq!(game.file_title, "New File");
        assert_eq!(game.user_title.as_deref(), Some("Mine"));
        assert_eq!(game.display_title(), "Mine");
        let metadata = game.metadata.unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Scraped"));
        assert_eq!(metadata.publisher.as_deref(), Some("Square"));
        assert_eq!(metadata.year, Some(1995));
        assert_eq!(metadata.genre.as_deref(), Some("RPG"));
    }

    #[test]
    fn upsert_folds_a_legacy_custom_title_before_the_scan_title() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("library.db")).unwrap();
        upsert_game(&conn, &sample("a", "chrono trigger", false)).unwrap();
        conn.execute(
            "UPDATE games SET title = '  My Chrono  ', title_custom = 1 WHERE id = 'a'",
            [],
        )
        .unwrap();
        upsert_game(&conn, &sample("a", "chrono trigger", false)).unwrap();
        let game = load_games(&conn, None).unwrap().pop().unwrap();
        assert_eq!(game.user_title.as_deref(), Some("My Chrono"));
        assert_eq!(game.file_title, "chrono trigger");
        assert_eq!(game.display_title(), "My Chrono");
        let custom: i32 = conn
            .query_row("SELECT title_custom FROM games WHERE id = 'a'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(custom, 0);
        upsert_game(&conn, &sample("a", "other name", false)).unwrap();
        let game = load_games(&conn, None).unwrap().pop().unwrap();
        assert_eq!(game.file_title, "other name");
        assert_eq!(game.user_title.as_deref(), Some("My Chrono"));
    }

    #[test]
    fn metadata_marker_is_the_scraped_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("library.db")).unwrap();
        upsert_game(&conn, &sample("fresh", "Fresh", false)).unwrap();
        upsert_game(&conn, &sample("stray", "Stray", false)).unwrap();
        conn.execute(
            "UPDATE games SET scraped_title = 'Ghost' WHERE id = 'stray'",
            [],
        )
        .unwrap();
        let stray = load_games(&conn, None)
            .unwrap()
            .into_iter()
            .find(|game| game.id == "stray")
            .unwrap();
        assert!(stray.metadata.is_none());
        assert_eq!(stray.display_title(), "Stray");
        set_game_metadata(
            &conn,
            &"fresh".to_string(),
            &GameMetadata {
                title: Some("Named".into()),
                publisher: None,
                year: Some(1995),
                genre: None,
            },
        )
        .unwrap();
        let fresh = load_games(&conn, None)
            .unwrap()
            .into_iter()
            .find(|game| game.id == "fresh")
            .unwrap();
        let metadata = fresh.metadata.unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Named"));
        assert!(metadata.publisher.is_none());
        assert_eq!(metadata.year, Some(1995));
        assert!(metadata.genre.is_none());
    }

    #[test]
    fn rename_wins_over_scraped_title() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("library.db")).unwrap();
        upsert_game(&conn, &sample("a", "file", false)).unwrap();
        set_game_title(&conn, &"a".to_string(), "Mine").unwrap();
        set_game_metadata(
            &conn,
            &"a".to_string(),
            &GameMetadata {
                title: Some("Scraped".into()),
                ..GameMetadata::default()
            },
        )
        .unwrap();
        let game = load_games(&conn, None).unwrap().pop().unwrap();
        assert_eq!(game.display_title(), "Mine");
        assert_eq!(game.metadata.unwrap().title.as_deref(), Some("Scraped"));
    }

    #[test]
    fn load_sorts_by_display_title_then_id() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("library.db")).unwrap();
        upsert_game(&conn, &sample("zeta", "zeta", false)).unwrap();
        upsert_game(&conn, &sample("beta", "Beta", false)).unwrap();
        upsert_game(&conn, &sample("aardvark", "Aardvark", false)).unwrap();
        upsert_game(&conn, &sample("b", "Same", false)).unwrap();
        upsert_game(&conn, &sample("a", "Same", false)).unwrap();
        set_game_metadata(
            &conn,
            &"zeta".to_string(),
            &GameMetadata {
                title: Some("Alpha".into()),
                ..GameMetadata::default()
            },
        )
        .unwrap();
        set_game_title(&conn, &"aardvark".to_string(), "Zed").unwrap();
        let titles: Vec<_> = load_games(&conn, None)
            .unwrap()
            .into_iter()
            .map(|game| (game.display_title().to_string(), game.id))
            .collect();
        assert_eq!(
            titles,
            vec![
                ("Alpha".into(), "zeta".into()),
                ("Beta".into(), "beta".into()),
                ("Same".into(), "a".into()),
                ("Same".into(), "b".into()),
                ("Zed".into(), "aardvark".into()),
            ]
        );
    }
}
