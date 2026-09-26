//! Per-game menu. GTK opens it with a right-click, the Menu key, Shift+F10,
//! or Select (Back / View). Scrape, rename, and delete then follow that window.
//! The header Scrape button and `s` open this same scrape dialog.

use crate::browse::{stats_of, Browse, LibraryKind};
use crate::database;
use crate::gamepad::NavDir;
use crate::scraper::{NameSearch, ScrapeCandidate};
use crate::types::{DeleteOptions, Game, GameAction};

/// What is in front of the grid. One value, so the menu and a dialog cannot both be open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    None,
    Menu(Menu),
    Rename(Rename),
    Delete(DeletePrompt),
    Scrape(ScrapePrompt),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub cursor: usize,
}

/// Single-line title or search box. `replace` is the GTK selection of the prefilled text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEdit {
    pub text: String,
    pub caret: usize,
    pub replace: bool,
}

impl LineEdit {
    fn selected(text: String) -> Self {
        let caret = text.chars().count();
        Self {
            text,
            caret,
            replace: true,
        }
    }

    pub(crate) fn plain(text: String) -> Self {
        let caret = text.chars().count();
        Self {
            text,
            caret,
            replace: false,
        }
    }

    pub fn caret_line(&self) -> CaretLine {
        if self.replace {
            return CaretLine {
                head: self.text.clone(),
                tail: String::new(),
                selected: true,
            };
        }
        let (head, tail) = split_chars(&self.text, self.caret);
        CaretLine {
            head,
            tail,
            selected: false,
        }
    }

    pub(crate) fn insert(&mut self, extra: &str) {
        if extra.is_empty() {
            return;
        }
        if self.replace {
            self.text = extra.to_string();
            self.caret = self.text.chars().count();
            self.replace = false;
            return;
        }
        let at = byte_index(&self.text, self.caret);
        self.text.insert_str(at, extra);
        self.caret += extra.chars().count();
    }

    pub(crate) fn backspace(&mut self) {
        if self.replace {
            self.text.clear();
            self.caret = 0;
            self.replace = false;
            return;
        }
        if self.caret == 0 {
            return;
        }
        self.caret -= 1;
        let start = byte_index(&self.text, self.caret);
        let end = byte_index(&self.text, self.caret + 1);
        self.text.replace_range(start..end, "");
    }

    pub(crate) fn delete_forward(&mut self) {
        if self.replace {
            self.text.clear();
            self.caret = 0;
            self.replace = false;
            return;
        }
        let start = byte_index(&self.text, self.caret);
        let Some(ch) = self.text[start..].chars().next() else {
            return;
        };
        let end = start + ch.len_utf8();
        self.text.replace_range(start..end, "");
    }

    pub(crate) fn move_caret(&mut self, delta: isize) {
        if self.replace {
            let len = self.text.chars().count();
            self.caret = if delta < 0 { 0 } else { len };
            self.replace = false;
            return;
        }
        let len = self.text.chars().count() as isize;
        self.caret = (self.caret as isize + delta).clamp(0, len) as usize;
    }

    fn at_end(&self) -> bool {
        self.replace || self.caret >= self.text.chars().count()
    }
}

