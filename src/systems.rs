use crate::config::Config;
use crate::cores::Core;
use crate::game_menu::LineEdit;
use crate::gamepad::NavDir;
use crate::split::{Side, Split};
use crate::types::{Console, Emulator, EmulatorKind};
use std::path::{Path, PathBuf};

pub fn systems_key(key: &str, key_char: Option<&str>, control: bool) -> bool {
    if !control {
        return false;
    }
    key.eq_ignore_ascii_case("p") || key_char.is_some_and(|ch| ch.eq_ignore_ascii_case("p"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Emulator,
    Core,
    Args,
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
pub struct ListRow {
    pub index: usize,
    pub title: String,
    pub detail: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    pub empty: bool,
    pub name: String,
    pub emulator: String,
    pub emulator_aimed: bool,
    pub core: String,
    pub core_missing: bool,
    pub core_enabled: bool,
    pub core_aimed: bool,
    pub args: LineEdit,
    pub args_aimed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Stay,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Systems {
    emulators: Vec<Emulator>,
    rows: Vec<SystemRow>,
    cores: Vec<Core>,
    split: Split,
    field: Field,
}

impl Systems {
    pub fn open(config: &Config, cores: Vec<Core>) -> Self {
        Self {
            emulators: config.emulators.clone(),
            rows: config.consoles.iter().map(system_row).collect(),
            cores,
            split: Split::list(),
            field: Field::Emulator,
        }
    }

    pub fn rows(&self) -> &[SystemRow] {
        &self.rows
    }

    pub fn list_focused(&self) -> bool {
        self.split.side == Side::List
    }

    pub fn selected_index(&self) -> usize {
        self.split.index
    }

    pub fn list_rows(&self) -> Vec<ListRow> {
        self.rows
            .iter()
            .enumerate()
            .map(|(index, row)| ListRow {
                index,
                title: row.name.clone(),
                detail: self.emulator_label(row.emulator_id.as_deref()),
                selected: index == self.split.index,
            })
            .collect()
    }

    pub fn panel(&self) -> Panel {
        let editing = self.split.side == Side::Panel;
        let Some(row) = self.rows.get(self.split.index) else {
            return Panel {
                empty: true,
                name: String::new(),
                emulator: "None".to_string(),
                emulator_aimed: false,
                core: "No core".to_string(),
                core_missing: false,
                core_enabled: false,
                core_aimed: false,
                args: LineEdit::plain(String::new()),
                args_aimed: false,
            };
        };
        let (core, missing) = core_label(&self.cores, row.core.as_deref());
        let retroarch = self.retroarch_selected(self.split.index);
        Panel {
            empty: false,
            name: row.name.clone(),
            emulator: self.emulator_label(row.emulator_id.as_deref()),
            emulator_aimed: editing && self.field == Field::Emulator,
            core,
            core_missing: missing,
            core_enabled: retroarch,
            core_aimed: editing && self.field == Field::Core,
            args: row.args.clone(),
            args_aimed: editing && self.field == Field::Args,
        }
    }

    pub fn select(&mut self, index: usize) {
        if index < self.rows.len() {
            self.split.select(index);
            self.field = Field::Emulator;
        }
    }

    pub fn aim(&mut self, field: Field) {
        if self.rows.is_empty() {
            return;
        }
        if field == Field::Core && !self.retroarch_selected(self.split.index) {
            return;
        }
        self.split.enter();
        self.field = field;
    }

    pub fn move_dir(&mut self, dir: NavDir) -> Step {
        if self.split.side == Side::List {
            match dir {
                NavDir::Up => self.split.move_list(-1, self.rows.len()),
                NavDir::Down => self.split.move_list(1, self.rows.len()),
                NavDir::Right => self.enter_panel(),
                NavDir::Left => {}
            }
            return Step::Stay;
        }
        match dir {
            NavDir::Up => self.shift_field(-1),
            NavDir::Down => self.shift_field(1),
            NavDir::Left | NavDir::Right => return self.edit_horizontal(dir),
        }
        Step::Stay
    }

    pub fn tab(&mut self, backward: bool) {
        if self.split.side == Side::List {
            if !backward {
                self.enter_panel();
            }
            return;
        }
        if backward {
            self.split.leave();
            return;
        }
        self.shift_field(1);
    }

    pub fn confirm(&mut self) -> Step {
        if self.split.side == Side::List {
            self.enter_panel();
            return Step::Stay;
        }
        match self.field {
            Field::Emulator => {
                self.cycle_emulator(1);
                Step::Write
            }
            Field::Core => {
                self.cycle_core(1);
                Step::Write
            }
            Field::Args => Step::Write,
        }
    }

    pub fn accepts_text(&self) -> bool {
        self.split.side == Side::Panel && self.field == Field::Args
    }

    pub fn type_text(&mut self, text: &str) {
        if let Some(edit) = self.args_mut() {
            edit.insert(text);
        }
    }

    pub fn backspace(&mut self) {
        if let Some(edit) = self.args_mut() {
            edit.backspace();
        }
    }

    pub fn delete_forward(&mut self) {
        if let Some(edit) = self.args_mut() {
            edit.delete_forward();
        }
    }

    pub fn set_cores(&mut self, cores: Vec<Core>) {
        self.cores = cores;
    }

    fn enter_panel(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.split.enter();
        self.field = Field::Emulator;
    }

    fn shift_field(&mut self, delta: isize) {
        let fields = self.fields();
        let pos = fields
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        let next = pos as isize + delta;
        if (0..fields.len() as isize).contains(&next) {
            self.field = fields[next as usize];
        }
    }

    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Emulator];
        if self.retroarch_selected(self.split.index) {
            fields.push(Field::Core);
        }
        fields.push(Field::Args);
        fields
    }

    fn edit_horizontal(&mut self, dir: NavDir) -> Step {
        let delta = if dir == NavDir::Left { -1 } else { 1 };
        match self.field {
            Field::Emulator => {
                self.cycle_emulator(delta);
                Step::Write
            }
            Field::Core => {
                self.cycle_core(delta);
                Step::Write
            }
            Field::Args => {
                let Some(edit) = self.args_mut() else {
                    return Step::Stay;
                };
                if dir == NavDir::Left && !edit.replace && edit.caret == 0 {
                    self.split.leave();
                    return Step::Stay;
                }
                edit.move_caret(delta);
                Step::Stay
            }
        }
    }

    fn args_mut(&mut self) -> Option<&mut LineEdit> {
        let index = self.split.index;
        self.rows.get_mut(index).map(|row| &mut row.args)
    }

    fn cycle_emulator(&mut self, delta: isize) {
        let ids: Vec<Option<String>> = std::iter::once(None)
            .chain(
                self.emulators
                    .iter()
                    .map(|emulator| Some(emulator.id.clone())),
            )
            .collect();
        let index = self.split.index;
        let Some(current) = self.rows.get(index).map(|row| row.emulator_id.clone()) else {
            return;
        };
        let pos = ids.iter().position(|id| *id == current).unwrap_or(0);
        let chosen = ids[wrap(pos, ids.len(), delta)].clone();
        let retroarch = chosen.as_ref().is_some_and(|id| {
            self.emulators
                .iter()
                .any(|emulator| emulator.id == *id && emulator.kind == EmulatorKind::RetroArch)
        });
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        row.emulator_id = chosen;
        if !retroarch {
            row.core = None;
            if self.field == Field::Core {
                self.field = Field::Emulator;
            }
        }
    }

    fn cycle_core(&mut self, delta: isize) {
        if !self.retroarch_selected(self.split.index) {
            return;
        }
        let choices = self.core_choices();
        let index = self.split.index;
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        let pos = choices
            .iter()
            .position(|path| *path == row.core)
            .unwrap_or(0);
        row.core = choices[wrap(pos, choices.len(), delta)].clone();
    }

    fn core_choices(&self) -> Vec<Option<PathBuf>> {
        let mut choices = vec![None];
        if let Some(path) = self
            .rows
            .get(self.split.index)
            .and_then(|row| row.core.clone())
        {
            if !self.cores.iter().any(|core| core.path == path) {
                choices.push(Some(path));
            }
        }
        for core in &self.cores {
            if !choices.iter().any(|path| path.as_ref() == Some(&core.path)) {
                choices.push(Some(core.path.clone()));
            }
        }
        choices
    }

    fn retroarch_selected(&self, index: usize) -> bool {
        let Some(id) = self.rows.get(index).and_then(|row| row.emulator_id.clone()) else {
            return false;
        };
        self.emulators
            .iter()
            .any(|emulator| emulator.id == id && emulator.kind == EmulatorKind::RetroArch)
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
}

pub fn apply_systems(config: &mut Config, rows: &[SystemRow]) {
    for console in &mut config.consoles {
        let Some(row) = rows.iter().find(|row| row.id == console.id) else {
            continue;
        };
        let retroarch = row.emulator_id.as_ref().is_some_and(|id| {
            config
                .emulators
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

fn core_label(cores: &[Core], path: Option<&Path>) -> (String, bool) {
    let Some(path) = path else {
        return ("No core".to_string(), false);
    };
    if let Some(core) = cores.iter().find(|core| core.path == path) {
        return (core.label.clone(), false);
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("core");
    (format!("Missing: {name}"), true)
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
    use crate::types::{GridArt, MediaToggles};

    fn snes() -> Console {
        Console {
            id: "snes".into(),
            name: "Super Nintendo".into(),
            rom_dirs: vec![PathBuf::from("/tmp/roms/snes")],
            extensions: vec!["sfc".into()],
            emulator: Some("retroarch".into()),
            core: Some(PathBuf::from("/old/snes9x_libretro.so")),
            extra_args: "--region ntsc".into(),
            grid_art: GridArt::BoxArt,
            media: MediaToggles::default(),
        }
    }

    fn retroarch() -> Emulator {
        Emulator {
            id: "retroarch".into(),
            name: "RetroArch".into(),
            kind: EmulatorKind::RetroArch,
            path: "/usr/bin/retroarch".into(),
            global_args: String::new(),
        }
    }

    fn core() -> Core {
        Core {
            name: "snes9x".into(),
            path: PathBuf::from("/cores/snes9x_libretro.so"),
            label: "Nintendo - SNES / SFC (Snes9x)".into(),
            systemname: "Super Nintendo Entertainment System".into(),
            extensions: vec!["sfc".into()],
        }
    }

    fn screen() -> Systems {
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.consoles.push(snes());
        Systems::open(&config, vec![core()])
    }

    #[test]
    fn ctrl_p_opens_systems() {
        assert!(systems_key("p", None, true));
        assert!(systems_key("P", Some("P"), true));
        assert!(!systems_key("p", None, false));
        assert!(!systems_key("e", None, true));
    }

    #[test]
    fn up_and_down_move_the_list_and_right_enters_the_panel() {
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.consoles.push(snes());
        let mut nes = snes();
        nes.id = "nes".into();
        nes.name = "NES".into();
        config.consoles.push(nes);
        let mut screen = Systems::open(&config, vec![core()]);
        assert!(screen.list_focused());
        screen.move_dir(NavDir::Down);
        assert_eq!(screen.list_rows()[1].selected, true);
        screen.move_dir(NavDir::Right);
        assert!(!screen.list_focused());
        assert!(screen.panel().emulator_aimed);
        screen.tab(true);
        assert!(screen.list_focused());
        assert!(screen.list_rows()[1].selected);
    }

    #[test]
    fn a_missing_core_stays_and_a_retroarch_core_can_be_chosen() {
        let mut screen = screen();
        let panel = screen.panel();
        assert!(panel.core_missing);
        assert!(panel.core.starts_with("Missing:"));
        assert!(panel.core_enabled);
        screen.move_dir(NavDir::Right);
        assert_eq!(screen.move_dir(NavDir::Down), Step::Stay);
        assert!(screen.panel().core_aimed);
        assert_eq!(screen.move_dir(NavDir::Right), Step::Write);
        assert_eq!(
            screen.rows()[0].core.as_deref(),
            Some(Path::new("/cores/snes9x_libretro.so"))
        );
        assert!(!screen.panel().core_missing);
        assert_eq!(screen.panel().core, "Nintendo - SNES / SFC (Snes9x)");
    }

    #[test]
    fn a_standalone_emulator_clears_the_core() {
        let mut screen = screen();
        screen.emulators.push(Emulator {
            id: "dolphin".into(),
            name: "Dolphin".into(),
            kind: EmulatorKind::Standalone,
            path: "dolphin-emu".into(),
            global_args: String::new(),
        });
        screen.move_dir(NavDir::Right);
        assert_eq!(screen.move_dir(NavDir::Right), Step::Write);
        assert_eq!(screen.rows()[0].emulator_id.as_deref(), Some("dolphin"));
        assert!(screen.rows()[0].core.is_none());
        assert!(!screen.panel().core_enabled);
    }

    #[test]
    fn assignments_round_trip_and_leave_other_console_fields() {
        let mut screen = screen();
        screen.move_dir(NavDir::Right);
        screen.move_dir(NavDir::Down);
        screen.move_dir(NavDir::Right);
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.consoles.push(snes());
        config.consoles[0].rom_dirs = vec![PathBuf::from("/keep")];
        apply_systems(&mut config, screen.rows());
        assert_eq!(
            config.consoles[0].core.as_deref(),
            Some(Path::new("/cores/snes9x_libretro.so"))
        );
        assert_eq!(config.consoles[0].extra_args, "--region ntsc");
        assert_eq!(config.consoles[0].rom_dirs, vec![PathBuf::from("/keep")]);
    }
}
