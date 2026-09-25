use crate::types::{ConsoleId, Game, GameId, Media, MediaKind, Source};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::PathBuf;

pub fn db_path() -> Result<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::with_prefix("retromarchy")?;
    Ok(xdg_dirs.place_data_file("library.db")?)
}

pub fn init_db() -> Result<Connection> {
    let path = db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)
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
    
    Ok(())
}

pub fn upsert_game(conn: &Connection, game: &Game) -> Result<()> {
    conn.execute(
        "INSERT INTO games (id, console, rom, title, crc32, profile, last_played, play_count, play_time)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
            console = excluded.console,
            rom = excluded.rom,
            title = excluded.title,
            crc32 = COALESCE(games.crc32, excluded.crc32),
            profile = COALESCE(games.profile, excluded.profile),
            last_played = COALESCE(games.last_played, excluded.last_played),
            play_count = games.play_count,
            play_time = games.play_time",
        params![
            game.id,
            game.console,
            game.rom.to_string_lossy().to_string(),
            game.title,
            game.crc32,
            game.profile.as_ref(),
            game.last_played.map(|dt| dt.to_rfc3339()),
            game.play_count,
            game.play_time,
        ],
    )?;

    conn.execute(
        "DELETE FROM media WHERE game_id = ?1 AND source = ?2",
        params![game.id, source_to_i32(Source::Local)],
    )?;

    for media in &game.media {
        if media.source == Source::Local {
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

pub fn remove_missing_games(conn: &Connection, console: &ConsoleId, existing_ids: &[GameId]) -> Result<usize> {
    if existing_ids.is_empty() {
        let count = conn.execute("DELETE FROM games WHERE console = ?1", params![console])?;
        return Ok(count);
    }

    let placeholders = existing_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
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
            "SELECT id, console, rom, title, crc32, profile, last_played, play_count, play_time
             FROM games WHERE console = ?1 ORDER BY title",
        )?
    } else {
        conn.prepare(
            "SELECT id, console, rom, title, crc32, profile, last_played, play_count, play_time
             FROM games ORDER BY title",
        )?
    };

    let game_mapper = |row: &rusqlite::Row| {
        Ok(Game {
            id: row.get(0)?,
            console: row.get(1)?,
            rom: PathBuf::from(row.get::<_, String>(2)?),
            title: row.get(3)?,
            crc32: row.get(4)?,
            profile: row.get(5)?,
            last_played: row
                .get::<_, Option<String>>(6)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            media: Vec::new(),
            play_count: row.get::<_, Option<u32>>(7)?.unwrap_or(0),
            play_time: row.get::<_, Option<u32>>(8)?.unwrap_or(0),
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

    Ok(games)
}

fn load_media(conn: &Connection, game_id: &GameId) -> Result<Vec<Media>> {
    let mut stmt = conn.prepare(
        "SELECT kind, path, source FROM media WHERE game_id = ?1",
    )?;

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

pub fn update_last_played(conn: &Connection, game_id: &GameId) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE games SET last_played = ?1 WHERE id = ?2",
        params![now, game_id],
    )?;
    Ok(())
}

pub fn increment_play_stats(conn: &Connection, game_id: &GameId, play_time_seconds: u32) -> Result<()> {
    conn.execute(
        "UPDATE games SET play_count = play_count + 1, play_time = play_time + ?1 WHERE id = ?2",
        params![play_time_seconds, game_id],
    )?;
    Ok(())
}

pub struct LibraryStats {
    pub total_games: u32,
    pub last_played_date: Option<DateTime<Utc>>,
    pub last_played_game: Option<String>,
    pub total_play_count: u32,
    pub total_play_time: u32,
    pub most_played_game: Option<String>,
    pub most_played_count: u32,
}

pub fn get_library_stats(conn: &Connection, console: &ConsoleId) -> Result<LibraryStats> {
    let total_games: u32 = conn.query_row(
        "SELECT COUNT(*) FROM games WHERE console = ?1",
        params![console],
        |row| row.get(0),
    )?;

    let (last_played_date, last_played_game): (Option<String>, Option<String>) = conn.query_row(
        "SELECT last_played, title FROM games WHERE console = ?1 AND last_played IS NOT NULL ORDER BY last_played DESC LIMIT 1",
        params![console],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or((None, None));

    let last_played_date = last_played_date
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let total_play_count: u32 = conn.query_row(
        "SELECT COALESCE(SUM(play_count), 0) FROM games WHERE console = ?1",
        params![console],
        |row| row.get(0),
    )?;

    let total_play_time: u32 = conn.query_row(
        "SELECT COALESCE(SUM(play_time), 0) FROM games WHERE console = ?1",
        params![console],
        |row| row.get(0),
    )?;

    let (most_played_game, most_played_count): (Option<String>, u32) = conn.query_row(
        "SELECT title, play_count FROM games WHERE console = ?1 AND play_count > 0 ORDER BY play_count DESC LIMIT 1",
        params![console],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or((None, 0));

    Ok(LibraryStats {
        total_games,
        last_played_date,
        last_played_game,
        total_play_count,
        total_play_time,
        most_played_game,
        most_played_count,
    })
}

fn media_kind_to_i32(kind: MediaKind) -> i32 {
    match kind {
        MediaKind::BoxArt => 0,
        MediaKind::Screenshot => 1,
        MediaKind::Manual => 2,
        MediaKind::Video => 3,
    }
}

fn i32_to_media_kind(val: i32) -> MediaKind {
    match val {
        0 => MediaKind::BoxArt,
        1 => MediaKind::Screenshot,
        2 => MediaKind::Manual,
        3 => MediaKind::Video,
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
