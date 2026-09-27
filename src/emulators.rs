//! Manage Emulators. The draft is the same data GTK writes: `profiles`, and
//! each console's `profile`. RetroArch profiles are a core path (optional
//! `config` stays unset). Standalone profiles are one command string with
//! `{rom}`. The shell paints this and calls [`apply_assignments`] on save.

use crate::config::Config;
use crate::cores::{CoreProfile, DiscoveredCore};
use crate::game_menu::LineEdit;
use crate::gamepad::NavDir;
use crate::types::EmulatorProfile;
use std::path::PathBuf;

/// Ctrl+E and Ctrl+M, including the shifted keysyms. GTK checks the control
/// mask and `e` / `E` / `m` / `M`.
pub fn emulator_key(key: &str, key_char: Option<&str>, control: bool) -> bool {
    if !control {
        return false;
    }
    let hit = |name: &str| {
        key.eq_ignore_ascii_case(name) || key_char.is_some_and(|ch| ch.eq_ignore_ascii_case(name))
    };
    hit("e") || hit("m")
}

/// GTK shows discovered cores only when `retroarch` is a file on `PATH`.
pub fn retroarch_on_path() -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join("retroarch").is_file()))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Standalone,
    RetroArch,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standalone => "Standalone",
            Self::RetroArch => "RetroArch",
        }
    }

    fn cycle(self, delta: isize) -> Self {
        let kinds = [Self::Standalone, Self::RetroArch];
        let pos = kinds.iter().position(|kind| *kind == self).unwrap_or(0);
        kinds[wrap(pos, kinds.len(), delta)]
    }
}

