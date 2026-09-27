//! Manage Emulators. Two tabs share one scroll region.
//! Emulators are installed programs. Systems assign one emulator, a RetroArch
//! core, and extra arguments.

use crate::config::Config;
use crate::cores::DiscoveredCore;
use crate::game_menu::LineEdit;
use crate::gamepad::NavDir;
use crate::types::{Console, Emulator, EmulatorKind};
use std::path::PathBuf;

/// Ctrl+E and Ctrl+M, including the shifted keysyms. The control mask must be set.
pub fn emulator_key(key: &str, key_char: Option<&str>, control: bool) -> bool {
    if !control {
        return false;
    }
    let hit = |name: &str| {
        key.eq_ignore_ascii_case(name) || key_char.is_some_and(|ch| ch.eq_ignore_ascii_case(name))
    };
    hit("e") || hit("m")
}

pub const HINT: &str = "Tab, 1, and 2 switch tabs. Arrows move. Left and right change a choice. Enter confirms. Esc closes.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Emulators,
    Systems,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Emulators => "Emulators",
            Self::Systems => "Systems",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Tabs,
    Row(usize),
    Delete(usize),
    Name,
    Kind,
    Path,
    GlobalArgs,
    Save,
    SystemEmulator(usize),
    SystemCore(usize),
    SystemArgs(usize),
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
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmulatorLine {
    pub index: usize,
    pub name: String,
    pub detail: String,
    pub row_aimed: bool,
    pub delete_aimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemLine {
    pub index: usize,
    pub name: String,
    pub emulator: String,
    pub emulator_aimed: bool,
    pub core: String,
    pub core_enabled: bool,
    pub core_aimed: bool,
    pub args: LineEdit,
    pub args_aimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Note(&'static str),
    Error(String),
    Emulator(EmulatorLine),
    Label(&'static str),
    Field {
        edit: LineEdit,
        aimed: bool,
        placeholder: &'static str,
        slot: Slot,
    },
    Kind {
        kind: EmulatorKind,
        aimed: bool,
    },
    Save {
        label: &'static str,
        aimed: bool,
    },
    System(SystemLine),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemRow {
    pub id: String,
    pub name: String,
    pub emulator_id: Option<String>,
    pub core: Option<PathBuf>,
    pub args: LineEdit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Draft {
    editing: Option<usize>,
    name: LineEdit,
    kind: EmulatorKind,
    path: LineEdit,
    args: LineEdit,
}

impl Draft {
    fn blank() -> Self {
        Self {
            editing: None,
            name: LineEdit::plain(String::new()),
            kind: EmulatorKind::Standalone,
            path: LineEdit::plain(String::new()),
            args: LineEdit::plain(String::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emulators {
    emulators: Vec<Emulator>,
    systems: Vec<SystemRow>,
    cores: Vec<DiscoveredCore>,
    tab: Tab,
    draft: Draft,
    focus: Slot,
    error: Option<String>,
}

impl Emulators {
    pub fn open(config: &Config, cores: Vec<DiscoveredCore>) -> Self {
        Self {
            emulators: config.emulators.clone(),
            systems: config.consoles.iter().map(system_row).collect(),
            cores,
            tab: Tab::Emulators,
            draft: Draft::blank(),
            focus: Slot::Tabs,
            error: None,
        }
    }

    pub fn emulators(&self) -> &[Emulator] {
        &self.emulators
    }

    pub fn systems(&self) -> &[SystemRow] {
        &self.systems
    }

    pub fn focus(&self) -> Slot {
        self.focus
    }

    pub fn tab(&self) -> Tab {
        self.tab
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

    pub fn show(&mut self, tab: Tab) {
        self.tab = tab;
        self.focus = Slot::Tabs;
    }

    pub fn set_kind(&mut self, kind: EmulatorKind) {
        self.draft.kind = kind;
        self.focus = Slot::Kind;
        if kind == EmulatorKind::RetroArch && self.draft.path.text.trim().is_empty() {
            self.draft.path = LineEdit::plain("retroarch".to_string());
        }
    }

    pub fn cycle_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Emulators => Tab::Systems,
            Tab::Systems => Tab::Emulators,
        };
        self.focus = Slot::Tabs;
    }

    /// `true` when a system assignment changed and should be saved.
    pub fn move_dir(&mut self, dir: NavDir) -> bool {
        if matches!(dir, NavDir::Left | NavDir::Right) {
            if let Some(edit) = self.edit_mut() {
                let delta = if dir == NavDir::Left { -1 } else { 1 };
                edit.move_caret(delta);
                return false;
            }
        }
        if self.focus == Slot::Tabs && matches!(dir, NavDir::Left | NavDir::Right) {
            self.step_tab(if dir == NavDir::Left { -1 } else { 1 });
            return false;
        }
        if self.focus == Slot::Kind && matches!(dir, NavDir::Left | NavDir::Right) {
            self.cycle_kind(if dir == NavDir::Left { -1 } else { 1 });
            return false;
        }
        if let Slot::SystemEmulator(index) = self.focus {
            if matches!(dir, NavDir::Left | NavDir::Right) {
                self.cycle_emulator(index, if dir == NavDir::Left { -1 } else { 1 });
                return true;
            }
        }
        if let Slot::SystemCore(index) = self.focus {
            if matches!(dir, NavDir::Left | NavDir::Right) {
                self.cycle_core(index, if dir == NavDir::Left { -1 } else { 1 });
                return true;
            }
        }

        let leaving_args = matches!(self.focus, Slot::SystemArgs(_));
        let stops = self.stops();
        let Some(current) = stops.iter().find(|stop| stop.slot == self.focus) else {
            self.focus = Slot::Tabs;
            return false;
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
        leaving_args && !matches!(self.focus, Slot::SystemArgs(_))
    }

    pub fn accepts_text(&self) -> bool {
        matches!(
            self.focus,
            Slot::Name | Slot::Path | Slot::GlobalArgs | Slot::SystemArgs(_)
        )
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
        let command = match self.focus {
            Slot::Tabs => {
                self.enter_tab();
                EmulatorCommand::None
            }
            Slot::Row(index) => {
                self.begin_edit(index);
                EmulatorCommand::None
            }
            Slot::Delete(index) => {
                if index < self.emulators.len() {
                    self.delete_at(index);
                    EmulatorCommand::Write
                } else {
                    EmulatorCommand::None
                }
            }
            Slot::Kind => {
                self.cycle_kind(1);
                EmulatorCommand::None
            }
            Slot::Name | Slot::Path | Slot::GlobalArgs => {
                self.focus_next();
                EmulatorCommand::None
            }
            Slot::Save => self.save(),
            Slot::SystemEmulator(index) => {
                self.cycle_emulator(index, 1);
                EmulatorCommand::Write
            }
            Slot::SystemCore(index) => {
                self.cycle_core(index, 1);
                EmulatorCommand::Write
            }
            Slot::SystemArgs(_) => EmulatorCommand::Write,
            Slot::Close => EmulatorCommand::Close,
        };
        if command == EmulatorCommand::Write {
            self.error = None;
        }
        command
    }

    pub fn blocks(&self) -> Vec<Block> {
        let mut blocks = Vec::new();
        match self.tab {
            Tab::Emulators => self.emulator_blocks(&mut blocks),
            Tab::Systems => self.system_blocks(&mut blocks),
        }
        if let Some(error) = &self.error {
            if self.tab == Tab::Emulators {
                blocks.push(Block::Error(error.clone()));
            }
        }
        blocks
    }

    pub fn scroll_blocks(&self) -> Vec<Block> {
        self.blocks()
    }

    pub fn scroll_index(&self) -> usize {
        self.scroll_blocks()
            .iter()
            .position(|block| block_has_focus(block, self.focus))
            .unwrap_or(0)
    }

    fn emulator_blocks(&self, blocks: &mut Vec<Block>) {
        if self.emulators.is_empty() {
            blocks.push(Block::Note("No emulators yet."));
        }
        for (index, emulator) in self.emulators.iter().enumerate() {
            blocks.push(Block::Emulator(EmulatorLine {
                index,
                name: emulator.name.clone(),
                detail: emulator_detail(emulator),
                row_aimed: self.focus == Slot::Row(index),
                delete_aimed: self.focus == Slot::Delete(index),
            }));
        }
        let editing = self.draft.editing.is_some();
        blocks.push(Block::Label(if editing {
            "Edit emulator"
        } else {
            "Add emulator"
        }));
        blocks.push(Block::Label("Name"));
        blocks.push(Block::Field {
            edit: self.draft.name.clone(),
            aimed: self.focus == Slot::Name,
            placeholder: "Name",
            slot: Slot::Name,
        });
        blocks.push(Block::Label("Kind"));
        blocks.push(Block::Kind {
            kind: self.draft.kind,
            aimed: self.focus == Slot::Kind,
        });
        blocks.push(Block::Label("Executable"));
        blocks.push(Block::Field {
            edit: self.draft.path.clone(),
            aimed: self.focus == Slot::Path,
            placeholder: "Executable path",
            slot: Slot::Path,
        });
        blocks.push(Block::Label("Global arguments"));
        blocks.push(Block::Field {
            edit: self.draft.args.clone(),
            aimed: self.focus == Slot::GlobalArgs,
            placeholder: "Global arguments",
            slot: Slot::GlobalArgs,
        });
        blocks.push(Block::Save {
            label: if editing { "Save" } else { "Add" },
            aimed: self.focus == Slot::Save,
        });
    }

    fn system_blocks(&self, blocks: &mut Vec<Block>) {
        if self.systems.is_empty() {
            blocks.push(Block::Note(
                "No systems in the library. Import ROMs, then assign an emulator here.",
            ));
            return;
        }
        if self.emulators.is_empty() {
            blocks.push(Block::Note(
                "Add an emulator on the Emulators tab, then assign it here.",
            ));
        }
        for (index, system) in self.systems.iter().enumerate() {
            let retroarch = self.retroarch_selected(index);
            blocks.push(Block::System(SystemLine {
                index,
                name: system.name.clone(),
                emulator: self.emulator_label(system.emulator_id.as_deref()),
                emulator_aimed: self.focus == Slot::SystemEmulator(index),
                core: core_label(&self.cores, system.core.as_deref()),
                core_enabled: retroarch,
                core_aimed: self.focus == Slot::SystemCore(index),
                args: system.args.clone(),
                args_aimed: self.focus == Slot::SystemArgs(index),
            }));
        }
    }

    fn emulator_label(&self, id: Option<&str>) -> String {
        let Some(id) = id else {
            return "None".to_string();
        };
        self.emulators
            .iter()
            .find(|emulator| emulator.id == id)
            .map(|emulator| emulator.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    fn step_tab(&mut self, delta: isize) {
        let next = match (self.tab, delta) {
            (Tab::Emulators, 1) => Tab::Systems,
            (Tab::Systems, -1) => Tab::Emulators,
            _ => self.tab,
        };
        self.tab = next;
        self.focus = Slot::Tabs;
    }

    fn enter_tab(&mut self) {
        let stops = self.stops();
        if let Some(next) = stops.iter().find(|stop| stop.slot != Slot::Tabs) {
            self.focus = next.slot;
        }
    }

    fn focus_next(&mut self) {
        let stops = self.stops();
        let Some(pos) = stops.iter().position(|stop| stop.slot == self.focus) else {
            return;
        };
        if let Some(next) = stops.get(pos + 1) {
            self.focus = next.slot;
        }
    }

    fn begin_edit(&mut self, index: usize) {
        let Some(emulator) = self.emulators.get(index) else {
            return;
        };
        self.draft = Draft {
            editing: Some(index),
            name: LineEdit::plain(emulator.name.clone()),
            kind: emulator.kind,
            path: LineEdit::plain(emulator.path.clone()),
            args: LineEdit::plain(emulator.global_args.clone()),
        };
        self.tab = Tab::Emulators;
        self.focus = Slot::Name;
        self.error = None;
    }

    fn save(&mut self) -> EmulatorCommand {
        let name = self.draft.name.text.trim().to_string();
        let path = self.draft.path.text.trim().to_string();
        if name.is_empty() || path.is_empty() {
            self.error = Some("Enter a name and an executable path.".to_string());
            return EmulatorCommand::None;
        }
        let kind = self.draft.kind;
        let global_args = self.draft.args.text.trim().to_string();
        if let Some(index) = self.draft.editing {
            let Some(emulator) = self.emulators.get_mut(index) else {
                return EmulatorCommand::None;
            };
            let id = emulator.id.clone();
            let was = emulator.kind;
            emulator.name = name;
            emulator.kind = kind;
            emulator.path = path;
            emulator.global_args = global_args;
            if was == EmulatorKind::RetroArch && kind != EmulatorKind::RetroArch {
                self.clear_cores(&id);
            }
            self.draft = Draft::blank();
            self.error = None;
            self.focus = Slot::Row(index);
            EmulatorCommand::Write
        } else {
            let id = fresh_id(&slug(&name), &self.emulators);
            self.emulators.push(Emulator {
                id,
                name,
                kind,
                path,
                global_args,
            });
            let index = self.emulators.len() - 1;
            self.draft = Draft::blank();
            self.error = None;
            self.focus = Slot::Row(index);
            EmulatorCommand::Write
        }
    }

    fn delete_at(&mut self, index: usize) {
        let id = self.emulators[index].id.clone();
        self.emulators.remove(index);
        for system in &mut self.systems {
            if system.emulator_id.as_ref() == Some(&id) {
                system.emulator_id = None;
                system.core = None;
            }
        }
        if self.draft.editing == Some(index) {
            self.draft = Draft::blank();
        } else if let Some(editing) = self.draft.editing.as_mut() {
            if *editing > index {
                *editing -= 1;
            }
        }
        self.settle();
    }

    fn clear_cores(&mut self, id: &str) {
        for system in &mut self.systems {
            if system.emulator_id.as_deref() == Some(id) {
                system.core = None;
            }
        }
    }

    fn cycle_kind(&mut self, delta: isize) {
        let kinds = [EmulatorKind::Standalone, EmulatorKind::RetroArch];
        let pos = kinds
            .iter()
            .position(|kind| *kind == self.draft.kind)
            .unwrap_or(0);
        self.draft.kind = kinds[wrap(pos, kinds.len(), delta)];
        if self.draft.kind == EmulatorKind::RetroArch && self.draft.path.text.trim().is_empty() {
            self.draft.path = LineEdit::plain("retroarch".to_string());
        }
    }

    fn cycle_emulator(&mut self, index: usize, delta: isize) {
        let ids: Vec<Option<String>> = std::iter::once(None)
            .chain(
                self.emulators
                    .iter()
                    .map(|emulator| Some(emulator.id.clone())),
            )
            .collect();
        let Some(current) = self
            .systems
            .get(index)
            .map(|system| system.emulator_id.clone())
        else {
            return;
        };
        let pos = ids.iter().position(|id| *id == current).unwrap_or(0);
        let chosen = ids[wrap(pos, ids.len(), delta)].clone();
        let retroarch = chosen.as_ref().is_some_and(|id| {
            self.emulators
                .iter()
                .any(|emulator| emulator.id == *id && emulator.kind == EmulatorKind::RetroArch)
        });
        let Some(system) = self.systems.get_mut(index) else {
            return;
        };
        system.emulator_id = chosen;
        if !retroarch {
            system.core = None;
        }
        if !retroarch && self.focus == Slot::SystemCore(index) {
            self.focus = Slot::SystemEmulator(index);
        }
    }

    fn cycle_core(&mut self, index: usize, delta: isize) {
        if !self.retroarch_selected(index) {
            return;
        }
        let choices = self.core_choices(index);
        let Some(system) = self.systems.get_mut(index) else {
            return;
        };
        let pos = choices
            .iter()
            .position(|path| *path == system.core)
            .unwrap_or(0);
        system.core = choices[wrap(pos, choices.len(), delta)].clone();
    }

    fn core_choices(&self, index: usize) -> Vec<Option<PathBuf>> {
        let mut choices = vec![None];
        for core in &self.cores {
            if !choices.iter().any(|path| path.as_ref() == Some(&core.path)) {
                choices.push(Some(core.path.clone()));
            }
        }
        if let Some(path) = self
            .systems
            .get(index)
            .and_then(|system| system.core.clone())
        {
            if !choices.iter().any(|choice| choice.as_ref() == Some(&path)) {
                choices.push(Some(path));
            }
        }
        choices
    }

    fn retroarch_selected(&self, index: usize) -> bool {
        let Some(id) = self
            .systems
            .get(index)
            .and_then(|system| system.emulator_id.clone())
        else {
            return false;
        };
        self.emulators
            .iter()
            .any(|emulator| emulator.id == id && emulator.kind == EmulatorKind::RetroArch)
    }

    fn settle(&mut self) {
        let stops = self.stops();
        if stops.iter().any(|stop| stop.slot == self.focus) {
            return;
        }
        self.focus = stops
            .iter()
            .find(|stop| stop.slot != Slot::Tabs)
            .map(|stop| stop.slot)
            .unwrap_or(Slot::Close);
    }

    fn stops(&self) -> Vec<Stop> {
        let mut stops = vec![Stop {
            slot: Slot::Tabs,
            row: 0,
            col: 0,
        }];
        let mut row = 1usize;
        match self.tab {
            Tab::Emulators => {
                for index in 0..self.emulators.len() {
                    stops.push(Stop {
                        slot: Slot::Row(index),
                        row,
                        col: 0,
                    });
                    stops.push(Stop {
                        slot: Slot::Delete(index),
                        row,
                        col: 1,
                    });
                    row += 1;
                }
                for slot in [
                    Slot::Name,
                    Slot::Kind,
                    Slot::Path,
                    Slot::GlobalArgs,
                    Slot::Save,
                ] {
                    stops.push(Stop { slot, row, col: 0 });
                    row += 1;
                }
            }
            Tab::Systems => {
                for index in 0..self.systems.len() {
                    stops.push(Stop {
                        slot: Slot::SystemEmulator(index),
                        row,
                        col: 0,
                    });
                    row += 1;
                    if self.retroarch_selected(index) {
                        stops.push(Stop {
                            slot: Slot::SystemCore(index),
                            row,
                            col: 0,
                        });
                        row += 1;
                    }
                    stops.push(Stop {
                        slot: Slot::SystemArgs(index),
                        row,
                        col: 0,
                    });
                    row += 1;
                }
            }
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
            Slot::Name => Some(&mut self.draft.name),
            Slot::Path => Some(&mut self.draft.path),
            Slot::GlobalArgs => Some(&mut self.draft.args),
            Slot::SystemArgs(index) => self.systems.get_mut(index).map(|system| &mut system.args),
            _ => None,
        }
    }
}

pub fn apply_assignments(config: &mut Config, emulators: &[Emulator], systems: &[SystemRow]) {
    config.emulators = emulators.to_vec();
    for console in &mut config.consoles {
        let Some(row) = systems.iter().find(|row| row.id == console.id) else {
            continue;
        };
        let retroarch = row.emulator_id.as_ref().is_some_and(|id| {
            emulators
                .iter()
                .any(|emulator| emulator.id == *id && emulator.kind == EmulatorKind::RetroArch)
        });
        console.emulator = row.emulator_id.clone().filter(|id| !id.is_empty());
        console.core = if retroarch { row.core.clone() } else { None };
        console.extra_args = row.args.text.clone();
    }
}

fn system_row(console: &Console) -> SystemRow {
    SystemRow {
        id: console.id.clone(),
        name: console.name.clone(),
        emulator_id: console.emulator.clone().filter(|id| !id.trim().is_empty()),
        core: console.core.clone(),
        args: LineEdit::plain(console.extra_args.clone()),
    }
}

fn emulator_detail(emulator: &Emulator) -> String {
    if emulator.global_args.trim().is_empty() {
        format!("{} · {}", emulator.kind.label(), emulator.path)
    } else {
        format!(
            "{} · {} · {}",
            emulator.kind.label(),
            emulator.path,
            emulator.global_args.trim()
        )
    }
}

fn core_label(cores: &[DiscoveredCore], path: Option<&std::path::Path>) -> String {
    let Some(path) = path else {
        return "No core".to_string();
    };
    cores
        .iter()
        .find(|core| core.path == path)
        .map(|core| core.name.clone())
        .unwrap_or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("core")
                .to_string()
        })
}

fn block_has_focus(block: &Block, focus: Slot) -> bool {
    match block {
        Block::Emulator(line) => {
            focus == Slot::Row(line.index) || focus == Slot::Delete(line.index)
        }
        Block::Field { slot, .. } => focus == *slot,
        Block::Kind { .. } => focus == Slot::Kind,
        Block::Save { .. } => focus == Slot::Save,
        Block::System(line) => {
            focus == Slot::SystemEmulator(line.index)
                || focus == Slot::SystemCore(line.index)
                || focus == Slot::SystemArgs(line.index)
        }
        Block::Note(_) | Block::Error(_) | Block::Label(_) => false,
    }
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "emulator".to_string()
    } else {
        out
    }
}

fn fresh_id(base: &str, emulators: &[Emulator]) -> String {
    if !emulators.iter().any(|emulator| emulator.id == base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !emulators.iter().any(|emulator| emulator.id == candidate) {
            return candidate;
        }
        n += 1;
    }
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
    use crate::config::Config;
    use crate::types::{EmulatorKind, GridArt, MediaToggles};

    fn snes() -> Console {
        Console {
            id: "snes".into(),
            name: "Super Nintendo".into(),
            rom_dirs: vec![PathBuf::from("/tmp/roms/snes")],
            extensions: vec!["sfc".into()],
            emulator: None,
            core: None,
            extra_args: String::new(),
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
        Emulators::open(&config, vec![snes9x()])
    }

    fn retroarch() -> Emulator {
        Emulator {
            id: "retroarch".into(),
            name: "RetroArch".into(),
            kind: EmulatorKind::RetroArch,
            path: "retroarch".into(),
            global_args: String::new(),
        }
    }

    #[test]
    fn ctrl_e_and_ctrl_m_open_emulators() {
        assert!(emulator_key("m", None, true));
        assert!(emulator_key("M", None, true));
        assert!(emulator_key("e", Some("E"), true));
        assert!(emulator_key("E", None, true));
        assert!(!emulator_key("m", None, false));
        assert!(!emulator_key("i", Some("i"), true));
    }

    #[test]
    fn the_scroller_is_one_tab_and_has_no_profile_id() {
        let dialog = dialog();
        assert_eq!(dialog.tab(), Tab::Emulators);
        assert_eq!(dialog.focus(), Slot::Tabs);
        let blocks = dialog.scroll_blocks();
        assert!(blocks.iter().any(|block| matches!(
            block,
            Block::Field {
                slot: Slot::Name,
                ..
            }
        )));
        assert!(blocks.iter().all(|block| !matches!(
            block,
            Block::Label(label) if label.contains("Profile")
        )));
    }

    #[test]
    fn tab_keys_switch_tabs_and_leave_one_list() {
        let mut dialog = dialog();
        dialog.cycle_tab();
        assert_eq!(dialog.tab(), Tab::Systems);
        assert_eq!(dialog.focus(), Slot::Tabs);
        assert!(dialog
            .scroll_blocks()
            .iter()
            .any(|block| matches!(block, Block::System(_))));
        assert!(dialog
            .scroll_blocks()
            .iter()
            .all(|block| !matches!(block, Block::Emulator(_))));
        dialog.show(Tab::Emulators);
        assert!(dialog
            .scroll_blocks()
            .iter()
            .all(|block| !matches!(block, Block::System(_))));
    }

    #[test]
    fn add_edit_and_delete_an_emulator() {
        let mut dialog = dialog();
        dialog.aim(Slot::Name);
        dialog.type_text("Dolphin");
        dialog.aim(Slot::Path);
        dialog.type_text("dolphin-emu");
        dialog.aim(Slot::GlobalArgs);
        dialog.type_text("-b -e {rom}");
        assert_eq!(dialog.confirm(), EmulatorCommand::None);
        dialog.aim(Slot::Save);
        assert_eq!(dialog.confirm(), EmulatorCommand::Write);
        assert_eq!(dialog.emulators().len(), 1);
        assert_eq!(dialog.emulators()[0].id, "dolphin");
        assert_eq!(dialog.emulators()[0].kind, EmulatorKind::Standalone);
        assert_eq!(dialog.emulators()[0].global_args, "-b -e {rom}");

        assert_eq!(dialog.focus(), Slot::Row(0));
        dialog.confirm();
        assert_eq!(dialog.focus(), Slot::Name);
        dialog.aim(Slot::Path);
        dialog.type_text(" --batch");
        dialog.aim(Slot::Save);
        assert_eq!(dialog.confirm(), EmulatorCommand::Write);
        assert_eq!(dialog.emulators()[0].path, "dolphin-emu --batch");
        assert_eq!(dialog.emulators()[0].id, "dolphin");

        dialog.aim(Slot::Delete(0));
        assert_eq!(dialog.confirm(), EmulatorCommand::Write);
        assert!(dialog.emulators().is_empty());
    }

    #[test]
    fn retroarch_kind_fills_an_empty_path() {
        let mut dialog = dialog();
        dialog.aim(Slot::Kind);
        dialog.move_dir(NavDir::Right);
        assert_eq!(dialog.focus(), Slot::Kind);
        let kind = dialog
            .blocks()
            .into_iter()
            .find_map(|block| match block {
                Block::Kind { kind, .. } => Some(kind),
                _ => None,
            })
            .unwrap();
        assert_eq!(kind, EmulatorKind::RetroArch);
        dialog.aim(Slot::Path);
        assert!(dialog.accepts_text());
        let path = dialog
            .blocks()
            .into_iter()
            .find_map(|block| match block {
                Block::Field {
                    slot: Slot::Path,
                    edit,
                    ..
                } => Some(edit.text),
                _ => None,
            })
            .unwrap();
        assert_eq!(path, "retroarch");
    }

    #[test]
    fn system_core_is_enabled_only_for_retroarch() {
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.emulators.push(Emulator {
            id: "dolphin".into(),
            name: "Dolphin".into(),
            kind: EmulatorKind::Standalone,
            path: "dolphin-emu".into(),
            global_args: String::new(),
        });
        config.consoles.push(snes());
        let mut dialog = Emulators::open(&config, vec![snes9x()]);
        dialog.show(Tab::Systems);
        dialog.aim(Slot::SystemEmulator(0));
        assert!(dialog.move_dir(NavDir::Right));
        assert_eq!(
            dialog.systems()[0].emulator_id.as_deref(),
            Some("retroarch")
        );
        assert!(dialog.aim(Slot::SystemCore(0)));
        assert!(dialog.move_dir(NavDir::Right));
        assert_eq!(
            dialog.systems()[0].core.as_deref(),
            Some(std::path::Path::new("/tmp/cores/snes9x_libretro.so"))
        );
        dialog.aim(Slot::SystemEmulator(0));
        assert!(!dialog.move_dir(NavDir::Down));
        assert_eq!(dialog.focus(), Slot::SystemCore(0));
        assert_eq!(
            dialog.systems()[0].emulator_id.as_deref(),
            Some("retroarch")
        );
        assert!(!dialog.move_dir(NavDir::Down));
        assert_eq!(dialog.focus(), Slot::SystemArgs(0));
        dialog.type_text("--verbose");
        assert!(dialog.move_dir(NavDir::Up));
        assert_eq!(dialog.focus(), Slot::SystemCore(0));
        assert_eq!(dialog.systems()[0].args.text, "--verbose");

        dialog.aim(Slot::SystemEmulator(0));
        dialog.move_dir(NavDir::Right);
        assert_eq!(dialog.systems()[0].emulator_id.as_deref(), Some("dolphin"));
        assert!(dialog.systems()[0].core.is_none());
        assert!(!dialog.aim(Slot::SystemCore(0)));
    }

    #[test]
    fn save_round_trips_emulators_and_leaves_the_rest_of_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let _env = crate::config::XdgEnv::sandbox(dir.path());
        let mut stored = Config::default();
        stored.theme = "launchbox".into();
        stored.cover_width = 240.0;
        stored.consoles.push(snes());
        crate::config::save_config(&stored).unwrap();

        let mut dialog = Emulators::open(&stored, vec![snes9x()]);
        dialog.emulators.push(retroarch());
        dialog.systems[0].emulator_id = Some("retroarch".into());
        dialog.systems[0].core = Some(PathBuf::from("/tmp/cores/snes9x_libretro.so"));
        dialog.systems[0].args = LineEdit::plain("--verbose".into());
        let mut loaded = crate::config::load_config().unwrap();
        apply_assignments(&mut loaded, dialog.emulators(), dialog.systems());
        crate::config::save_config(&loaded).unwrap();

        let loaded = crate::config::load_config().unwrap();
        assert_eq!(loaded.theme, "launchbox");
        assert_eq!(loaded.cover_width, 240.0);
        assert_eq!(loaded.emulators.len(), 1);
        assert_eq!(loaded.emulators[0].id, "retroarch");
        assert_eq!(loaded.consoles[0].emulator.as_deref(), Some("retroarch"));
        assert_eq!(
            loaded.consoles[0].core.as_deref(),
            Some(std::path::Path::new("/tmp/cores/snes9x_libretro.so"))
        );
        assert_eq!(loaded.consoles[0].extra_args, "--verbose");
    }
}
