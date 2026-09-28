//! Options → Input. Four millisecond spins, saved to `[input]` as they change.
//!
//! The window title is Options. The group is Input. Each spin steps by 10 and
//! stops at 60_000. Starting pause and the transition may be 0. The repeat
//! intervals may not. [`InputSettings::sanitize`] still pulls a fast repeat
//! down when it would outrun the slow one, and the row shows that result.

use crate::config::InputSettings;
use crate::gamepad::NavDir;

pub const STEP_MS: u32 = 10;
/// Spin upper bound, and the cap in [`InputSettings::sanitize`].
pub const MAX_MS: u32 = 60_000;

pub const SECTION: &str = "Input";
pub const INTRO: &str = "Arrow keys, d-pad, and left stick. Saved to config.toml as you edit. Confirm, Back, and Favorite do not repeat.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Starting,
    Slow,
    Fast,
    Ramp,
    Close,
}

const ORDER: [Slot; 5] = [
    Slot::Starting,
    Slot::Slow,
    Slot::Fast,
    Slot::Ramp,
    Slot::Close,
];

struct Field {
    slot: Slot,
    title: &'static str,
    subtitle: &'static str,
    min: u32,
}

const FIELDS: [Field; 4] = [
    Field {
        slot: Slot::Starting,
        title: "Starting pause",
        subtitle: "Milliseconds before the first repeat",
        min: 0,
    },
    Field {
        slot: Slot::Slow,
        title: "Slow repeat",
        subtitle: "Milliseconds between steps at first",
        min: 1,
    },
    Field {
        slot: Slot::Fast,
        title: "Fast repeat",
        subtitle: "Milliseconds between steps after the transition",
        min: 1,
    },
    Field {
        slot: Slot::Ramp,
        title: "Slow-to-fast transition",
        subtitle: "Milliseconds to ease from the slow repeat to the fast one",
        min: 0,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub slot: Slot,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub value: u32,
    pub aimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputOptions {
    input: InputSettings,
    focus: Slot,
}

impl InputOptions {
    pub fn open(input: InputSettings) -> Self {
        Self {
            input: input.sanitized(),
            focus: Slot::Starting,
        }
    }

    pub fn input(&self) -> InputSettings {
        self.input
    }

    pub fn focus(&self) -> Slot {
        self.focus
    }

    pub fn close_aimed(&self) -> bool {
        self.focus == Slot::Close
    }

    pub fn rows(&self) -> [Row; 4] {
        FIELDS.map(|field| Row {
            slot: field.slot,
            title: field.title,
            subtitle: field.subtitle,
            value: value_of(self.input, field.slot),
            aimed: self.focus == field.slot,
        })
    }

    pub fn aim(&mut self, slot: Slot) {
        self.focus = slot;
    }

    /// Up and down move. Left and right step the aimed spin by [`STEP_MS`].
    /// Returns whether `[input]` changed.
    pub fn move_dir(&mut self, dir: NavDir) -> bool {
        match dir {
            NavDir::Up => {
                self.shift(-1, false);
                false
            }
            NavDir::Down => {
                self.shift(1, false);
                false
            }
            NavDir::Left => self.step_focused(-1),
            NavDir::Right => self.step_focused(1),
        }
    }

    pub fn tab(&mut self, backward: bool) {
        self.shift(if backward { -1 } else { 1 }, true);
    }

    /// Aim `slot` and step it. A mouse click on − or + uses this.
    pub fn step(&mut self, slot: Slot, steps: i32) -> bool {
        self.focus = slot;
        self.step_focused(steps)
    }

    fn shift(&mut self, delta: isize, wrap: bool) {
        let pos = ORDER
            .iter()
            .position(|slot| *slot == self.focus)
            .unwrap_or(0);
        let len = ORDER.len() as isize;
        let next = pos as isize + delta;
        let next = if wrap {
            next.rem_euclid(len)
        } else if (0..len).contains(&next) {
            next
        } else {
            return;
        };
        self.focus = ORDER[next as usize];
    }

    fn step_focused(&mut self, steps: i32) -> bool {
        let Some(field) = FIELDS.iter().find(|field| field.slot == self.focus) else {
            return false;
        };
        if steps == 0 {
            return false;
        }
        let before = self.input;
        let next = step_ms(value_of(self.input, field.slot), field.min, steps);
        set_value(&mut self.input, field.slot, next);
        self.input.sanitize();
        self.input != before
    }
}

fn value_of(input: InputSettings, slot: Slot) -> u32 {
    match slot {
        Slot::Starting => input.initial_delay_ms,
        Slot::Slow => input.slow_interval_ms,
        Slot::Fast => input.fast_interval_ms,
        Slot::Ramp => input.ramp_ms,
        Slot::Close => 0,
    }
}

fn set_value(input: &mut InputSettings, slot: Slot, value: u32) {
    match slot {
        Slot::Starting => input.initial_delay_ms = value,
        Slot::Slow => input.slow_interval_ms = value,
        Slot::Fast => input.fast_interval_ms = value,
        Slot::Ramp => input.ramp_ms = value,
        Slot::Close => {}
    }
}

pub fn options_key(key: &str, key_char: Option<&str>, control: bool) -> bool {
    if !control {
        return false;
    }
    key.eq_ignore_ascii_case("o") || key_char.is_some_and(|ch| ch.eq_ignore_ascii_case("o"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Input,
    Theme,
    Scraper,
    Grid,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Theme => "Theme",
            Self::Scraper => "Scraper",
            Self::Grid => "Grid",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::Input => "Hold repeat",
            Self::Theme => "Omarchy or LaunchBox",
            Self::Scraper => "Artwork and accounts",
            Self::Grid => "Cover width and order",
        }
    }
}

const SECTIONS: [Section; 4] = [
    Section::Input,
    Section::Theme,
    Section::Scraper,
    Section::Grid,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GridField {
    Cover,
    Sort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    None,
    Close,
    Input,
    Theme,
    Cover,
    Sort,
    SaveScraper,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionRow {
    pub section: Section,
    pub title: &'static str,
    pub detail: &'static str,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    split: crate::split::Split,
    input: InputOptions,
    theme: String,
    scraper: crate::scraper_settings::ScraperSettings,
    cover_width: f32,
    system_sort: crate::config::SystemSort,
    grid_field: GridField,
}

impl Options {
    pub fn open(config: &crate::config::Config, section: Section) -> Self {
        let index = SECTIONS
            .iter()
            .position(|item| *item == section)
            .unwrap_or(0);
        Self {
            split: crate::split::Split::at(index),
            input: InputOptions::open(config.input),
            theme: config.theme.clone(),
            scraper: crate::scraper_settings::ScraperSettings::open(config.scraper.clone()),
            cover_width: crate::config::clamp_cover_width(config.cover_width),
            system_sort: config.system_sort,
            grid_field: GridField::Cover,
        }
    }

    pub fn section(&self) -> Section {
        SECTIONS[self.split.index.min(SECTIONS.len() - 1)]
    }

    pub fn list_focused(&self) -> bool {
        self.split.side == crate::split::Side::List
    }

    pub fn selected_index(&self) -> usize {
        self.split.index
    }

    pub fn sections(&self) -> [SectionRow; 4] {
        SECTIONS.map(|section| SectionRow {
            section,
            title: section.label(),
            detail: section.detail(),
            selected: section == self.section(),
        })
    }

    pub fn input_rows(&self) -> [Row; 4] {
        let mut rows = self.input.rows();
        if !self.panel_live(Section::Input) {
            for row in &mut rows {
                row.aimed = false;
            }
        }
        rows
    }

    pub fn input(&self) -> InputSettings {
        self.input.input()
    }

    pub fn input_close_aimed(&self) -> bool {
        self.panel_live(Section::Input) && self.input.close_aimed()
    }

    pub fn theme(&self) -> &str {
        &self.theme
    }

    pub fn theme_launchbox(&self) -> bool {
        crate::appearance::is_launchbox(&self.theme)
    }

    pub fn theme_aimed(&self) -> bool {
        self.panel_live(Section::Theme)
    }

    pub fn cover_width(&self) -> f32 {
        self.cover_width
    }

    pub fn system_sort(&self) -> crate::config::SystemSort {
        self.system_sort
    }

    pub fn cover_aimed(&self) -> bool {
        self.panel_live(Section::Grid) && self.grid_field == GridField::Cover
    }

    pub fn sort_aimed(&self) -> bool {
        self.panel_live(Section::Grid) && self.grid_field == GridField::Sort
    }

    pub fn scraper(&self) -> &crate::scraper_settings::ScraperSettings {
        &self.scraper
    }

    pub fn scraper_mut(&mut self) -> &mut crate::scraper_settings::ScraperSettings {
        &mut self.scraper
    }

    pub fn select(&mut self, section: Section) {
        if let Some(index) = SECTIONS.iter().position(|item| *item == section) {
            self.split.select(index);
        }
    }

    pub fn focus_panel(&mut self) {
        self.split.enter();
    }

    pub fn choose_theme(&mut self, launchbox: bool) -> bool {
        self.split.enter();
        let next = if launchbox {
            crate::appearance::LAUNCHBOX
        } else {
            crate::appearance::SYSTEM
        };
        if self.theme == next {
            return false;
        }
        self.theme = next.to_string();
        true
    }

    pub fn adjust_cover(&mut self, steps: i32) -> bool {
        self.split.enter();
        self.grid_field = GridField::Cover;
        let next = crate::config::step_cover_width(self.cover_width, steps);
        if (next - self.cover_width).abs() < 0.5 {
            return false;
        }
        self.cover_width = next;
        true
    }

    pub fn set_system_sort(&mut self, sort: crate::config::SystemSort) {
        self.split.enter();
        self.grid_field = GridField::Sort;
        self.system_sort = sort;
    }

    pub fn aim_input(&mut self, slot: Slot) {
        self.split.enter();
        self.input.aim(slot);
    }

    pub fn step_input(&mut self, slot: Slot, steps: i32) -> bool {
        self.split.enter();
        self.input.step(slot, steps)
    }

    pub fn move_dir(&mut self, dir: NavDir) -> Command {
        if self.split.side == crate::split::Side::List {
            match dir {
                NavDir::Up => self.split.move_list(-1, SECTIONS.len()),
                NavDir::Down => self.split.move_list(1, SECTIONS.len()),
                NavDir::Right => self.enter_panel(),
                NavDir::Left => {}
            }
            return Command::None;
        }
        match self.section() {
            Section::Input => self.move_input(dir),
            Section::Theme => self.move_theme(dir),
            Section::Scraper => self.move_scraper(dir),
            Section::Grid => self.move_grid(dir),
        }
    }

    pub fn tab(&mut self, backward: bool) {
        if self.split.side == crate::split::Side::List {
            if !backward {
                self.enter_panel();
            }
            return;
        }
        if backward {
            self.split.leave();
            return;
        }
        match self.section() {
            Section::Input => self.input.tab(false),
            Section::Scraper => self.scraper.tab(false),
            Section::Grid => {
                self.grid_field = match self.grid_field {
                    GridField::Cover => GridField::Sort,
                    GridField::Sort => GridField::Cover,
                };
            }
            Section::Theme => {}
        }
    }

    pub fn confirm(&mut self) -> Command {
        if self.split.side == crate::split::Side::List {
            self.enter_panel();
            return Command::None;
        }
        match self.section() {
            Section::Input => {
                if self.input.close_aimed() {
                    Command::Close
                } else {
                    Command::None
                }
            }
            Section::Theme => {
                self.toggle_theme();
                Command::Theme
            }
            Section::Scraper => match self.scraper.confirm() {
                crate::scraper_settings::Command::Save => Command::SaveScraper,
                crate::scraper_settings::Command::None => Command::None,
            },
            Section::Grid => match self.grid_field {
                GridField::Cover => Command::None,
                GridField::Sort => {
                    self.system_sort = self.system_sort.next();
                    Command::Sort
                }
            },
        }
    }

    pub fn accepts_text(&self) -> bool {
        self.panel_live(Section::Scraper) && self.scraper.accepts_text()
    }

    pub fn type_text(&mut self, text: &str) {
        if self.panel_live(Section::Scraper) {
            self.scraper.type_text(text);
        }
    }

    pub fn backspace(&mut self) {
        if self.panel_live(Section::Scraper) {
            self.scraper.backspace();
        }
    }

    pub fn delete_forward(&mut self) {
        if self.panel_live(Section::Scraper) {
            self.scraper.delete_forward();
        }
    }

    fn panel_live(&self, section: Section) -> bool {
        self.split.side == crate::split::Side::Panel && self.section() == section
    }

    fn enter_panel(&mut self) {
        self.split.enter();
        match self.section() {
            Section::Input => self.input.aim(Slot::Starting),
            Section::Scraper => {
                self.scraper.aim(crate::scraper_settings::Slot::BoxArt);
            }
            Section::Grid => self.grid_field = GridField::Cover,
            Section::Theme => {}
        }
    }

    fn move_input(&mut self, dir: NavDir) -> Command {
        if dir == NavDir::Left && self.input.close_aimed() {
            self.split.leave();
            return Command::None;
        }
        if self.input.move_dir(dir) {
            Command::Input
        } else {
            Command::None
        }
    }

    fn move_theme(&mut self, dir: NavDir) -> Command {
        match dir {
            NavDir::Left | NavDir::Right => {
                self.toggle_theme();
                Command::Theme
            }
            NavDir::Up | NavDir::Down => Command::None,
        }
    }

    fn toggle_theme(&mut self) {
        self.theme = crate::appearance::next_theme(&self.theme).to_string();
    }

    fn move_scraper(&mut self, dir: NavDir) -> Command {
        if dir == NavDir::Left && self.scraper.at_text_start() {
            self.split.leave();
            return Command::None;
        }
        let before = self.scraper.focus();
        self.scraper.move_dir(dir);
        if dir == NavDir::Left && self.scraper.focus() == before && !self.scraper.accepts_text() {
            self.split.leave();
        }
        Command::None
    }

    fn move_grid(&mut self, dir: NavDir) -> Command {
        match dir {
            NavDir::Up | NavDir::Down => {
                self.grid_field = match self.grid_field {
                    GridField::Cover => GridField::Sort,
                    GridField::Sort => GridField::Cover,
                };
                Command::None
            }
            NavDir::Left | NavDir::Right => match self.grid_field {
                GridField::Cover => {
                    let steps = if dir == NavDir::Left { -1 } else { 1 };
                    let next = crate::config::step_cover_width(self.cover_width, steps);
                    if (next - self.cover_width).abs() < 0.5 {
                        return Command::None;
                    }
                    self.cover_width = next;
                    Command::Cover
                }
                GridField::Sort => {
                    self.system_sort = self.system_sort.next();
                    Command::Sort
                }
            },
        }
    }
}

fn step_ms(value: u32, min: u32, steps: i32) -> u32 {
    let magnitude = steps.unsigned_abs().saturating_mul(STEP_MS);
    let stepped = if steps < 0 {
        value.saturating_sub(magnitude)
    } else {
        value.saturating_add(magnitude)
    };
    stepped.clamp(min, MAX_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_on_the_input_defaults() {
        let dialog = InputOptions::open(InputSettings::default());
        assert_eq!(dialog.focus(), Slot::Starting);
        let rows = dialog.rows();
        assert_eq!(rows[0].title, "Starting pause");
        assert_eq!(rows[0].value, 400);
        assert!(rows[0].aimed);
        assert_eq!(rows[1].title, "Slow repeat");
        assert_eq!(rows[1].value, 180);
        assert_eq!(rows[2].title, "Fast repeat");
        assert_eq!(rows[2].value, 50);
        assert_eq!(rows[3].title, "Slow-to-fast transition");
        assert_eq!(rows[3].value, 2000);
        assert!(rows.iter().all(|row| row.subtitle.contains("Milliseconds")));
    }

    #[test]
    fn steps_stay_inside_the_spin_range() {
        let mut dialog = InputOptions::open(InputSettings::default());
        assert!(dialog.move_dir(NavDir::Left));
        assert_eq!(dialog.input().initial_delay_ms, 390);
        dialog.aim(Slot::Starting);
        set_value(&mut dialog.input, Slot::Starting, 0);
        assert!(!dialog.move_dir(NavDir::Left));
        assert_eq!(dialog.input().initial_delay_ms, 0);
        dialog.aim(Slot::Starting);
        set_value(&mut dialog.input, Slot::Starting, MAX_MS);
        assert!(!dialog.move_dir(NavDir::Right));
        assert_eq!(dialog.input().initial_delay_ms, MAX_MS);

        dialog.aim(Slot::Slow);
        set_value(&mut dialog.input, Slot::Slow, 1);
        set_value(&mut dialog.input, Slot::Fast, 1);
        assert!(!dialog.step(Slot::Slow, -1));
        assert_eq!(dialog.input().slow_interval_ms, 1);
    }

    #[test]
    fn fast_repeat_cannot_outrun_slow_repeat() {
        let mut dialog = InputOptions::open(InputSettings {
            initial_delay_ms: 400,
            slow_interval_ms: 50,
            fast_interval_ms: 50,
            ramp_ms: 2000,
        });
        dialog.aim(Slot::Fast);
        assert!(!dialog.move_dir(NavDir::Right));
        assert_eq!(dialog.input().fast_interval_ms, 50);

        dialog.aim(Slot::Slow);
        assert!(dialog.move_dir(NavDir::Left));
        assert_eq!(dialog.input().slow_interval_ms, 40);
        assert_eq!(dialog.input().fast_interval_ms, 40);
    }

    #[test]
    fn arrows_move_and_tab_wraps() {
        let mut dialog = InputOptions::open(InputSettings::default());
        assert!(!dialog.move_dir(NavDir::Up));
        assert_eq!(dialog.focus(), Slot::Starting);
        dialog.move_dir(NavDir::Down);
        dialog.move_dir(NavDir::Down);
        assert_eq!(dialog.focus(), Slot::Fast);
        dialog.tab(false);
        dialog.tab(false);
        assert_eq!(dialog.focus(), Slot::Close);
        assert!(dialog.close_aimed());
        dialog.tab(false);
        assert_eq!(dialog.focus(), Slot::Starting);
        dialog.tab(true);
        assert_eq!(dialog.focus(), Slot::Close);
        assert!(!dialog.move_dir(NavDir::Left));
        assert_eq!(dialog.focus(), Slot::Close);
    }

    #[test]
    fn options_sections_move_on_the_left_and_edit_on_the_right() {
        let mut options = Options::open(&crate::config::Config::default(), Section::Input);
        assert!(options.list_focused());
        assert_eq!(options.section(), Section::Input);
        assert_eq!(options.move_dir(NavDir::Down), Command::None);
        assert_eq!(options.section(), Section::Theme);
        options.move_dir(NavDir::Right);
        assert!(!options.list_focused());
        assert_eq!(options.move_dir(NavDir::Left), Command::Theme);
        assert!(options.theme_launchbox());
        options.tab(true);
        assert!(options.list_focused());
        options.move_dir(NavDir::Down);
        options.move_dir(NavDir::Down);
        assert_eq!(options.section(), Section::Grid);
        options.move_dir(NavDir::Right);
        assert_eq!(options.move_dir(NavDir::Left), Command::Cover);
        assert_eq!(
            options.cover_width(),
            crate::config::COVER_WIDTH_DEFAULT - 10.0
        );
        assert!(options_key("o", None, true));
        assert!(!options_key("o", None, false));
    }
}
