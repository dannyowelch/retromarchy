//! GTK Scraper settings.
//!
//! The draft is [`ScraperConfig`]: `box_art`, `screenshot`, `providers` in
//! priority order (`id` + `enabled`), and `credentials` (`screenscraper_user`,
//! `screenscraper_password`, `thegamesdb_api_key`). Save replaces `[scraper]`
//! through [`config::save_scraper_settings`]. Esc drops the draft. The shell
//! paints passwords and API keys as bullets; the draft keeps the characters.

use crate::game_menu::LineEdit;
use crate::gamepad::NavDir;
use crate::types::{ScraperConfig, ScraperCredentials};

pub const TITLE: &str = "Scraper settings";
pub const HEADING: &str = "Artwork scraper";
pub const INTRO: &str =
    "Downloads box art and screenshots only. ROMs and BIOS are never downloaded.";
pub const PROVIDERS_HEADING: &str = "Providers, highest priority first";
pub const PROVIDERS_NOTE: &str =
    "Each missing artwork type tries this list in order and stops at the first hit.";
pub const CREDS_HEADING: &str = "Credentials";
pub const CREDS_NOTE: &str = "Stored in config.toml. ScreenScraper needs your free member username and password. The application Softname is built in. TheGamesDB needs an API key.";

/// Ctrl+G, including Ctrl+Shift+G. GTK checks the control mask and `g` / `G`.
pub fn scraper_key(key: &str, key_char: Option<&str>, control: bool) -> bool {
    if !control {
        return false;
    }
    key.eq_ignore_ascii_case("g") || key_char.is_some_and(|ch| ch.eq_ignore_ascii_case("g"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Enabled,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    BoxArt,
    Screenshot,
    Provider { index: usize, part: Part },
    User,
    Password,
    ApiKey,
    Save,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    None,
    Save,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtLine {
    pub slot: Slot,
    pub label: &'static str,
    pub on: bool,
    pub aimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLine {
    pub index: usize,
    pub label: &'static str,
    pub enabled: bool,
    pub check_aimed: bool,
    pub up_aimed: bool,
    pub down_aimed: bool,
    pub up_on: bool,
    pub down_on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldLine {
    pub slot: Slot,
    pub placeholder: &'static str,
    /// Username is the typed text. Password and API key are bullets of the same length.
    pub edit: LineEdit,
    pub aimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(&'static str),
    Note(&'static str),
    Art(ArtLine),
    Provider(ProviderLine),
    Field(FieldLine),
    Error(String),
    Save { aimed: bool },
}

struct Stop {
    slot: Slot,
    row: usize,
    col: usize,
}

pub struct ScraperSettings {
    box_art: bool,
    screenshot: bool,
    providers: Vec<crate::types::ProviderEntry>,
    user: LineEdit,
    password: LineEdit,
    api_key: LineEdit,
    focus: Slot,
    error: Option<String>,
}

impl ScraperSettings {
    pub fn open(config: ScraperConfig) -> Self {
        Self {
            box_art: config.box_art,
            screenshot: config.screenshot,
            providers: config.providers,
            user: LineEdit::plain(config.credentials.screenscraper_user),
            password: LineEdit::plain(config.credentials.screenscraper_password),
            api_key: LineEdit::plain(config.credentials.thegamesdb_api_key),
            focus: Slot::BoxArt,
            error: None,
        }
    }

    pub fn focus(&self) -> Slot {
        self.focus
    }

    pub fn draft(&self) -> ScraperConfig {
        ScraperConfig {
            box_art: self.box_art,
            screenshot: self.screenshot,
            providers: self.providers.clone(),
            credentials: ScraperCredentials {
                screenscraper_user: self.user.text.clone(),
                screenscraper_password: self.password.text.clone(),
                thegamesdb_api_key: self.api_key.text.clone(),
            },
        }
    }

    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
    }

    pub fn aim(&mut self, slot: Slot) -> bool {
        if self.stops().iter().any(|stop| stop.slot == slot) {
            self.focus = slot;
            true
        } else {
            false
        }
    }

    /// Mouse path. The checkbox reports the next state, so this stores it.
    pub fn set_checked(&mut self, slot: Slot, on: bool) {
        match slot {
            Slot::BoxArt => self.box_art = on,
            Slot::Screenshot => self.screenshot = on,
            Slot::Provider {
                index,
                part: Part::Enabled,
            } => {
                if let Some(entry) = self.providers.get_mut(index) {
                    entry.enabled = on;
                }
            }
            _ => {}
        }
    }

    /// Up and down change rows. Left and right move along a provider row, or the caret.
    pub fn move_dir(&mut self, dir: NavDir) {
        if matches!(self.focus, Slot::User | Slot::Password | Slot::ApiKey)
            && matches!(dir, NavDir::Left | NavDir::Right)
        {
            let delta = if dir == NavDir::Left { -1 } else { 1 };
            if let Some(edit) = self.edit_mut() {
                edit.move_caret(delta);
            }
            return;
        }
        let stops = self.stops();
        let Some(current) = stops.iter().find(|stop| stop.slot == self.focus) else {
            self.focus = Slot::BoxArt;
            return;
        };
        match dir {
            NavDir::Left | NavDir::Right => {
                let delta: isize = if dir == NavDir::Left { -1 } else { 1 };
                if let Some(next) = stops.iter().find(|stop| {
                    stop.row == current.row && stop.col as isize == current.col as isize + delta
                }) {
                    self.focus = next.slot;
                }
            }
            NavDir::Up | NavDir::Down => {
                let target = if dir == NavDir::Up {
                    stops
                        .iter()
                        .filter(|stop| stop.row < current.row)
                        .map(|stop| stop.row)
                        .max()
                } else {
                    stops
                        .iter()
                        .filter(|stop| stop.row > current.row)
                        .map(|stop| stop.row)
                        .min()
                };
                if let Some(row) = target {
                    if let Some(next) =
                        stops
                            .iter()
                            .filter(|stop| stop.row == row)
                            .min_by_key(|stop| {
                                (stop.col as isize - current.col as isize).unsigned_abs()
                            })
                    {
                        self.focus = next.slot;
                    }
                }
            }
        }
    }

    pub fn tab(&mut self, backward: bool) {
        let stops = self.stops();
        let len = stops.len();
        if len == 0 {
            return;
        }
        let pos = stops
            .iter()
            .position(|stop| stop.slot == self.focus)
            .unwrap_or(0);
        let next = if backward {
            (pos + len - 1) % len
        } else {
            (pos + 1) % len
        };
        self.focus = stops[next].slot;
    }

    pub fn accepts_text(&self) -> bool {
        matches!(self.focus, Slot::User | Slot::Password | Slot::ApiKey)
    }

    pub fn type_text(&mut self, text: &str) {
        if let Some(edit) = self.edit_mut() {
            edit.insert(text);
        }
    }

    pub fn backspace(&mut self) {
        if let Some(edit) = self.edit_mut() {
            edit.backspace();
        }
    }

    pub fn delete_forward(&mut self) {
        if let Some(edit) = self.edit_mut() {
            edit.delete_forward();
        }
    }

    pub fn confirm(&mut self) -> Command {
        match self.focus {
            Slot::BoxArt => {
                self.box_art = !self.box_art;
                Command::None
            }
            Slot::Screenshot => {
                self.screenshot = !self.screenshot;
                Command::None
            }
            Slot::Provider {
                index,
                part: Part::Enabled,
            } => {
                if let Some(entry) = self.providers.get_mut(index) {
                    entry.enabled = !entry.enabled;
                }
                Command::None
            }
            Slot::Provider {
                index,
                part: Part::Up,
            } => {
                self.shift_provider(index, -1);
                Command::None
            }
            Slot::Provider {
                index,
                part: Part::Down,
            } => {
                self.shift_provider(index, 1);
                Command::None
            }
            Slot::User | Slot::Password | Slot::ApiKey => Command::None,
            Slot::Save => Command::Save,
        }
    }

    pub fn blocks(&self) -> Vec<Block> {
        let mut blocks = vec![
            Block::Heading(HEADING),
            Block::Note(INTRO),
            Block::Art(self.art_line(Slot::BoxArt, "Box art", self.box_art)),
            Block::Art(self.art_line(Slot::Screenshot, "Screenshot", self.screenshot)),
            Block::Heading(PROVIDERS_HEADING),
            Block::Note(PROVIDERS_NOTE),
        ];
        for line in self.provider_lines() {
            blocks.push(Block::Provider(line));
        }
        blocks.push(Block::Heading(CREDS_HEADING));
        blocks.push(Block::Note(CREDS_NOTE));
        for line in self.field_lines() {
            blocks.push(Block::Field(line));
        }
        if let Some(error) = &self.error {
            blocks.push(Block::Error(error.clone()));
        }
        blocks.push(Block::Save {
            aimed: self.focus == Slot::Save,
        });
        blocks
    }

    pub fn scroll_index(&self) -> usize {
        self.blocks()
            .iter()
            .position(|block| block_focused(block, self.focus))
            .unwrap_or(0)
    }

    fn art_line(&self, slot: Slot, label: &'static str, on: bool) -> ArtLine {
        ArtLine {
            slot,
            label,
            on,
            aimed: self.focus == slot,
        }
    }

    fn provider_lines(&self) -> Vec<ProviderLine> {
        let len = self.providers.len();
        self.providers
            .iter()
            .enumerate()
            .map(|(index, entry)| ProviderLine {
                index,
                label: entry.id.label(),
                enabled: entry.enabled,
                check_aimed: self.focus
                    == Slot::Provider {
                        index,
                        part: Part::Enabled,
                    },
                up_aimed: self.focus
                    == Slot::Provider {
                        index,
                        part: Part::Up,
                    },
                down_aimed: self.focus
                    == Slot::Provider {
                        index,
                        part: Part::Down,
                    },
                up_on: index > 0,
                down_on: index + 1 < len,
            })
            .collect()
    }

    fn field_lines(&self) -> [FieldLine; 3] {
        [
            self.field_line(Slot::User, "ScreenScraper username", &self.user, false),
            self.field_line(
                Slot::Password,
                "ScreenScraper password",
                &self.password,
                true,
            ),
            self.field_line(Slot::ApiKey, "TheGamesDB API key", &self.api_key, true),
        ]
    }

    fn field_line(
        &self,
        slot: Slot,
        placeholder: &'static str,
        edit: &LineEdit,
        secret: bool,
    ) -> FieldLine {
        FieldLine {
            slot,
            placeholder,
            edit: shown(edit, secret),
            aimed: self.focus == slot,
        }
    }

    fn shift_provider(&mut self, index: usize, delta: isize) {
        let Some(next) = index.checked_add_signed(delta) else {
            return;
        };
        if next >= self.providers.len() {
            return;
        }
        self.providers.swap(index, next);
        let part = match self.focus {
            Slot::Provider { part, .. } => part,
            _ => Part::Enabled,
        };
        self.focus = Slot::Provider { index: next, part };
        self.settle();
    }

    fn settle(&mut self) {
        if self.stops().iter().any(|stop| stop.slot == self.focus) {
            return;
        }
        if let Slot::Provider { index, .. } = self.focus {
            let enabled = Slot::Provider {
                index,
                part: Part::Enabled,
            };
            if self.stops().iter().any(|stop| stop.slot == enabled) {
                self.focus = enabled;
                return;
            }
        }
        self.focus = Slot::Save;
    }

    fn stops(&self) -> Vec<Stop> {
        let mut stops = Vec::new();
        let mut row = 0usize;
        stops.push(Stop {
            slot: Slot::BoxArt,
            row,
            col: 0,
        });
        row += 1;
        stops.push(Stop {
            slot: Slot::Screenshot,
            row,
            col: 0,
        });
        row += 1;
        for index in 0..self.providers.len() {
            let mut col = 0usize;
            stops.push(Stop {
                slot: Slot::Provider {
                    index,
                    part: Part::Enabled,
                },
                row,
                col,
            });
            col += 1;
            if index > 0 {
                stops.push(Stop {
                    slot: Slot::Provider {
                        index,
                        part: Part::Up,
                    },
                    row,
                    col,
                });
                col += 1;
            }
            if index + 1 < self.providers.len() {
                stops.push(Stop {
                    slot: Slot::Provider {
                        index,
                        part: Part::Down,
                    },
                    row,
                    col,
                });
            }
            row += 1;
        }
        for slot in [Slot::User, Slot::Password, Slot::ApiKey, Slot::Save] {
            stops.push(Stop { slot, row, col: 0 });
            row += 1;
        }
        stops
    }

    fn edit_mut(&mut self) -> Option<&mut LineEdit> {
        match self.focus {
            Slot::User => Some(&mut self.user),
            Slot::Password => Some(&mut self.password),
            Slot::ApiKey => Some(&mut self.api_key),
            _ => None,
        }
    }
}

fn shown(edit: &LineEdit, secret: bool) -> LineEdit {
    LineEdit {
        text: if secret {
            mask(&edit.text)
        } else {
            edit.text.clone()
        },
        caret: edit.caret,
        replace: edit.replace,
    }
}

fn mask(text: &str) -> String {
    "•".repeat(text.chars().count())
}

fn block_focused(block: &Block, focus: Slot) -> bool {
    match block {
        Block::Art(line) => line.slot == focus,
        Block::Provider(line) => matches!(
            focus,
            Slot::Provider { index, .. } if index == line.index
        ),
        Block::Field(line) => line.slot == focus,
        Block::Save { .. } => focus == Slot::Save,
        Block::Heading(_) | Block::Note(_) | Block::Error(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::types::{GridArt, MediaToggles, ProviderEntry, ScrapeProvider};

    fn field(dialog: &ScraperSettings, slot: Slot) -> FieldLine {
        dialog
            .blocks()
            .into_iter()
            .find_map(|block| match block {
                Block::Field(line) if line.slot == slot => Some(line),
                _ => None,
            })
            .expect("field")
    }

    fn same(left: &str, right: &str) -> bool {
        left == right
    }

    #[test]
    fn ctrl_g_matches_the_gtk_chord() {
        assert!(scraper_key("g", None, true));
        assert!(scraper_key("G", None, true));
        assert!(scraper_key("g", Some("G"), true));
        assert!(!scraper_key("g", None, false));
        assert!(!scraper_key("s", Some("s"), true));
    }

    #[test]
    fn opens_on_box_art_with_the_gtk_rows() {
        let mut dialog = ScraperSettings::open(ScraperConfig::default());
        assert_eq!(dialog.focus(), Slot::BoxArt);
        let blocks = dialog.blocks();
        assert_eq!(
            blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Heading(text) => Some(*text),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![HEADING, PROVIDERS_HEADING, CREDS_HEADING]
        );
        assert!(blocks.iter().any(|block| matches!(
            block,
            Block::Note(INTRO) | Block::Note(PROVIDERS_NOTE) | Block::Note(CREDS_NOTE)
        )));
        match &blocks[2] {
            Block::Art(line) => {
                assert_eq!(line.label, "Box art");
                assert!(line.on && line.aimed);
            }
            other => panic!("expected box art, got {other:?}"),
        }
        match &blocks[3] {
            Block::Art(line) => {
                assert_eq!(line.label, "Screenshot");
                assert!(line.on && !line.aimed);
            }
            other => panic!("expected screenshot, got {other:?}"),
        }
        let providers: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::Provider(line) => Some(line),
                _ => None,
            })
            .collect();
        assert_eq!(providers[0].label, "ScreenScraper");
        assert!(providers[0].enabled && !providers[0].up_on && providers[0].down_on);
        assert_eq!(providers[1].label, "TheGamesDB");
        assert!(providers[1].enabled && providers[1].up_on && !providers[1].down_on);
        assert_eq!(
            field(&dialog, Slot::User).placeholder,
            "ScreenScraper username"
        );
        assert_eq!(
            field(&dialog, Slot::Password).placeholder,
            "ScreenScraper password"
        );
        assert_eq!(
            field(&dialog, Slot::ApiKey).placeholder,
            "TheGamesDB API key"
        );
        assert_eq!(dialog.scroll_index(), 2);
        assert!(!dialog.aim(Slot::Provider {
            index: 0,
            part: Part::Up,
        }));
        assert_eq!(dialog.focus(), Slot::BoxArt);
    }

    #[test]
    fn arrows_tab_and_enter_follow_the_rows() {
        let mut dialog = ScraperSettings::open(ScraperConfig::default());
        dialog.move_dir(NavDir::Up);
        assert_eq!(dialog.focus(), Slot::BoxArt);
        dialog.move_dir(NavDir::Down);
        dialog.move_dir(NavDir::Down);
        assert_eq!(
            dialog.focus(),
            Slot::Provider {
                index: 0,
                part: Part::Enabled,
            }
        );
        dialog.move_dir(NavDir::Right);
        assert_eq!(
            dialog.focus(),
            Slot::Provider {
                index: 0,
                part: Part::Down,
            }
        );
        dialog.move_dir(NavDir::Down);
        assert_eq!(
            dialog.focus(),
            Slot::Provider {
                index: 1,
                part: Part::Up,
            }
        );
        dialog.confirm();
        let labels: Vec<_> = dialog
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::Provider(line) => Some(line.label),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["TheGamesDB", "ScreenScraper"]);
        assert_eq!(
            dialog.focus(),
            Slot::Provider {
                index: 0,
                part: Part::Enabled,
            }
        );

        dialog.aim(Slot::Provider {
            index: 0,
            part: Part::Down,
        });
        dialog.confirm();
        let labels: Vec<_> = dialog
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::Provider(line) => Some(line.label),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["ScreenScraper", "TheGamesDB"]);
        assert_eq!(
            dialog.focus(),
            Slot::Provider {
                index: 1,
                part: Part::Enabled,
            }
        );

        dialog.aim(Slot::Save);
        dialog.tab(false);
        assert_eq!(dialog.focus(), Slot::BoxArt);
        dialog.tab(true);
        assert_eq!(dialog.focus(), Slot::Save);
        assert_eq!(dialog.confirm(), Command::Save);
    }

    #[test]
    fn checks_and_secrets_stay_in_the_draft_until_save() {
        let mut dialog = ScraperSettings::open(ScraperConfig::default());
        dialog.confirm();
        assert!(!dialog.draft().box_art);
        dialog.aim(Slot::Screenshot);
        dialog.set_checked(Slot::Screenshot, false);
        assert!(!dialog.draft().screenshot);
        dialog.aim(Slot::Provider {
            index: 1,
            part: Part::Enabled,
        });
        dialog.confirm();
        assert!(!dialog.draft().providers[1].enabled);

        dialog.aim(Slot::User);
        dialog.type_text("testuser");
        dialog.move_dir(NavDir::Left);
        dialog.type_text("-");
        assert_eq!(field(&dialog, Slot::User).edit.text, "testuse-r");
        dialog.aim(Slot::Password);
        dialog.type_text("abcd");
        assert_eq!(field(&dialog, Slot::Password).edit.text, "••••");
        assert_eq!(field(&dialog, Slot::Password).edit.caret, 4);
        dialog.move_dir(NavDir::Left);
        dialog.backspace();
        assert_eq!(field(&dialog, Slot::Password).edit.text, "•••");
        dialog.aim(Slot::ApiKey);
        dialog.type_text("éé");
        assert_eq!(field(&dialog, Slot::ApiKey).edit.text, "••");
        dialog.move_dir(NavDir::Left);
        dialog.delete_forward();
        assert_eq!(field(&dialog, Slot::ApiKey).edit.text, "•");

        dialog.aim(Slot::BoxArt);
        dialog.type_text("nope");
        let draft = dialog.draft();
        assert!(same(&draft.credentials.screenscraper_user, "testuse-r"));
        assert_eq!(draft.credentials.screenscraper_password.chars().count(), 3);
        assert!(same(&draft.credentials.screenscraper_password, "abd"));
        assert_eq!(draft.credentials.thegamesdb_api_key.chars().count(), 1);
        assert_eq!(dialog.confirm(), Command::None);
        assert!(dialog.draft().box_art);
    }

    #[test]
    fn save_writes_scraper_and_leaves_the_rest_of_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let _env = config::XdgEnv::sandbox(dir.path());
        let mut config = config::Config::default();
        config.theme = "launchbox".into();
        config.cover_width = 220.0;
        config.input.initial_delay_ms = 500;
        config.consoles.push(crate::types::Console {
            id: "snes".into(),
            name: "Super Nintendo".into(),
            rom_dirs: vec![],
            extensions: vec!["sfc".into()],
            profile: Some("snes9x".into()),
            grid_art: GridArt::BoxArt,
            media: MediaToggles::default(),
        });
        config::save_config(&config).unwrap();

        let mut dialog = ScraperSettings::open(config::load_config().unwrap().scraper);
        dialog.aim(Slot::Screenshot);
        dialog.confirm();
        dialog.aim(Slot::Provider {
            index: 0,
            part: Part::Down,
        });
        dialog.confirm();
        dialog.aim(Slot::Provider {
            index: 0,
            part: Part::Enabled,
        });
        dialog.confirm();
        dialog.aim(Slot::User);
        dialog.type_text("testuser");
        dialog.aim(Slot::Password);
        dialog.type_text("fakepass");
        dialog.aim(Slot::ApiKey);
        dialog.type_text("tgdb-test");
        assert_eq!(dialog.confirm(), Command::None);
        dialog.aim(Slot::Save);
        assert_eq!(dialog.confirm(), Command::Save);

        config::save_scraper_settings(dialog.draft()).unwrap();
        let loaded = config::load_config().unwrap();
        assert_eq!(loaded.theme, "launchbox");
        assert_eq!(loaded.cover_width, 220.0);
        assert_eq!(loaded.input.initial_delay_ms, 500);
        assert_eq!(loaded.consoles[0].name, "Super Nintendo");
        assert_eq!(loaded.consoles[0].profile.as_deref(), Some("snes9x"));
        assert!(loaded.scraper.box_art);
        assert!(!loaded.scraper.screenshot);
        assert_eq!(
            loaded.scraper.providers,
            vec![
                ProviderEntry {
                    id: ScrapeProvider::TheGamesDb,
                    enabled: false,
                },
                ProviderEntry {
                    id: ScrapeProvider::ScreenScraper,
                    enabled: true,
                },
            ]
        );
        assert!(same(
            &loaded.scraper.credentials.screenscraper_user,
            "testuser"
        ));
        assert!(same(
            &loaded.scraper.credentials.screenscraper_password,
            "fakepass"
        ));
        assert!(same(
            &loaded.scraper.credentials.thegamesdb_api_key,
            "tgdb-test"
        ));
        let text = std::fs::read_to_string(config::config_path().unwrap()).unwrap();
        assert!(text.contains("[scraper]"));
        assert!(text.contains("testuser"));
        assert!(!text.contains("screenscraper_dev_"));
    }
}