/// One console row in the dialog. `profile` is that console's default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleSlot {
    pub id: String,
    pub name: String,
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Add,
    Assign(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Core { index: usize, action: usize },
    Delete(usize),
    Kind,
    Id,
    Detail,
    Browse,
    Add,
    Console(usize),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stop {
    slot: Slot,
    row: usize,
    col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmulatorCommand {
    None,
    Close,
    Browse,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreButton {
    pub label: String,
    pub enabled: bool,
    pub aimed: bool,
    pub slot: Option<Slot>,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLine {
    pub index: usize,
    pub name: String,
    pub path: String,
    pub buttons: Vec<CoreButton>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileLine {
    pub index: usize,
    pub label: String,
    pub aimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleLine {
    pub index: usize,
    pub name: String,
    pub value: String,
    pub aimed: bool,
}

/// Painted above the scroller. The dialog title sits on top of a scrollport,
/// and this heading was clipped under that title when it was the first row.
pub const PROFILES_HEADING: &str = "Emulator profiles";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(&'static str),
    Note(&'static str),
    Core(CoreLine),
    Profile(ProfileLine),
    Kind {
        kind: Kind,
        aimed: bool,
    },
    Id {
        edit: LineEdit,
        aimed: bool,
    },
    Detail {
        edit: LineEdit,
        field_aimed: bool,
        browse_aimed: bool,
    },
    Add {
        aimed: bool,
    },
    Console(ConsoleLine),
    Close {
        aimed: bool,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emulators {
    profiles: Vec<EmulatorProfile>,
    consoles: Vec<ConsoleSlot>,
    cores: Vec<DiscoveredCore>,
    show_cores: bool,
    kind: Kind,
    id: LineEdit,
    detail: LineEdit,
    focus: Slot,
    error: Option<String>,
}

impl Emulators {
    pub fn open(config: &Config, mut cores: Vec<DiscoveredCore>, show_cores: bool) -> Self {
        if !show_cores {
            cores.clear();
        }
        Self {
            profiles: config.profiles.clone(),
            consoles: config
                .consoles
                .iter()
                .map(|console| ConsoleSlot {
                    id: console.id.clone(),
                    name: console.name.clone(),
                    profile: console.profile.clone().filter(|id| !id.is_empty()),
                })
                .collect(),
            cores,
            show_cores,
            kind: Kind::Standalone,
            id: LineEdit::plain(String::new()),
            detail: LineEdit::plain(String::new()),
            focus: Slot::Id,
            error: None,
        }
    }

    pub fn profiles(&self) -> &[EmulatorProfile] {
        &self.profiles
    }

    pub fn consoles(&self) -> &[ConsoleSlot] {
        &self.consoles
    }

    pub fn focus(&self) -> Slot {
        self.focus
    }

    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
    }

    pub fn set_kind(&mut self, kind: Kind) {
        self.kind = kind;
        self.focus = Slot::Kind;
    }

    /// GTK's file chooser fills the core path, switches the kind to RetroArch,
    /// and uses the file name as the id when that field is still empty.
    pub fn set_core_path(&mut self, path: PathBuf) {
        if self.id.text.is_empty() {
            if let Some(core) = DiscoveredCore::from_path(path.clone()) {
                self.id = LineEdit::plain(core.name);
            }
        }
        self.detail = LineEdit::plain(path.display().to_string());
        self.kind = Kind::RetroArch;
        self.focus = Slot::Detail;
    }

    pub fn aim(&mut self, slot: Slot) -> bool {
        if self.stops().iter().any(|stop| stop.slot == slot) {
            self.focus = slot;
            true
        } else {
            false
        }
    }

    /// `true` when a console's default profile changed and should be saved.
    pub fn move_dir(&mut self, dir: NavDir) -> bool {
        if let Slot::Console(index) = self.focus {
            if matches!(dir, NavDir::Left | NavDir::Right) {
                self.cycle_console(index, if dir == NavDir::Left { -1 } else { 1 });
                return true;
            }
        }
        if self.focus == Slot::Kind && matches!(dir, NavDir::Left | NavDir::Right) {
            let delta = if dir == NavDir::Left { -1 } else { 1 };
            self.kind = self.kind.cycle(delta);
            return false;
        }
        if self.focus == Slot::Detail && matches!(dir, NavDir::Left | NavDir::Right) {
            if dir == NavDir::Right && caret_at_end(&self.detail) {
                self.focus = Slot::Browse;
            } else if !(dir == NavDir::Left && caret_at_start(&self.detail)) {
                let delta = if dir == NavDir::Left { -1 } else { 1 };
                self.detail.move_caret(delta);
            }
            return false;
        }
        if self.focus == Slot::Browse && dir == NavDir::Left {
            self.focus = Slot::Detail;
            return false;
        }
        if self.focus == Slot::Id && matches!(dir, NavDir::Left | NavDir::Right) {
            let delta = if dir == NavDir::Left { -1 } else { 1 };
            self.id.move_caret(delta);
            return false;
        }

        let stops = self.stops();
        let Some(current) = stops.iter().find(|stop| stop.slot == self.focus) else {
            self.focus = Slot::Id;
            return false;
        };
        let delta: isize = match dir {
            NavDir::Left => -1,
            NavDir::Right => 1,
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
                return false;
            }
        };
        if let Some(next) = stops.iter().find(|stop| {
            stop.row == current.row && stop.col as isize == current.col as isize + delta
        }) {
            self.focus = next.slot;
        }
        false
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
        matches!(self.focus, Slot::Id | Slot::Detail)
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

    pub fn confirm(&mut self) -> EmulatorCommand {
        let focus = self.focus;
        let command = match focus {
            Slot::Core { index, action } => {
                let Some(action) = self.enabled_actions(index).get(action).copied() else {
                    return EmulatorCommand::None;
                };
                match action {
                    Action::Add => self.add_core(index, None),
                    Action::Assign(console) => self.add_core(index, Some(console)),
                }
                EmulatorCommand::Write
            }
            Slot::Delete(index) => {
                if index < self.profiles.len() {
                    self.profiles.remove(index);
                    EmulatorCommand::Write
                } else {
                    EmulatorCommand::None
                }
            }
            Slot::Kind => {
                self.kind = self.kind.cycle(1);
                EmulatorCommand::None
            }
            Slot::Id | Slot::Detail => EmulatorCommand::None,
            Slot::Browse => EmulatorCommand::Browse,
            Slot::Add => self.add_typed(),
            Slot::Console(index) => {
                self.cycle_console(index, 1);
                EmulatorCommand::Write
            }
            Slot::Close => EmulatorCommand::Close,
        };
        if command == EmulatorCommand::Write {
            self.error = None;
            self.settle();
        }
        command
    }

    pub fn blocks(&self) -> Vec<Block> {
        let mut blocks = vec![Block::Heading(PROFILES_HEADING)];
        if self.show_cores {
            blocks.push(Block::Note(
                "retroarch is on PATH. Pick a discovered core or a core file to make a RetroArch profile. Cores are not downloaded.",
            ));
            blocks.push(Block::Heading("Discovered cores"));
            if self.cores.is_empty() {
                blocks.push(Block::Note(
                    "No cores found in the usual directories. Paste a core path or browse for a .so file. Cores are not downloaded.",
                ));
            }
            for (index, core) in self.cores.iter().enumerate() {
                blocks.push(Block::Core(self.core_line(index, core)));
            }
        }
        for (index, profile) in self.profiles.iter().enumerate() {
            blocks.push(Block::Profile(ProfileLine {
                index,
                label: profile_label(profile),
                aimed: self.focus == Slot::Delete(index),
            }));
        }
        blocks.push(Block::Kind {
            kind: self.kind,
            aimed: self.focus == Slot::Kind,
        });
        blocks.push(Block::Id {
            edit: self.id.clone(),
            aimed: self.focus == Slot::Id,
        });
        blocks.push(Block::Detail {
            edit: self.detail.clone(),
            field_aimed: self.focus == Slot::Detail,
            browse_aimed: self.focus == Slot::Browse,
        });
        blocks.push(Block::Add {
            aimed: self.focus == Slot::Add,
        });
        if let Some(error) = &self.error {
            blocks.push(Block::Error(error.clone()));
        }
        blocks.push(Block::Heading("Default profile per system"));
        for (index, console) in self.consoles.iter().enumerate() {
            blocks.push(Block::Console(ConsoleLine {
                index,
                name: console.name.clone(),
                value: console
                    .profile
                    .clone()
                    .unwrap_or_else(|| "None".to_string()),
                aimed: self.focus == Slot::Console(index),
            }));
        }
        blocks.push(Block::Close {
            aimed: self.focus == Slot::Close,
        });
        blocks
    }

    /// Rows inside the scroller. [`PROFILES_HEADING`] is painted above it.
    pub fn scroll_blocks(&self) -> Vec<Block> {
        let mut blocks = self.blocks();
        if matches!(blocks.first(), Some(Block::Heading(PROFILES_HEADING))) {
            blocks.remove(0);
        }
        blocks
    }

    pub fn scroll_index(&self) -> usize {
        self.scroll_blocks()
            .iter()
            .position(|block| block_has_focus(block, self.focus))
            .unwrap_or(0)
    }

    fn core_line(&self, index: usize, core: &DiscoveredCore) -> CoreLine {
        let resolved = core.to_profile(&self.profiles);
        let added_id = match &resolved {
            CoreProfile::AlreadyAdded { id } => Some(id.clone()),
            CoreProfile::New(_) => None,
        };
        let mut buttons = Vec::new();
        let mut action = 0usize;
        match &resolved {
            CoreProfile::AlreadyAdded { .. } => buttons.push(CoreButton {
                label: "Added".to_string(),
                enabled: false,
                aimed: false,
                slot: None,
                primary: false,
            }),
            CoreProfile::New(_) => {
                let slot = Slot::Core { index, action };
                buttons.push(CoreButton {
                    label: "Add profile".to_string(),
                    enabled: true,
                    aimed: self.focus == slot,
                    slot: Some(slot),
                    primary: false,
                });
                action += 1;
            }
        }
        for system_id in core.system_ids() {
            let Some(console_index) = self
                .consoles
                .iter()
                .position(|console| console.id == *system_id)
            else {
                continue;
            };
            let console = &self.consoles[console_index];
            let assigned = added_id
                .as_ref()
                .is_some_and(|id| console.profile.as_ref() == Some(id));
            let label = if assigned {
                format!("Assigned to {}", console.name)
            } else if added_id.is_some() {
                format!("Assign to {}", console.name)
            } else {
                format!("Add & assign to {}", console.name)
            };
            if assigned {
                buttons.push(CoreButton {
                    label,
                    enabled: false,
                    aimed: false,
                    slot: None,
                    primary: true,
                });
            } else {
                let slot = Slot::Core { index, action };
                buttons.push(CoreButton {
                    label,
                    enabled: true,
                    aimed: self.focus == slot,
                    slot: Some(slot),
                    primary: true,
                });
                action += 1;
            }
        }
        CoreLine {
            index,
            name: core.name.clone(),
            path: core.path.display().to_string(),
            buttons,
        }
    }

    fn enabled_actions(&self, index: usize) -> Vec<Action> {
        let Some(core) = self.cores.get(index) else {
            return Vec::new();
        };
        let resolved = core.to_profile(&self.profiles);
        let mut actions = Vec::new();
        if matches!(resolved, CoreProfile::New(_)) {
            actions.push(Action::Add);
        }
        let added_id = match &resolved {
            CoreProfile::AlreadyAdded { id } => Some(id.as_str()),
            CoreProfile::New(_) => None,
        };
        for system_id in core.system_ids() {
            let Some(console_index) = self
                .consoles
                .iter()
                .position(|console| console.id == *system_id)
            else {
                continue;
            };
            let assigned = added_id
                .is_some_and(|id| self.consoles[console_index].profile.as_deref() == Some(id));
            if !assigned {
                actions.push(Action::Assign(console_index));
            }
        }
        actions
    }

    fn add_core(&mut self, index: usize, assign: Option<usize>) {
        let Some(core) = self.cores.get(index).cloned() else {
            return;
        };
        let id = match core.to_profile(&self.profiles) {
            CoreProfile::AlreadyAdded { id } => id,
            CoreProfile::New(profile) => {
                let id = profile.id().clone();
                self.profiles.push(profile);
                id
            }
        };
        if let Some(console_index) = assign {
            if let Some(console) = self.consoles.get_mut(console_index) {
                console.profile = Some(id);
            }
        }
    }

    fn add_typed(&mut self) -> EmulatorCommand {
        if self.id.text.is_empty() || self.detail.text.is_empty() {
            self.error = Some("Enter a profile id and a command or core path.".to_string());
            return EmulatorCommand::None;
        }
        let profile = match self.kind {
            Kind::RetroArch => EmulatorProfile::RetroArch {
                id: self.id.text.clone(),
                core: PathBuf::from(&self.detail.text),
                config: None,
            },
            Kind::Standalone => EmulatorProfile::Standalone {
                id: self.id.text.clone(),
                command: self.detail.text.clone(),
            },
        };
        self.profiles.push(profile);
        self.id = LineEdit::plain(String::new());
        self.detail = LineEdit::plain(String::new());
        self.error = None;
        EmulatorCommand::Write
    }

    fn cycle_console(&mut self, index: usize, delta: isize) {
        let ids: Vec<Option<String>> = std::iter::once(None)
            .chain(
                self.profiles
                    .iter()
                    .map(|profile| Some(profile.id().clone())),
            )
            .collect();
        let Some(console) = self.consoles.get_mut(index) else {
            return;
        };
        let pos = ids
            .iter()
            .position(|id| *id == console.profile)
            .unwrap_or(0);
        console.profile = ids[wrap(pos, ids.len(), delta)].clone();
    }

    fn settle(&mut self) {
        let stops = self.stops();
        if stops.iter().any(|stop| stop.slot == self.focus) {
            return;
        }
        self.focus = [Slot::Add, Slot::Id, Slot::Close]
            .into_iter()
            .find(|slot| stops.iter().any(|stop| stop.slot == *slot))
            .unwrap_or(Slot::Close);
    }

    fn stops(&self) -> Vec<Stop> {
        let mut stops = Vec::new();
        let mut row = 0usize;
        if self.show_cores {
            for index in 0..self.cores.len() {
                for (col, _) in self.enabled_actions(index).iter().enumerate() {
                    stops.push(Stop {
                        slot: Slot::Core { index, action: col },
                        row,
                        col,
                    });
                }
                row += 1;
            }
        }
        for index in 0..self.profiles.len() {
            stops.push(Stop {
                slot: Slot::Delete(index),
                row,
                col: 0,
            });
            row += 1;
        }
        for slot in [Slot::Kind, Slot::Id] {
            stops.push(Stop { slot, row, col: 0 });
            row += 1;
        }
        stops.push(Stop {
            slot: Slot::Detail,
            row,
            col: 0,
        });
        stops.push(Stop {
            slot: Slot::Browse,
            row,
            col: 1,
        });
        row += 1;
        stops.push(Stop {
            slot: Slot::Add,
            row,
            col: 0,
        });
        row += 1;
        for index in 0..self.consoles.len() {
            stops.push(Stop {
                slot: Slot::Console(index),
                row,
                col: 0,
            });
            row += 1;
        }
        stops.push(Stop {
            slot: Slot::Close,
            row,
            col: 0,
        });
        stops
    }

    fn edit_mut(&mut self) -> Option<&mut LineEdit> {
        match self.focus {
            Slot::Id => Some(&mut self.id),
            Slot::Detail => Some(&mut self.detail),
            _ => None,
        }
    }
}

/// Copy the dialog onto a config. Other fields (theme, cover width, scraper)
/// stay as they were. Console rows the dialog does not know about are left alone.
pub fn apply_assignments(
    config: &mut Config,
    profiles: &[EmulatorProfile],
    consoles: &[ConsoleSlot],
) {
    config.profiles = profiles.to_vec();
    for console in &mut config.consoles {
        if let Some(slot) = consoles.iter().find(|slot| slot.id == console.id) {
            console.profile = slot.profile.clone().filter(|id| !id.is_empty());
        }
    }
}

fn block_has_focus(block: &Block, focus: Slot) -> bool {
    match block {
        Block::Core(line) => line.buttons.iter().any(|button| button.slot == Some(focus)),
        Block::Profile(line) => focus == Slot::Delete(line.index),
        Block::Kind { .. } => focus == Slot::Kind,
        Block::Id { .. } => focus == Slot::Id,
        Block::Detail { .. } => matches!(focus, Slot::Detail | Slot::Browse),
        Block::Add { .. } => focus == Slot::Add,
        Block::Console(line) => focus == Slot::Console(line.index),
        Block::Close { .. } => focus == Slot::Close,
        Block::Heading(_) | Block::Note(_) | Block::Error(_) => false,
    }
}

fn profile_label(profile: &EmulatorProfile) -> String {
    match profile {
        EmulatorProfile::RetroArch { id, core, .. } => {
            format!("{id} · RetroArch · {}", core.display())
        }
        EmulatorProfile::Standalone { id, command } => format!("{id} · {command}"),
    }
}

fn caret_at_end(edit: &LineEdit) -> bool {
    edit.replace || edit.caret >= edit.text.chars().count()
}

fn caret_at_start(edit: &LineEdit) -> bool {
    !edit.replace && edit.caret == 0
}

fn wrap(pos: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (pos as isize + delta).rem_euclid(len as isize) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{self, Config};
    use crate::types::{Console, GridArt, MediaToggles};

    fn snes() -> Console {
        Console {
            id: "snes".into(),
            name: "Super Nintendo".into(),
            rom_dirs: vec![PathBuf::from("/tmp/roms/snes")],
            extensions: vec!["sfc".into()],
            profile: None,
            grid_art: GridArt::BoxArt,
            media: MediaToggles::default(),
        }
    }

    fn snes9x() -> DiscoveredCore {
        DiscoveredCore::from_path(PathBuf::from("/tmp/cores/snes9x_libretro.so")).unwrap()
    }

    fn dialog() -> Emulators {
        let mut config = Config::default();
        config.consoles.push(snes());
        Emulators::open(&config, vec![snes9x()], true)
    }

    #[test]
    fn profiles_heading_is_not_inside_the_scroller() {
        let dialog = dialog();
        assert_eq!(dialog.blocks()[0], Block::Heading(PROFILES_HEADING));
        assert!(dialog
            .scroll_blocks()
            .iter()
            .all(|block| block != &Block::Heading(PROFILES_HEADING)));
        let id = dialog
            .scroll_blocks()
            .iter()
            .position(|block| matches!(block, Block::Id { aimed: true, .. }))
            .unwrap();
        assert_eq!(dialog.scroll_index(), id);
    }

    #[test]
    fn ctrl_e_and_ctrl_m_match_the_gtk_chords() {
        assert!(emulator_key("m", None, true));
        assert!(emulator_key("M", None, true));
        assert!(emulator_key("e", Some("E"), true));
        assert!(emulator_key("E", None, true));
        assert!(!emulator_key("m", None, false));
        assert!(!emulator_key("i", Some("i"), true));
    }

    #[test]
    fn hidden_cores_are_not_stops_when_retroarch_is_absent() {
        let mut config = Config::default();
        config.consoles.push(snes());
        let mut dialog = Emulators::open(&config, vec![snes9x()], false);
        assert!(dialog
            .blocks()
            .iter()
            .all(|block| !matches!(block, Block::Core(_))));
        dialog.move_dir(NavDir::Up);
        assert_eq!(dialog.focus(), Slot::Kind);
    }

    #[test]
    fn add_and_assign_uses_the_discovered_core_profile() {
        let mut dialog = dialog();
        assert_eq!(dialog.focus(), Slot::Id);
        assert!(!dialog.move_dir(NavDir::Up));
        assert!(!dialog.move_dir(NavDir::Up));
        assert_eq!(
            dialog.focus(),
            Slot::Core {
                index: 0,
                action: 0
            }
        );
        dialog.move_dir(NavDir::Right);
        assert_eq!(
            dialog.focus(),
            Slot::Core {
                index: 0,
                action: 1
            }
        );
        assert_eq!(dialog.confirm(), EmulatorCommand::Write);
        match &dialog.profiles()[0] {
            EmulatorProfile::RetroArch { id, core, config } => {
                assert_eq!(id, "snes9x");
                assert_eq!(core, &PathBuf::from("/tmp/cores/snes9x_libretro.so"));
                assert!(config.is_none());
            }
            EmulatorProfile::Standalone { .. } => panic!("expected RetroArch"),
        }
        assert_eq!(dialog.consoles()[0].profile.as_deref(), Some("snes9x"));
        let line = dialog.blocks().into_iter().find_map(|block| match block {
            Block::Core(line) => Some(line),
            _ => None,
        });
        let line = line.unwrap();
        assert_eq!(line.buttons[0].label, "Added");
        assert!(!line.buttons[0].enabled);
        assert_eq!(line.buttons[1].label, "Assigned to Super Nintendo");
        assert!(!line.buttons[1].enabled);
    }

    #[test]
    fn core_then_standalone_then_console_assignment() {
        let mut dialog = dialog();
        dialog.move_dir(NavDir::Up);
        dialog.move_dir(NavDir::Up);
        assert_eq!(dialog.confirm(), EmulatorCommand::Write);
        assert_eq!(
            dialog.focus(),
            Slot::Core {
                index: 0,
                action: 0
            }
        );
        dialog.tab(false);
        dialog.tab(false);
        dialog.tab(false);
        assert_eq!(dialog.focus(), Slot::Id);
        dialog.type_text("stub");
        dialog.tab(false);
        assert_eq!(dialog.focus(), Slot::Detail);
        dialog.type_text("/tmp/stub.sh {rom}");
        dialog.tab(false);
        dialog.tab(false);
        assert_eq!(dialog.focus(), Slot::Add);
        assert_eq!(dialog.confirm(), EmulatorCommand::Write);
        dialog.tab(false);
        assert_eq!(dialog.focus(), Slot::Console(0));
        assert_eq!(dialog.consoles()[0].profile, None);
        assert!(dialog.move_dir(NavDir::Right));
        assert_eq!(dialog.consoles()[0].profile.as_deref(), Some("snes9x"));
        assert!(dialog.move_dir(NavDir::Right));
        assert_eq!(dialog.consoles()[0].profile.as_deref(), Some("stub"));
        match &dialog.profiles()[1] {
            EmulatorProfile::Standalone { id, command } => {
                assert_eq!(id, "stub");
                assert_eq!(command, "/tmp/stub.sh {rom}");
            }
            EmulatorProfile::RetroArch { .. } => panic!("expected standalone"),
        }
    }

    #[test]
    fn delete_keeps_the_console_profile_id() {
        let mut dialog = dialog();
        dialog.move_dir(NavDir::Up);
        dialog.move_dir(NavDir::Up);
        dialog.move_dir(NavDir::Right);
        dialog.confirm();
        assert!(dialog.aim(Slot::Delete(0)));
        assert_eq!(dialog.confirm(), EmulatorCommand::Write);
        assert!(dialog.profiles().is_empty());
        assert_eq!(dialog.consoles()[0].profile.as_deref(), Some("snes9x"));
    }

    #[test]
    fn empty_add_does_not_write() {
        let mut dialog = dialog();
        assert!(dialog.aim(Slot::Add));
        assert_eq!(dialog.confirm(), EmulatorCommand::None);
        assert!(dialog.profiles().is_empty());
        assert!(dialog
            .blocks()
            .iter()
            .any(|block| matches!(block, Block::Error(_))));
    }

    #[test]
    fn browse_fills_a_retroarch_id_from_the_file_name() {
        let mut dialog = dialog();
        dialog.set_core_path(PathBuf::from("/tmp/cores/fceumm_libretro.so"));
        assert_eq!(dialog.focus(), Slot::Detail);
        assert!(dialog.blocks().iter().any(|block| matches!(
            block,
            Block::Kind {
                kind: Kind::RetroArch,
                ..
            }
        )));
        dialog.aim(Slot::Add);
        assert_eq!(dialog.confirm(), EmulatorCommand::Write);
        match &dialog.profiles()[0] {
            EmulatorProfile::RetroArch { id, core, config } => {
                assert_eq!(id, "fceumm");
                assert_eq!(core, &PathBuf::from("/tmp/cores/fceumm_libretro.so"));
                assert!(config.is_none());
            }
            EmulatorProfile::Standalone { .. } => panic!("expected RetroArch"),
        }
    }

    #[test]
    fn manual_kind_cycle_writes_a_retroarch_profile() {
        let mut dialog = dialog();
        dialog.move_dir(NavDir::Up);
        assert_eq!(dialog.focus(), Slot::Kind);
        assert!(!dialog.move_dir(NavDir::Right));
        dialog.move_dir(NavDir::Down);
        dialog.type_text("handy");
        dialog.move_dir(NavDir::Down);
        dialog.type_text("/tmp/cores/handy_libretro.so");
        dialog.aim(Slot::Add);
        dialog.confirm();
        match &dialog.profiles()[0] {
            EmulatorProfile::RetroArch { id, core, .. } => {
                assert_eq!(id, "handy");
                assert_eq!(core, &PathBuf::from("/tmp/cores/handy_libretro.so"));
            }
            EmulatorProfile::Standalone { .. } => panic!("expected RetroArch"),
        }
    }

    #[test]
    fn save_round_trips_profiles_and_leaves_the_rest_of_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let _env = config::XdgEnv::sandbox(dir.path());
        let mut config = Config::default();
        config.theme = "custom".into();
        config.cover_width = 250.0;
        config.consoles.push(snes());
        config::save_config(&config).unwrap();

        let mut dialog = Emulators::open(&config::load_config().unwrap(), vec![snes9x()], true);
        dialog.move_dir(NavDir::Up);
        dialog.move_dir(NavDir::Up);
        dialog.confirm();
        dialog.tab(false);
        dialog.tab(false);
        dialog.tab(false);
        dialog.type_text("stub");
        dialog.tab(false);
        dialog.type_text("stub.sh {rom}");
        dialog.tab(false);
        dialog.tab(false);
        dialog.confirm();
        dialog.tab(false);
        dialog.move_dir(NavDir::Right);
        dialog.move_dir(NavDir::Right);

        let mut stored = config::load_config().unwrap();
        apply_assignments(&mut stored, dialog.profiles(), dialog.consoles());
        config::save_config(&stored).unwrap();
        let loaded = config::load_config().unwrap();
        assert_eq!(loaded.theme, "custom");
        assert_eq!(loaded.cover_width, 250.0);
        assert_eq!(
            loaded.consoles[0].rom_dirs,
            vec![PathBuf::from("/tmp/roms/snes")]
        );
        assert_eq!(loaded.consoles[0].profile.as_deref(), Some("stub"));
        assert_eq!(loaded.profiles.len(), 2);
        let text = std::fs::read_to_string(config::config_path().unwrap()).unwrap();
        assert!(
            text.contains("type = 'RetroArch'") || text.contains("type = \"RetroArch\""),
            "{text}"
        );
        assert!(text.contains("snes9x_libretro.so"), "{text}");
        assert!(
            text.contains("stub.sh {{rom}}") || text.contains("stub.sh {rom}"),
            "{text}"
        );
        assert!(!text.contains("\nconfig ="), "{text}");
    }
}
