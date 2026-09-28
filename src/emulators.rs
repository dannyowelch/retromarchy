use crate::config::Config;
use crate::cores::{Core, CoreCatalog};
use crate::game_menu::LineEdit;
use crate::gamepad::NavDir;
use crate::split::{Side, Split};
use crate::types::{Emulator, EmulatorKind};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Kind,
    Path,
    Args,
    Config,
    Rescan,
    Delete,
    Save,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Stay,
    Write,
    Rescan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub index: usize,
    pub title: String,
    pub detail: String,
    pub selected: bool,
    pub add: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLine {
    pub label: String,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    pub adding: bool,
    pub name: LineEdit,
    pub name_aimed: bool,
    pub kind: EmulatorKind,
    pub kind_aimed: bool,
    pub path: LineEdit,
    pub path_aimed: bool,
    pub args: LineEdit,
    pub args_aimed: bool,
    pub show_cores: bool,
    pub cfg: LineEdit,
    pub cfg_aimed: bool,
    pub cores_dir: String,
    pub cores_note: Option<String>,
    pub cores: Vec<CoreLine>,
    pub missing: bool,
    pub rescan_aimed: bool,
    pub show_delete: bool,
    pub delete_aimed: bool,
    pub save_label: &'static str,
    pub save_aimed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Draft {
    name: LineEdit,
    kind: EmulatorKind,
    path: LineEdit,
    args: LineEdit,
    config: LineEdit,
}

impl Draft {
    fn blank() -> Self {
        Self {
            name: LineEdit::plain(String::new()),
            kind: EmulatorKind::Standalone,
            path: LineEdit::plain(String::new()),
            args: LineEdit::plain(String::new()),
            config: LineEdit::plain(String::new()),
        }
    }

    fn from_emulator(emulator: &Emulator, cfg: &PathBuf) -> Self {
        let config = emulator
            .config
            .clone()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| cfg.clone());
        let config = if config.as_os_str().is_empty() {
            String::new()
        } else {
            config.display().to_string()
        };
        Self {
            name: LineEdit::plain(emulator.name.clone()),
            kind: emulator.kind,
            path: LineEdit::plain(emulator.path.clone()),
            args: LineEdit::plain(emulator.global_args.clone()),
            config: LineEdit::plain(config),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emulators {
    emulators: Vec<Emulator>,
    catalogs: Vec<CoreCatalog>,
    missing: Vec<bool>,
    draft_catalog: CoreCatalog,
    split: Split,
    draft: Draft,
    field: Field,
    error: Option<String>,
}

impl Emulators {
    pub fn open(config: &Config, catalogs: Vec<CoreCatalog>, missing: Vec<bool>) -> Self {
        let index = config
            .emulators
            .iter()
            .position(|emulator| emulator.kind == EmulatorKind::RetroArch)
            .unwrap_or(0);
        let mut catalogs = catalogs;
        catalogs.resize(config.emulators.len(), CoreCatalog::empty());
        let mut missing = missing;
        missing.resize(config.emulators.len(), false);
        let mut screen = Self {
            emulators: config.emulators.clone(),
            catalogs,
            missing,
            draft_catalog: CoreCatalog::empty(),
            split: Split::at(index.min(config.emulators.len())),
            draft: Draft::blank(),
            field: Field::Name,
            error: None,
        };
        screen.load_draft();
        screen
    }

    pub fn emulators(&self) -> &[Emulator] {
        &self.emulators
    }

    pub fn list_focused(&self) -> bool {
        self.split.side == Side::List
    }

    pub fn selected_index(&self) -> usize {
        self.split.index
    }

    pub fn list_rows(&self) -> Vec<ListRow> {
        let mut rows: Vec<_> = self
            .emulators
            .iter()
            .enumerate()
            .map(|(index, emulator)| ListRow {
                index,
                title: emulator.name.clone(),
                detail: emulator_detail(
                    emulator,
                    self.missing.get(index).copied().unwrap_or(false),
                ),
                selected: index == self.split.index,
                add: false,
            })
            .collect();
        rows.push(ListRow {
            index: self.emulators.len(),
            title: "Add emulator".to_string(),
            detail: String::new(),
            selected: self.split.index == self.emulators.len(),
            add: true,
        });
        rows
    }

    pub fn panel(&self) -> Panel {
        let editing = self.split.side == Side::Panel;
        let adding = self.adding();
        let show_cores = self.draft.kind == EmulatorKind::RetroArch;
        let cores_dir = self.active_catalog().directory.display().to_string();
        let cores_note = self.active_catalog().note.clone();
        let cores = if show_cores {
            core_lines(&self.active_catalog().cores)
        } else {
            Vec::new()
        };
        Panel {
            adding,
            name: self.draft.name.clone(),
            name_aimed: editing && self.field == Field::Name,
            kind: self.draft.kind,
            kind_aimed: editing && self.field == Field::Kind,
            path: self.draft.path.clone(),
            path_aimed: editing && self.field == Field::Path,
            args: self.draft.args.clone(),
            args_aimed: editing && self.field == Field::Args,
            show_cores,
            cfg: self.draft.config.clone(),
            cfg_aimed: editing && self.field == Field::Config,
            cores_dir,
            cores_note,
            cores,
            missing: self.missing.get(self.split.index).copied().unwrap_or(false),
            rescan_aimed: editing && self.field == Field::Rescan,
            show_delete: !adding,
            delete_aimed: editing && self.field == Field::Delete,
            save_label: if adding { "Add" } else { "Save" },
            save_aimed: editing && self.field == Field::Save,
            error: self.error.clone(),
        }
    }

    pub fn set_catalog(&mut self, catalog: CoreCatalog) {
        if self.adding() {
            self.draft_catalog = catalog;
            return;
        }
        let index = self.split.index;
        if let Some(slot) = self.catalogs.get_mut(index) {
            *slot = catalog;
        }
    }

    pub fn selected_emulator(&self) -> Option<&Emulator> {
        self.emulators.get(self.split.index)
    }

    /// The RetroArch row a rescan should read. An unsaved row uses the draft command.
    pub fn retroarch_target(&self) -> Option<Emulator> {
        if self.draft.kind != EmulatorKind::RetroArch {
            return None;
        }
        if let Some(emulator) = self.selected_emulator() {
            return Some(emulator.clone());
        }
        let path = self.draft.path.text.trim();
        if path.is_empty() {
            return None;
        }
        Some(Emulator {
            id: String::new(),
            name: self.draft.name.text.trim().to_string(),
            kind: EmulatorKind::RetroArch,
            path: path.to_string(),
            global_args: self.draft.args.text.trim().to_string(),
            config: config_path(&self.draft.config.text),
        })
    }

    pub fn select(&mut self, index: usize) {
        self.store_draft();
        let len = self.emulators.len() + 1;
        if index < len {
            self.split.select(index);
            self.field = Field::Name;
            self.load_draft();
        }
    }

    pub fn aim(&mut self, field: Field) {
        if !self.fields().contains(&field) {
            return;
        }
        self.split.enter();
        self.field = field;
    }

    pub fn set_kind(&mut self, kind: EmulatorKind) {
        self.split.enter();
        self.field = Field::Kind;
        self.draft.kind = kind;
        if kind == EmulatorKind::RetroArch && self.draft.path.text.trim().is_empty() {
            self.draft.path = LineEdit::plain("retroarch".to_string());
        }
        if !self.fields().contains(&self.field) {
            self.field = Field::Kind;
        }
    }

    pub fn move_dir(&mut self, dir: NavDir) -> Step {
        if self.split.side == Side::List {
            let len = self.emulators.len() + 1;
            match dir {
                NavDir::Up => {
                    self.store_draft();
                    self.split.move_list(-1, len);
                    self.load_draft();
                }
                NavDir::Down => {
                    self.store_draft();
                    self.split.move_list(1, len);
                    self.load_draft();
                }
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
            Field::Kind => {
                self.cycle_kind(1);
                Step::Stay
            }
            Field::Rescan => Step::Rescan,
            Field::Delete => {
                self.delete_selected();
                Step::Write
            }
            Field::Save => self.save(),
            Field::Name | Field::Path | Field::Args | Field::Config => {
                self.shift_field(1);
                Step::Stay
            }
        }
    }

    pub fn rescan_key(&self, key: &str, key_char: Option<&str>) -> bool {
        if self.accepts_text() || self.draft.kind != EmulatorKind::RetroArch {
            return false;
        }
        key == "r" || key_char == Some("r")
    }

    pub fn accepts_text(&self) -> bool {
        self.split.side == Side::Panel
            && matches!(
                self.field,
                Field::Name | Field::Path | Field::Args | Field::Config
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

    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
    }

    pub fn commit(&mut self) -> bool {
        if self.adding() {
            let name = self.draft.name.text.trim();
            let path = self.draft.path.text.trim();
            if name.is_empty() || path.is_empty() {
                return false;
            }
            return self.save() == Step::Write;
        }
        let before = self.emulators.clone();
        self.store_draft();
        self.emulators != before
    }

    fn enter_panel(&mut self) {
        self.split.enter();
        self.field = Field::Name;
    }

    fn adding(&self) -> bool {
        self.split.index >= self.emulators.len()
    }

    fn load_draft(&mut self) {
        self.draft = if let Some(emulator) = self.emulators.get(self.split.index) {
            let cfg = self
                .catalogs
                .get(self.split.index)
                .map(|catalog| catalog.cfg.clone())
                .unwrap_or_default();
            Draft::from_emulator(emulator, &cfg)
        } else {
            Draft::blank()
        };
        self.error = None;
    }

    fn store_draft(&mut self) {
        if self.adding() {
            return;
        }
        let Some(index) = self
            .emulators
            .get(self.split.index)
            .map(|_| self.split.index)
        else {
            return;
        };
        if self.apply_draft(index).is_none() {
            self.load_draft();
        }
    }

    fn save(&mut self) -> Step {
        let name = self.draft.name.text.trim().to_string();
        let path = self.draft.path.text.trim().to_string();
        if name.is_empty() || path.is_empty() {
            self.error = Some("Enter a name and an executable path.".to_string());
            return Step::Stay;
        }
        if self.adding() {
            let id = fresh_id(&slug(&name), &self.emulators);
            self.emulators.push(Emulator {
                id,
                name,
                kind: self.draft.kind,
                path,
                global_args: self.draft.args.text.trim().to_string(),
                config: config_path(&self.draft.config.text),
            });
            self.catalogs.push(self.draft_catalog.clone());
            self.missing.push(false);
            self.split.index = self.emulators.len() - 1;
            self.load_draft();
            self.field = Field::Name;
            self.split.leave();
            Step::Write
        } else if self.apply_draft(self.split.index).is_some() {
            self.error = None;
            self.split.leave();
            Step::Write
        } else {
            Step::Stay
        }
    }

    fn apply_draft(&mut self, index: usize) -> Option<()> {
        let name = self.draft.name.text.trim().to_string();
        let path = self.draft.path.text.trim().to_string();
        if name.is_empty() || path.is_empty() {
            return None;
        }
        let emulator = self.emulators.get_mut(index)?;
        emulator.name = name;
        emulator.kind = self.draft.kind;
        emulator.path = path;
        emulator.global_args = self.draft.args.text.trim().to_string();
        emulator.config = config_path(&self.draft.config.text);
        Some(())
    }

    fn delete_selected(&mut self) {
        if self.adding() {
            return;
        }
        let index = self.split.index;
        self.emulators.remove(index);
        if index < self.catalogs.len() {
            self.catalogs.remove(index);
        }
        if index < self.missing.len() {
            self.missing.remove(index);
        }
        if self.split.index > self.emulators.len() {
            self.split.index = self.emulators.len();
        }
        self.load_draft();
        self.field = Field::Name;
        self.split.leave();
    }

    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Name, Field::Kind, Field::Path, Field::Args];
        if self.draft.kind == EmulatorKind::RetroArch {
            fields.push(Field::Config);
            fields.push(Field::Rescan);
        }
        if !self.adding() {
            fields.push(Field::Delete);
        }
        fields.push(Field::Save);
        fields
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

    fn edit_horizontal(&mut self, dir: NavDir) -> Step {
        let delta = if dir == NavDir::Left { -1 } else { 1 };
        match self.field {
            Field::Kind => {
                self.cycle_kind(delta);
                Step::Stay
            }
            Field::Name | Field::Path | Field::Args | Field::Config => {
                let Some(edit) = self.edit_mut() else {
                    return Step::Stay;
                };
                if dir == NavDir::Left && !edit.replace && edit.caret == 0 {
                    self.split.leave();
                    return Step::Stay;
                }
                edit.move_caret(delta);
                Step::Stay
            }
            Field::Rescan | Field::Delete | Field::Save => {
                if dir == NavDir::Left {
                    self.split.leave();
                }
                Step::Stay
            }
        }
    }

    fn cycle_kind(&mut self, delta: isize) {
        let kinds = [EmulatorKind::Standalone, EmulatorKind::RetroArch];
        let pos = kinds
            .iter()
            .position(|kind| *kind == self.draft.kind)
            .unwrap_or(0);
        self.set_kind(kinds[wrap(pos, kinds.len(), delta)]);
        self.field = Field::Kind;
    }

    fn edit_mut(&mut self) -> Option<&mut LineEdit> {
        match self.field {
            Field::Name => Some(&mut self.draft.name),
            Field::Path => Some(&mut self.draft.path),
            Field::Args => Some(&mut self.draft.args),
            Field::Config => Some(&mut self.draft.config),
            _ => None,
        }
    }

    fn active_catalog(&self) -> &CoreCatalog {
        if !self.adding() {
            if let Some(catalog) = self.catalogs.get(self.split.index) {
                return catalog;
            }
        }
        &self.draft_catalog
    }
}

fn core_lines(cores: &[Core]) -> Vec<CoreLine> {
    cores
        .iter()
        .map(|core| CoreLine {
            label: core.label.clone(),
            file_name: core
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_string(),
        })
        .collect()
}

fn config_path(text: &str) -> Option<PathBuf> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

pub fn apply_emulators(config: &mut Config, emulators: &[Emulator]) {
    for console in &mut config.consoles {
        let Some(id) = console.emulator.clone() else {
            continue;
        };
        match emulators.iter().find(|emulator| emulator.id == id) {
            None => {
                console.emulator = None;
                console.core = None;
            }
            Some(emulator) if emulator.kind != EmulatorKind::RetroArch => {
                console.core = None;
            }
            Some(_) => {}
        }
    }
    config.emulators = emulators.to_vec();
}

fn emulator_detail(emulator: &Emulator, missing: bool) -> String {
    let mut detail = if emulator.global_args.trim().is_empty() {
        format!("{} · {}", emulator.kind.label(), emulator.path)
    } else {
        format!(
            "{} · {} · {}",
            emulator.kind.label(),
            emulator.path,
            emulator.global_args.trim()
        )
    };
    if missing {
        detail.push_str(" · not found");
    }
    detail
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
    use crate::types::EmulatorKind;

    fn retroarch() -> Emulator {
        Emulator {
            id: "retroarch".into(),
            name: "RetroArch".into(),
            kind: EmulatorKind::RetroArch,
            path: "/usr/bin/retroarch".into(),
            global_args: String::new(),
            config: None,
        }
    }

    fn core() -> Core {
        Core {
            name: "snes9x".into(),
            path: std::path::PathBuf::from("/cores/snes9x_libretro.so"),
            label: "Nintendo - SNES / SFC (Snes9x)".into(),
            systemname: "Super Nintendo".into(),
            extensions: vec!["sfc".into()],
        }
    }

    fn catalog(core: Core) -> CoreCatalog {
        CoreCatalog {
            directory: std::path::PathBuf::from("/cores"),
            cores: vec![core],
            cfg: std::path::PathBuf::from("/cfg/retroarch.cfg"),
            note: None,
        }
    }

    fn screen() -> Emulators {
        let mut config = Config::default();
        config.emulators.push(retroarch());
        Emulators::open(&config, vec![catalog(core())], vec![false])
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
    fn the_list_selects_retroarch_and_the_panel_shows_its_cores() {
        let screen = screen();
        assert!(screen.list_focused());
        let rows = screen.list_rows();
        assert!(rows[0].selected);
        assert_eq!(rows[0].title, "RetroArch");
        assert!(rows[1].add);
        let panel = screen.panel();
        assert!(panel.show_cores);
        assert_eq!(panel.cores.len(), 1);
        assert_eq!(panel.cores[0].label, "Nintendo - SNES / SFC (Snes9x)");
        assert_eq!(panel.cores[0].file_name, "snes9x_libretro.so");
        assert!(panel.show_delete);
    }

    #[test]
    fn right_enters_the_panel_and_shift_tab_returns() {
        let mut screen = screen();
        screen.move_dir(NavDir::Right);
        assert!(!screen.list_focused());
        assert!(screen.panel().name_aimed);
        screen.tab(true);
        assert!(screen.list_focused());
    }

    #[test]
    fn add_edit_and_delete_an_emulator() {
        let mut screen = Emulators::open(&Config::default(), Vec::new(), Vec::new());
        assert!(screen.list_rows()[0].add);
        screen.move_dir(NavDir::Right);
        screen.type_text("Dolphin");
        screen.move_dir(NavDir::Down);
        screen.move_dir(NavDir::Down);
        screen.type_text("dolphin-emu");
        assert_eq!(screen.confirm(), Step::Stay);
        screen.move_dir(NavDir::Down);
        assert_eq!(screen.confirm(), Step::Write);
        assert_eq!(screen.emulators().len(), 1);
        assert_eq!(screen.emulators()[0].id, "dolphin");
        assert_eq!(screen.emulators()[0].path, "dolphin-emu");
        assert!(screen.list_focused());

        screen.move_dir(NavDir::Right);
        screen.type_text("!");
        screen.aim(Field::Save);
        assert_eq!(screen.confirm(), Step::Write);
        assert_eq!(screen.emulators()[0].name, "Dolphin!");

        screen.move_dir(NavDir::Right);
        screen.aim(Field::Delete);
        assert_eq!(screen.confirm(), Step::Write);
        assert!(screen.emulators().is_empty());
    }

    #[test]
    fn retroarch_kind_fills_an_empty_path() {
        let mut screen = Emulators::open(&Config::default(), vec![catalog(core())], Vec::new());
        screen.move_dir(NavDir::Right);
        screen.set_kind(EmulatorKind::RetroArch);
        assert_eq!(screen.panel().path.text, "retroarch");
        assert!(screen.panel().show_cores);
    }

    #[test]
    fn r_rescans_cores_for_retroarch_and_not_while_typing() {
        let mut screen = screen();
        assert!(screen.rescan_key("r", None));
        screen.move_dir(NavDir::Right);
        assert!(!screen.rescan_key("r", Some("r")));
        screen.aim(Field::Rescan);
        assert_eq!(screen.confirm(), Step::Rescan);
    }

    #[test]
    fn deleting_an_emulator_clears_its_system_assignment() {
        use crate::types::{Console, GridArt, MediaToggles};
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.consoles.push(Console {
            id: "snes".into(),
            name: "Super Nintendo".into(),
            rom_dirs: vec![std::path::PathBuf::from("/roms")],
            extensions: vec!["sfc".into()],
            emulator: Some("retroarch".into()),
            core: Some(std::path::PathBuf::from("/cores/snes9x_libretro.so")),
            extra_args: "--a".into(),
            grid_art: GridArt::BoxArt,
            media: MediaToggles::default(),
        });
        apply_emulators(&mut config, &[]);
        assert!(config.emulators.is_empty());
        assert!(config.consoles[0].emulator.is_none());
        assert!(config.consoles[0].core.is_none());
        assert_eq!(config.consoles[0].extra_args, "--a");
        assert_eq!(config.consoles[0].rom_dirs.len(), 1);
    }
}