/// How the shell paints the line. `selected` is the whole prefilled title.
pub struct CaretLine {
    pub head: String,
    pub tail: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameSlot {
    Title,
    Cancel,
    Save,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub game_id: String,
    pub edit: LineEdit,
    pub slot: RenameSlot,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteSlot {
    Rom,
    Assets,
    Cancel,
    Confirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePrompt {
    pub game_id: String,
    pub title: String,
    pub options: DeleteOptions,
    pub slot: DeleteSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrapeSlot {
    Query,
    Search,
    Results,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapePrompt {
    pub game_id: String,
    pub console_id: String,
    pub edit: LineEdit,
    pub slot: ScrapeSlot,
    pub cursor: usize,
    pub candidates: Vec<ScrapeCandidate>,
    pub message: String,
    pub searching: bool,
}

/// Work the shell still has to do after the overlay changes. Rename and delete
/// stay on the overlay until [`Browse::commit_rename`] or [`Browse::commit_delete`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayCommand {
    None,
    CommitRename,
    CommitDelete,
    Search {
        query: String,
        console_id: String,
    },
    Apply {
        game_id: String,
        candidate: ScrapeCandidate,
    },
}

/// Mouse target. The shell aims, then confirms when the click chooses an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aim {
    Menu(usize),
    Rename(RenameSlot),
    Delete(DeleteSlot),
    ScrapeQuery,
    ScrapeSearch,
    ScrapeResult(usize),
}

/// Game removed from the library row. The shell may still delete files.
pub struct RemovedGame {
    pub game: Game,
    pub options: DeleteOptions,
}

/// Menu key or Shift+F10. `modified` is Ctrl, Alt, or the platform key, not Shift.
pub fn game_menu_key(key: &str, shift: bool, modified: bool) -> bool {
    if modified {
        return false;
    }
    let key = key.to_ascii_lowercase();
    key == "menu" || key == "context_menu" || (shift && key == "f10")
}

impl Browse {
    pub fn overlay_open(&self) -> bool {
        !matches!(self.overlay, Overlay::None)
    }

    pub fn menu_open(&self) -> bool {
        matches!(self.overlay, Overlay::Menu(_))
    }

    pub fn menu_cursor(&self) -> Option<usize> {
        match &self.overlay {
            Overlay::Menu(menu) => Some(menu.cursor),
            _ => None,
        }
    }

    /// The title field or the scrape query is taking characters.
    pub fn accepts_text(&self) -> bool {
        match &self.overlay {
            Overlay::Rename(rename) => rename.slot == RenameSlot::Title,
            Overlay::Scrape(scrape) => scrape.slot == ScrapeSlot::Query,
            _ => false,
        }
    }

    /// Right-click, Menu, Shift+F10, or Select. The games pane must be focused.
    pub fn open_game_menu(&mut self) -> bool {
        if self.pane != crate::browse::Pane::Grid || self.selected_game().is_none() {
            self.status = "Select a game.".into();
            return false;
        }
        self.overlay = Overlay::Menu(Menu { cursor: 0 });
        true
    }

    pub fn close_overlay(&mut self) {
        self.overlay = Overlay::None;
    }

    pub fn aim(&mut self, aim: Aim) {
        match aim {
            Aim::Menu(index) => {
                if let Overlay::Menu(menu) = &mut self.overlay {
                    if index < GameAction::ALL.len() {
                        menu.cursor = index;
                    }
                }
            }
            Aim::Rename(slot) => {
                if let Overlay::Rename(rename) = &mut self.overlay {
                    rename.slot = slot;
                }
            }
            Aim::Delete(slot) => {
                if let Overlay::Delete(prompt) = &mut self.overlay {
                    prompt.slot = slot;
                }
            }
            Aim::ScrapeQuery => {
                if let Overlay::Scrape(prompt) = &mut self.overlay {
                    prompt.slot = ScrapeSlot::Query;
                }
            }
            Aim::ScrapeSearch => {
                if let Overlay::Scrape(prompt) = &mut self.overlay {
                    prompt.slot = ScrapeSlot::Search;
                }
            }
            Aim::ScrapeResult(index) => {
                if let Overlay::Scrape(prompt) = &mut self.overlay {
                    if prompt.candidates.is_empty() {
                        return;
                    }
                    prompt.slot = ScrapeSlot::Results;
                    prompt.cursor = index.min(prompt.candidates.len() - 1);
                }
            }
        }
    }

    pub fn set_delete_flag(&mut self, assets: bool, on: bool) {
        let Overlay::Delete(prompt) = &mut self.overlay else {
            return;
        };
        if assets {
            prompt.slot = DeleteSlot::Assets;
            prompt.options.scraped_assets = on;
        } else {
            prompt.slot = DeleteSlot::Rom;
            prompt.options.rom_file = on;
        }
    }

    pub fn move_overlay(&mut self, dir: NavDir) {
        match &mut self.overlay {
            Overlay::Menu(menu) => move_menu(menu, dir),
            Overlay::Rename(rename) => move_rename(rename, dir),
            Overlay::Delete(prompt) => move_delete(prompt, dir),
            Overlay::Scrape(prompt) => move_scrape(prompt, dir),
            Overlay::None => {}
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        match &mut self.overlay {
            Overlay::Rename(rename) if rename.slot == RenameSlot::Title => {
                rename.error = None;
                rename.edit.insert(text);
            }
            Overlay::Scrape(prompt) if prompt.slot == ScrapeSlot::Query => {
                prompt.edit.insert(text);
            }
            _ => {}
        }
    }

    pub fn backspace(&mut self) {
        match &mut self.overlay {
            Overlay::Rename(rename) if rename.slot == RenameSlot::Title => {
                rename.error = None;
                rename.edit.backspace();
            }
            Overlay::Scrape(prompt) if prompt.slot == ScrapeSlot::Query => {
                prompt.edit.backspace();
            }
            _ => {}
        }
    }

    pub fn delete_forward(&mut self) {
        match &mut self.overlay {
            Overlay::Rename(rename) if rename.slot == RenameSlot::Title => {
                rename.error = None;
                rename.edit.delete_forward();
            }
            Overlay::Scrape(prompt) if prompt.slot == ScrapeSlot::Query => {
                prompt.edit.delete_forward();
            }
            _ => {}
        }
    }

    /// Enter, A, or a click that chooses the aimed row.
    /// `scrape_busy` is an apply already writing artwork.
    pub fn confirm_overlay(&mut self, scrape_busy: bool) -> OverlayCommand {
        let step = match &self.overlay {
            Overlay::None => ConfirmStep::Closed,
            Overlay::Menu(menu) => ConfirmStep::Menu(GameAction::from_index(menu.cursor as i32)),
            Overlay::Rename(rename) => {
                ConfirmStep::Rename(rename.slot, rename.edit.text.trim().is_empty())
            }
            Overlay::Delete(prompt) => ConfirmStep::Delete(prompt.slot),
            Overlay::Scrape(prompt) => ConfirmStep::Scrape(prompt.slot),
        };
        match step {
            ConfirmStep::Closed => OverlayCommand::None,
            ConfirmStep::Menu(action) => {
                let Some(game) = self.selected_game().cloned() else {
                    self.status = "Select a game.".into();
                    self.overlay = Overlay::None;
                    return OverlayCommand::None;
                };
                match action {
                    Some(GameAction::Scrape) => self.begin_scrape(&game),
                    Some(GameAction::Rename) => {
                        self.begin_rename(&game);
                        OverlayCommand::None
                    }
                    Some(GameAction::Delete) => {
                        self.begin_delete(&game);
                        OverlayCommand::None
                    }
                    None => OverlayCommand::None,
                }
            }
            ConfirmStep::Rename(RenameSlot::Cancel, _) => {
                self.overlay = Overlay::None;
                OverlayCommand::None
            }
            ConfirmStep::Rename(_, true) => {
                if let Overlay::Rename(rename) = &mut self.overlay {
                    rename.error = Some("Enter a title.".into());
                }
                OverlayCommand::None
            }
            ConfirmStep::Rename(_, false) => OverlayCommand::CommitRename,
            ConfirmStep::Delete(DeleteSlot::Cancel) => {
                self.overlay = Overlay::None;
                OverlayCommand::None
            }
            ConfirmStep::Delete(DeleteSlot::Rom) => {
                if let Overlay::Delete(prompt) = &mut self.overlay {
                    prompt.options.rom_file = !prompt.options.rom_file;
                }
                OverlayCommand::None
            }
            ConfirmStep::Delete(DeleteSlot::Assets) => {
                if let Overlay::Delete(prompt) = &mut self.overlay {
                    prompt.options.scraped_assets = !prompt.options.scraped_assets;
                }
                OverlayCommand::None
            }
            ConfirmStep::Delete(DeleteSlot::Confirm) => OverlayCommand::CommitDelete,
            ConfirmStep::Scrape(ScrapeSlot::Query | ScrapeSlot::Search) => {
                self.begin_listed_search()
            }
            ConfirmStep::Scrape(ScrapeSlot::Results) => self.take_candidate(scrape_busy),
        }
    }

    pub fn commit_rename(&mut self, conn: Option<&rusqlite::Connection>) -> bool {
        let Overlay::Rename(rename) = &self.overlay else {
            return false;
        };
        let title = rename.edit.text.trim().to_string();
        let id = rename.game_id.clone();
        if title.is_empty() {
            if let Overlay::Rename(rename) = &mut self.overlay {
                rename.error = Some("Enter a title.".into());
            }
            return false;
        }
        if self.library.kind == LibraryKind::Demo {
            self.overlay = Overlay::None;
            self.status = "Demo library. Rename is not saved.".into();
            return false;
        }
        let Some(conn) = conn else {
            self.overlay = Overlay::None;
            self.status = "Could not rename the game.".into();
            return false;
        };
        if database::set_game_title(conn, &id, &title).is_err() {
            self.overlay = Overlay::None;
            self.status = "Could not rename the game.".into();
            return false;
        }
        self.rewrite_title(&id, &title);
        self.overlay = Overlay::None;
        if self.status.starts_with("Could not rename")
            || self.status.starts_with("Demo library. Rename")
        {
            self.status.clear();
        }
        true
    }

    pub fn commit_delete(&mut self, conn: Option<&rusqlite::Connection>) -> Option<RemovedGame> {
        let Overlay::Delete(prompt) = &self.overlay else {
            return None;
        };
        let id = prompt.game_id.clone();
        let options = prompt.options;
        if self.library.kind == LibraryKind::Demo {
            self.overlay = Overlay::None;
            self.status = "Demo library. Delete is not saved.".into();
            return None;
        }
        let Some(conn) = conn else {
            self.overlay = Overlay::None;
            self.status = "Could not remove the game from the library.".into();
            return None;
        };
        if let Err(err) = database::delete_game(conn, &id) {
            self.overlay = Overlay::None;
            self.status = format!("Could not remove the game from the library: {err}");
            return None;
        }
        let Some(game) = self.detach_game(&id) else {
            self.overlay = Overlay::None;
            self.status = "Could not remove the game from the library.".into();
            return None;
        };
        self.overlay = Overlay::None;
        self.status = format!("Removed {} from the library.", game.title);
        Some(RemovedGame { game, options })
    }

    pub fn finish_search(&mut self, outcome: NameSearch) {
        let Overlay::Scrape(prompt) = &mut self.overlay else {
            return;
        };
        prompt.searching = false;
        prompt.cursor = 0;
        prompt.message = search_message(&outcome);
        prompt.candidates = outcome.candidates;
    }

    pub fn fail_search(&mut self, message: String) {
        let Overlay::Scrape(prompt) = &mut self.overlay else {
            self.status = message;
            return;
        };
        prompt.searching = false;
        prompt.message = message;
    }

    pub fn game_by_id(&self, id: &str) -> Option<&Game> {
        self.library
            .shelves
            .iter()
            .flat_map(|shelf| shelf.games.iter())
            .find(|game| game.id == id)
    }

    pub fn remember_media(&mut self, game_id: &str, media: &crate::types::Media) {
        for shelf in &mut self.library.shelves {
            let Some(game) = shelf.games.iter_mut().find(|game| game.id == game_id) else {
                continue;
            };
            if let Some(slot) = game.media.iter_mut().find(|item| item.kind == media.kind) {
                *slot = media.clone();
            } else {
                game.media.push(media.clone());
            }
            return;
        }
    }

    fn begin_rename(&mut self, game: &Game) {
        self.overlay = Overlay::Rename(Rename {
            game_id: game.id.clone(),
            edit: LineEdit::selected(game.title.clone()),
            slot: RenameSlot::Title,
            error: None,
        });
    }

    fn begin_delete(&mut self, game: &Game) {
        self.overlay = Overlay::Delete(DeletePrompt {
            game_id: game.id.clone(),
            title: game.title.clone(),
            options: DeleteOptions::default(),
            slot: DeleteSlot::Cancel,
        });
    }

    /// GTK Scrape button and the `s` key. Same dialog as the menu's Scrape row.
    pub fn scrape_selected(&mut self) -> OverlayCommand {
        let Some(game) = self.selected_game().cloned() else {
            self.status = "Select a game to scrape.".into();
            return OverlayCommand::None;
        };
        self.begin_scrape(&game)
    }

    fn begin_scrape(&mut self, game: &Game) -> OverlayCommand {
        let query = scrape_query(game);
        let demo = self.library.kind == LibraryKind::Demo;
        self.overlay = Overlay::Scrape(ScrapePrompt {
            game_id: game.id.clone(),
            console_id: game.console.clone(),
            edit: LineEdit::selected(query.clone()),
            slot: ScrapeSlot::Query,
            cursor: 0,
            candidates: Vec::new(),
            message: if demo {
                "Demo library. Scrape needs a library on disk.".into()
            } else {
                "Searching…".into()
            },
            searching: !demo,
        });
        if demo {
            OverlayCommand::None
        } else {
            OverlayCommand::Search {
                query,
                console_id: game.console.clone(),
            }
        }
    }

    fn begin_listed_search(&mut self) -> OverlayCommand {
        let Overlay::Scrape(prompt) = &self.overlay else {
            return OverlayCommand::None;
        };
        let query = prompt.edit.text.trim().to_string();
        let console_id = prompt.console_id.clone();
        if self.library.kind == LibraryKind::Demo {
            if let Overlay::Scrape(prompt) = &mut self.overlay {
                prompt.searching = false;
                prompt.message = "Demo library. Scrape needs a library on disk.".into();
            }
            return OverlayCommand::None;
        }
        if query.is_empty() {
            if let Overlay::Scrape(prompt) = &mut self.overlay {
                prompt.searching = false;
                prompt.candidates.clear();
                prompt.message = "Enter a name to search.".into();
            }
            return OverlayCommand::None;
        }
        if let Overlay::Scrape(prompt) = &mut self.overlay {
            prompt.searching = true;
            prompt.candidates.clear();
            prompt.cursor = 0;
            prompt.message = "Searching…".into();
        }
        OverlayCommand::Search { query, console_id }
    }

    fn take_candidate(&mut self, scrape_busy: bool) -> OverlayCommand {
        if scrape_busy {
            self.status = "A scrape is already running.".into();
            return OverlayCommand::None;
        }
        let (game_id, candidate) = {
            let Overlay::Scrape(prompt) = &self.overlay else {
                return OverlayCommand::None;
            };
            let candidate = prompt.candidates.get(prompt.cursor).cloned();
            (prompt.game_id.clone(), candidate)
        };
        let Some(candidate) = candidate else {
            return OverlayCommand::None;
        };
        if self.game_by_id(&game_id).is_none() {
            self.overlay = Overlay::None;
            self.status = "Select a game.".into();
            return OverlayCommand::None;
        }
        if self.library.kind == LibraryKind::Demo {
            self.overlay = Overlay::None;
            self.status = "Demo library. Scrape needs a library on disk.".into();
            return OverlayCommand::None;
        }
        self.overlay = Overlay::None;
        OverlayCommand::Apply { game_id, candidate }
    }

    fn rewrite_title(&mut self, id: &str, title: &str) {
        for shelf in &mut self.library.shelves {
            if let Some(game) = shelf.games.iter_mut().find(|game| game.id == id) {
                game.title = title.to_string();
                shelf.stats = stats_of(&shelf.games);
                return;
            }
        }
    }

    fn detach_game(&mut self, id: &str) -> Option<Game> {
        let selected = self.selected_game().map(|game| game.id.clone());
        let mut removed = None;
        for shelf in &mut self.library.shelves {
            let Some(index) = shelf.games.iter().position(|game| game.id == id) else {
                continue;
            };
            let game = shelf.games.remove(index);
            shelf.stats = stats_of(&shelf.games);
            removed = Some(game);
            break;
        }
        let game = removed?;
        if selected.as_deref() == Some(game.id.as_str()) {
            self.game = None;
        }
        if self.visible_len() == 0 {
            self.pane = crate::browse::Pane::Sidebar;
        }
        Some(game)
    }
}

enum ConfirmStep {
    Closed,
    Menu(Option<GameAction>),
    Rename(RenameSlot, bool),
    Delete(DeleteSlot),
    Scrape(ScrapeSlot),
}

fn move_menu(menu: &mut Menu, dir: NavDir) {
    let last = GameAction::ALL.len().saturating_sub(1);
    match dir {
        NavDir::Up => menu.cursor = menu.cursor.saturating_sub(1),
        NavDir::Down => menu.cursor = (menu.cursor + 1).min(last),
        NavDir::Left | NavDir::Right => {}
    }
}

fn move_rename(rename: &mut Rename, dir: NavDir) {
    match (rename.slot, dir) {
        (RenameSlot::Title, NavDir::Left) => rename.edit.move_caret(-1),
        (RenameSlot::Title, NavDir::Right) => rename.edit.move_caret(1),
        (RenameSlot::Title, NavDir::Down) => rename.slot = RenameSlot::Cancel,
        (RenameSlot::Cancel, NavDir::Right) => rename.slot = RenameSlot::Save,
        (RenameSlot::Save, NavDir::Left) => rename.slot = RenameSlot::Cancel,
        (RenameSlot::Cancel | RenameSlot::Save, NavDir::Up) => rename.slot = RenameSlot::Title,
        _ => {}
    }
}

fn move_delete(prompt: &mut DeletePrompt, dir: NavDir) {
    prompt.slot = match (prompt.slot, dir) {
        (DeleteSlot::Rom, NavDir::Down) => DeleteSlot::Assets,
        (DeleteSlot::Assets, NavDir::Up) => DeleteSlot::Rom,
        (DeleteSlot::Assets, NavDir::Down) => DeleteSlot::Cancel,
        (DeleteSlot::Cancel, NavDir::Up) => DeleteSlot::Assets,
        (DeleteSlot::Cancel, NavDir::Down) => DeleteSlot::Confirm,
        (DeleteSlot::Cancel, NavDir::Right) => DeleteSlot::Confirm,
        (DeleteSlot::Confirm, NavDir::Up) => DeleteSlot::Cancel,
        (DeleteSlot::Confirm, NavDir::Left) => DeleteSlot::Cancel,
        _ => prompt.slot,
    };
}

fn move_scrape(prompt: &mut ScrapePrompt, dir: NavDir) {
    match (prompt.slot, dir) {
        (ScrapeSlot::Query, NavDir::Left) => prompt.edit.move_caret(-1),
        (ScrapeSlot::Query, NavDir::Right) if prompt.edit.at_end() => {
            prompt.slot = ScrapeSlot::Search;
        }
        (ScrapeSlot::Query, NavDir::Right) => prompt.edit.move_caret(1),
        (ScrapeSlot::Query, NavDir::Down) if !prompt.candidates.is_empty() => {
            prompt.slot = ScrapeSlot::Results;
            prompt.cursor = 0;
        }
        (ScrapeSlot::Search, NavDir::Left) => prompt.slot = ScrapeSlot::Query,
        (ScrapeSlot::Search, NavDir::Down) if !prompt.candidates.is_empty() => {
            prompt.slot = ScrapeSlot::Results;
            prompt.cursor = 0;
        }
        (ScrapeSlot::Results, NavDir::Up) if prompt.cursor == 0 => {
            prompt.slot = ScrapeSlot::Query;
        }
        (ScrapeSlot::Results, NavDir::Up) => prompt.cursor = prompt.cursor.saturating_sub(1),
        (ScrapeSlot::Results, NavDir::Down) => {
            if !prompt.candidates.is_empty() {
                prompt.cursor = (prompt.cursor + 1).min(prompt.candidates.len() - 1);
            }
        }
        _ => {}
    }
}

fn scrape_query(game: &Game) -> String {
    let title = game.title.trim();
    if title.is_empty() {
        crate::scanner::derive_title(&game.rom)
    } else {
        title.to_string()
    }
}

fn search_message(outcome: &NameSearch) -> String {
    if outcome.candidates.is_empty() {
        if outcome.errors.is_empty() {
            "No matches. Edit the name and search again.".into()
        } else {
            format!("No matches. {}", outcome.errors.join(" "))
        }
    } else if outcome.errors.is_empty() {
        String::new()
    } else {
        outcome.errors.join(" ")
    }
}

fn split_chars(text: &str, caret: usize) -> (String, String) {
    let mut count = 0;
    for (index, _) in text.char_indices() {
        if count == caret {
            return (text[..index].to_string(), text[index..].to_string());
        }
        count += 1;
    }
    (text.to_string(), String::new())
}

fn byte_index(text: &str, caret: usize) -> usize {
    text.char_indices()
        .nth(caret)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::{demo_library, Browse, Pane};
    use crate::types::{ScrapeProvider, Source};

    fn entered() -> Browse {
        let mut browse = Browse::new(demo_library("Demo library."));
        assert_eq!(browse.confirm(), Some(crate::browse::Confirm::Entered));
        browse
    }

    fn open_menu(browse: &mut Browse) {
        assert!(browse.open_game_menu());
        assert!(browse.menu_open());
        assert_eq!(browse.menu_cursor(), Some(0));
    }

    #[test]
    fn menu_needs_the_games_pane_and_does_not_wrap() {
        let mut browse = Browse::new(demo_library("Demo library."));
        assert!(!browse.open_game_menu());
        assert_eq!(browse.status, "Select a game.");
        assert!(!browse.overlay_open());

        browse.confirm();
        browse.back();
        assert_eq!(browse.pane, Pane::Sidebar);
        assert!(browse.selected_game().is_some());
        assert!(!browse.open_game_menu());

        browse.confirm();
        open_menu(&mut browse);
        browse.move_overlay(NavDir::Up);
        assert_eq!(browse.menu_cursor(), Some(0));
        browse.move_overlay(NavDir::Down);
        browse.move_overlay(NavDir::Down);
        browse.move_overlay(NavDir::Down);
        assert_eq!(browse.menu_cursor(), Some(2));
        assert!(!game_menu_key("f10", false, false));
        assert!(game_menu_key("f10", true, false));
        assert!(game_menu_key("menu", false, false));
        assert!(!game_menu_key("menu", false, true));
    }

    #[test]
    fn right_click_selects_then_opens_on_that_card() {
        let mut browse = Browse::new(demo_library("Demo library."));
        browse.select_game(2);
        open_menu(&mut browse);
        assert_eq!(browse.selected_game().unwrap().title, "Super Metroid");
    }

    #[test]
    fn delete_prompt_starts_on_cancel_and_matches_gtk_focus() {
        let mut browse = entered();
        open_menu(&mut browse);
        browse.move_overlay(NavDir::Down);
        browse.move_overlay(NavDir::Down);
        assert_eq!(browse.confirm_overlay(false), OverlayCommand::None);
        match &browse.overlay {
            Overlay::Delete(prompt) => {
                assert_eq!(prompt.title, "Super Mario World");
                assert_eq!(prompt.slot, DeleteSlot::Cancel);
                assert_eq!(prompt.options, DeleteOptions::default());
            }
            other => panic!("{other:?}"),
        }
        browse.move_overlay(NavDir::Up);
        match &browse.overlay {
            Overlay::Delete(prompt) => assert_eq!(prompt.slot, DeleteSlot::Assets),
            other => panic!("{other:?}"),
        }
        browse.move_overlay(NavDir::Up);
        assert_eq!(browse.confirm_overlay(false), OverlayCommand::None);
        match &browse.overlay {
            Overlay::Delete(prompt) => {
                assert_eq!(prompt.slot, DeleteSlot::Rom);
                assert!(prompt.options.rom_file);
                assert!(!prompt.options.scraped_assets);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rename_moves_like_the_gtk_dialog_and_rejects_a_blank_title() {
        let mut browse = entered();
        open_menu(&mut browse);
        browse.move_overlay(NavDir::Down);
        browse.confirm_overlay(false);
        match &browse.overlay {
            Overlay::Rename(rename) => {
                assert_eq!(rename.edit.text, "Super Mario World");
                assert!(rename.edit.replace);
                assert_eq!(rename.slot, RenameSlot::Title);
            }
            other => panic!("{other:?}"),
        }
        browse.move_overlay(NavDir::Down);
        match &browse.overlay {
            Overlay::Rename(rename) => assert_eq!(rename.slot, RenameSlot::Cancel),
            other => panic!("{other:?}"),
        }
        browse.move_overlay(NavDir::Right);
        match &browse.overlay {
            Overlay::Rename(rename) => assert_eq!(rename.slot, RenameSlot::Save),
            other => panic!("{other:?}"),
        }
        browse.move_overlay(NavDir::Up);
        browse.backspace();
        assert_eq!(browse.confirm_overlay(false), OverlayCommand::None);
        match &browse.overlay {
            Overlay::Rename(rename) => {
                assert_eq!(rename.error.as_deref(), Some("Enter a title."));
                assert!(rename.edit.text.trim().is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(browse.selected_game().unwrap().title, "Super Mario World");
    }

    #[test]
    fn demo_rename_and_delete_do_not_change_the_library() {
        let mut browse = entered();
        open_menu(&mut browse);
        browse.move_overlay(NavDir::Down);
        browse.confirm_overlay(false);
        browse.insert_text("Zelda");
        assert_eq!(browse.confirm_overlay(false), OverlayCommand::CommitRename);
        assert!(!browse.commit_rename(None));
        assert!(!browse.overlay_open());
        assert_eq!(browse.status, "Demo library. Rename is not saved.");
        assert_eq!(browse.selected_game().unwrap().title, "Super Mario World");

        open_menu(&mut browse);
        browse.move_overlay(NavDir::Down);
        browse.move_overlay(NavDir::Down);
        browse.confirm_overlay(false);
        browse.aim(Aim::Delete(DeleteSlot::Confirm));
        assert_eq!(browse.confirm_overlay(false), OverlayCommand::CommitDelete);
        assert!(browse.commit_delete(None).is_none());
        assert_eq!(browse.status, "Demo library. Delete is not saved.");
        assert_eq!(browse.visible_len(), 4);
    }

    #[test]
    fn disk_rename_persists_and_delete_drops_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let conn = database::open_db(&dir.path().join("library.db")).unwrap();
        let mut browse = entered();
        browse.library.kind = LibraryKind::Disk;
        browse.library.note.clear();
        for shelf in &browse.library.shelves {
            for game in &shelf.games {
                database::upsert_game(&conn, game).unwrap();
            }
        }
        let id = browse.selected_game().unwrap().id.clone();

        open_menu(&mut browse);
        browse.move_overlay(NavDir::Down);
        browse.confirm_overlay(false);
        browse.move_overlay(NavDir::Right);
        browse.insert_text("!");
        match &browse.overlay {
            Overlay::Rename(rename) => {
                assert!(rename.edit.text.ends_with('!'));
                assert!(rename.edit.text.starts_with("Super Mario World"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(browse.confirm_overlay(false), OverlayCommand::CommitRename);
        assert!(browse.commit_rename(Some(&conn)));
        assert_eq!(browse.selected_game().unwrap().title, "Super Mario World!");
        let stored = database::load_games(&conn, None).unwrap();
        let stored = stored.iter().find(|game| game.id == id).unwrap();
        assert_eq!(stored.title, "Super Mario World!");
        let custom: i32 = conn
            .query_row(
                "SELECT title_custom FROM games WHERE id = ?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(custom, 1);

        open_menu(&mut browse);
        browse.aim(Aim::Menu(2));
        browse.confirm_overlay(false);
        browse.aim(Aim::Delete(DeleteSlot::Confirm));
        assert_eq!(browse.confirm_overlay(false), OverlayCommand::CommitDelete);
        let removed = browse.commit_delete(Some(&conn)).unwrap();
        assert_eq!(removed.game.id, id);
        assert!(!removed.options.rom_file);
        assert!(!removed.options.scraped_assets);
        assert!(browse.game.is_none());
        assert!(browse.status.starts_with("Removed Super Mario World!"));
        assert!(browse.game_by_id(&id).is_none());
        let snes = "snes".to_string();
        let left = database::load_games(&conn, Some(&snes)).unwrap();
        assert!(left.iter().all(|game| game.id != id));
        assert_eq!(
            browse.shelf().unwrap().stats.total_games,
            browse.shelf().unwrap().games.len() as u32
        );
    }

    #[test]
    fn scrape_on_demo_does_not_search_and_a_disk_hit_can_be_applied() {
        let mut browse = entered();
        open_menu(&mut browse);
        assert_eq!(browse.confirm_overlay(false), OverlayCommand::None);
        match &browse.overlay {
            Overlay::Scrape(prompt) => {
                assert_eq!(
                    prompt.message,
                    "Demo library. Scrape needs a library on disk."
                );
                assert!(!prompt.searching);
            }
            other => panic!("{other:?}"),
        }
        browse.move_overlay(NavDir::Right);
        match &browse.overlay {
            Overlay::Scrape(prompt) => assert_eq!(prompt.slot, ScrapeSlot::Search),
            other => panic!("{other:?}"),
        }
        assert_eq!(browse.confirm_overlay(false), OverlayCommand::None);

        let mut browse = entered();
        browse.library.kind = LibraryKind::Disk;
        open_menu(&mut browse);
        assert_eq!(
            browse.confirm_overlay(false),
            OverlayCommand::Search {
                query: "Super Mario World".into(),
                console_id: "snes".into(),
            }
        );
        browse.finish_search(NameSearch {
            candidates: vec![ScrapeCandidate {
                provider: ScrapeProvider::ScreenScraper,
                remote_id: "9".into(),
                title: "Super Mario World".into(),
                system: "Super Nintendo".into(),
            }],
            errors: vec![],
        });
        browse.aim(Aim::ScrapeResult(0));
        assert_eq!(browse.confirm_overlay(true), OverlayCommand::None);
        assert_eq!(browse.status, "A scrape is already running.");
        assert!(matches!(browse.overlay, Overlay::Scrape(_)));

        let command = browse.confirm_overlay(false);
        let OverlayCommand::Apply { game_id, candidate } = command else {
            panic!("expected apply");
        };
        assert_eq!(game_id, browse.selected_game().unwrap().id);
        assert_eq!(candidate.remote_id, "9");
        assert_eq!(candidate.provider.source(), Source::ScreenScraper);
        assert!(!browse.overlay_open());
    }

    #[test]
    fn scrape_selected_opens_the_menu_dialog_and_keeps_the_game() {
        let mut browse = Browse::new(demo_library("Demo library."));
        assert_eq!(browse.scrape_selected(), OverlayCommand::None);
        assert_eq!(browse.status, "Select a game to scrape.");
        assert!(!browse.overlay_open());

        let mut browse = entered();
        browse.library.kind = LibraryKind::Disk;
        let id = browse.selected_game().unwrap().id.clone();
        assert_eq!(
            browse.scrape_selected(),
            OverlayCommand::Search {
                query: "Super Mario World".into(),
                console_id: "snes".into(),
            }
        );
        match &browse.overlay {
            Overlay::Scrape(prompt) => {
                assert_eq!(prompt.game_id, id);
                assert!(prompt.searching);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(browse.game, Some(0));

        browse.close_overlay();
        browse.back();
        assert_eq!(browse.pane, Pane::Sidebar);
        assert!(matches!(
            browse.scrape_selected(),
            OverlayCommand::Search { .. }
        ));
        assert_eq!(browse.game, Some(0));
    }

    #[test]
    fn saved_artwork_lands_on_the_game_without_moving_the_selection() {
        use crate::types::{Media, MediaKind};

        let mut browse = entered();
        let id = browse.selected_game().unwrap().id.clone();
        let index = browse.game;
        browse.remember_media(
            &id,
            &Media {
                kind: MediaKind::Screenshot,
                path: std::path::PathBuf::from("/tmp/retromarchy-shot.png"),
                source: Source::ScreenScraper,
            },
        );
        assert_eq!(browse.game, index);
        assert_eq!(browse.selected_game().unwrap().title, "Super Mario World");
        assert!(browse
            .selected_game()
            .unwrap()
            .media
            .iter()
            .any(|media| media.kind == MediaKind::Screenshot
                && media.source == Source::ScreenScraper));
    }
}
