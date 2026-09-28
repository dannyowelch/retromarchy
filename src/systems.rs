use crate::config::Config;
use crate::cores::{Core, CoreCatalog};
use crate::game_menu::LineEdit;
use crate::gamepad::NavDir;
use crate::picker::{Item, Jump, Menu, Picker};
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
    pub extensions: Vec<String>,
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
    pub emulator_menu: Option<Menu>,
    pub core_menu: Option<Menu>,
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
    catalogs: Vec<CoreCatalog>,
    split: Split,
    field: Field,
    picker: Picker,
}

impl Systems {
    pub fn open(config: &Config, catalogs: Vec<CoreCatalog>) -> Self {
        let mut catalogs = catalogs;
        catalogs.resize(config.emulators.len(), CoreCatalog::empty());
        Self {
            emulators: config.emulators.clone(),
            rows: config.consoles.iter().map(system_row).collect(),
            catalogs,
            split: Split::list(),
            field: Field::Emulator,
            picker: Picker::closed(),
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
                emulator_menu: None,
                core_menu: None,
            };
        };
        let row = row.clone();
        let name = row.name.clone();
        let emulator = self.emulator_label(row.emulator_id.as_deref());
        let (core, missing) = core_label(self.cores_for(self.split.index), row.core.as_deref());
        let args = row.args.clone();
        let retroarch = self.retroarch_selected(self.split.index);
        let (emulator_menu, core_menu) = self.open_menus();
        Panel {
            empty: false,
            name,
            emulator,
            emulator_aimed: editing && self.field == Field::Emulator,
            core,
            core_missing: missing,
            core_enabled: retroarch,
            core_aimed: editing && self.field == Field::Core,
            args,
            args_aimed: editing && self.field == Field::Args,
            emulator_menu,
            core_menu,
        }
    }

    pub fn select(&mut self, index: usize) {
        if index < self.rows.len() {
            self.picker.close();
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
        self.picker.close();
        self.split.enter();
        self.field = field;
    }

    pub fn click_field(&mut self, field: Field) {
        if self.rows.is_empty() {
            return;
        }
        if field == Field::Core && !self.retroarch_selected(self.split.index) {
            return;
        }
        let open_here =
            self.picker.is_open() && self.split.side == Side::Panel && self.field == field;
        if open_here {
            self.picker.close();
            return;
        }
        self.split.enter();
        self.field = field;
        self.open_current();
    }

    pub fn choose(&mut self, index: usize) -> Step {
        if !self.picker.is_open() {
            return Step::Stay;
        }
        self.picker.close();
        match self.field {
            Field::Emulator => self.set_emulator(index),
            Field::Core => self.set_core(index),
            Field::Args => Step::Stay,
        }
    }

    pub fn dismiss(&mut self) -> bool {
        if !self.picker.is_open() {
            return false;
        }
        self.picker.close();
        true
    }

    pub fn jump(&mut self, jump: Jump) -> bool {
        if !self.picker.is_open() {
            return false;
        }
        self.picker.jump(jump, self.choice_len());
        true
    }

    pub fn open_mark(&self) -> Option<(Field, usize)> {
        if self.split.side != Side::Panel {
            return None;
        }
        let cursor = self.picker.cursor()?;
        let len = self.choice_len();
        if len == 0 {
            return None;
        }
        Some((self.field, cursor.min(len - 1)))
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
        if self.picker.is_open() {
            let len = self.choice_len();
            match dir {
                NavDir::Up => self.picker.move_by(-1, len),
                NavDir::Down => self.picker.move_by(1, len),
                NavDir::Left => self.picker.close(),
                NavDir::Right => {}
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
        self.picker.close();
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
            Field::Emulator | Field::Core => {
                if self.picker.is_open() {
                    let index = self.picker.cursor().unwrap_or(0);
                    self.choose(index)
                } else {
                    self.open_current();
                    Step::Stay
                }
            }
            Field::Args => Step::Write,
        }
    }

    pub fn accepts_text(&self) -> bool {
        self.split.side == Side::Panel && self.field == Field::Args
    }

    pub fn type_text(&mut self, text: &str) {
        if self.picker.is_open() {
            let labels = self.choice_labels();
            self.picker.type_ahead(text, &labels);
            return;
        }
        if self.accepts_text() {
            if let Some(edit) = self.args_mut() {
                edit.insert(text);
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.picker.is_open() || !self.accepts_text() {
            return;
        }
        if let Some(edit) = self.args_mut() {
            edit.backspace();
        }
    }

    pub fn delete_forward(&mut self) {
        if self.picker.is_open() || !self.accepts_text() {
            return;
        }
        if let Some(edit) = self.args_mut() {
            edit.delete_forward();
        }
    }

    pub fn set_catalogs(&mut self, catalogs: Vec<CoreCatalog>) {
        self.catalogs = catalogs;
    }

    fn enter_panel(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.picker.close();
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
        match self.field {
            Field::Emulator | Field::Core => {
                if dir == NavDir::Left {
                    self.split.leave();
                } else {
                    self.open_current();
                }
                Step::Stay
            }
            Field::Args => {
                let Some(edit) = self.args_mut() else {
                    return Step::Stay;
                };
                if dir == NavDir::Left && !edit.replace && edit.caret == 0 {
                    self.split.leave();
                    return Step::Stay;
                }
                let delta = if dir == NavDir::Left { -1 } else { 1 };
                edit.move_caret(delta);
                Step::Stay
            }
        }
    }

    fn args_mut(&mut self) -> Option<&mut LineEdit> {
        let index = self.split.index;
        self.rows.get_mut(index).map(|row| &mut row.args)
    }

    fn open_current(&mut self) {
        let index = self.current_index();
        self.picker.open_at(index);
    }

    fn current_index(&self) -> usize {
        match self.field {
            Field::Emulator => {
                let current = self
                    .rows
                    .get(self.split.index)
                    .and_then(|row| row.emulator_id.clone());
                self.emulator_choices()
                    .iter()
                    .position(|choice| choice.id == current)
                    .unwrap_or(0)
            }
            Field::Core => {
                let current = self
                    .rows
                    .get(self.split.index)
                    .and_then(|row| row.core.clone());
                self.core_choices()
                    .iter()
                    .position(|choice| choice.path == current)
                    .unwrap_or(0)
            }
            Field::Args => 0,
        }
    }

    fn set_emulator(&mut self, index: usize) -> Step {
        let choices = self.emulator_choices();
        let Some(chosen) = choices.get(index) else {
            return Step::Stay;
        };
        let id = chosen.id.clone();
        let retroarch = id.as_ref().is_some_and(|id| {
            self.emulators
                .iter()
                .any(|emulator| emulator.id == *id && emulator.kind == EmulatorKind::RetroArch)
        });
        let Some(row) = self.rows.get_mut(self.split.index) else {
            return Step::Stay;
        };
        row.emulator_id = id;
        if !retroarch {
            row.core = None;
            if self.field == Field::Core {
                self.field = Field::Emulator;
            }
        }
        Step::Write
    }

    fn set_core(&mut self, index: usize) -> Step {
        if !self.retroarch_selected(self.split.index) {
            return Step::Stay;
        }
        let choices = self.core_choices();
        let Some(chosen) = choices.get(index) else {
            return Step::Stay;
        };
        let path = chosen.path.clone();
        let Some(row) = self.rows.get_mut(self.split.index) else {
            return Step::Stay;
        };
        row.core = path;
        Step::Write
    }

    fn choice_len(&self) -> usize {
        match self.field {
            Field::Emulator => self.emulator_choices().len(),
            Field::Core => self.core_choices().len(),
            Field::Args => 0,
        }
    }

    fn choice_labels(&self) -> Vec<String> {
        match self.field {
            Field::Emulator => self
                .emulator_choices()
                .into_iter()
                .map(|choice| choice.label)
                .collect(),
            Field::Core => self
                .core_choices()
                .into_iter()
                .map(|choice| choice.label)
                .collect(),
            Field::Args => Vec::new(),
        }
    }

    fn open_menus(&self) -> (Option<Menu>, Option<Menu>) {
        let Some(cursor) = self.picker.cursor() else {
            return (None, None);
        };
        if self.split.side != Side::Panel {
            return (None, None);
        }
        match self.field {
            Field::Emulator => (
                Some(menu_from(
                    self.emulator_choices()
                        .into_iter()
                        .map(|choice| Item {
                            label: choice.label,
                            detail: String::new(),
                        })
                        .collect(),
                    cursor,
                )),
                None,
            ),
            Field::Core => (
                None,
                Some(menu_from(
                    self.core_choices()
                        .into_iter()
                        .map(|choice| Item {
                            label: choice.label,
                            detail: choice.detail,
                        })
                        .collect(),
                    cursor,
                )),
            ),
            Field::Args => (None, None),
        }
    }

    fn emulator_choices(&self) -> Vec<EmulatorChoice> {
        std::iter::once(EmulatorChoice {
            id: None,
            label: "None".to_string(),
        })
        .chain(self.emulators.iter().map(|emulator| EmulatorChoice {
            id: Some(emulator.id.clone()),
            label: emulator.name.clone(),
        }))
        .collect()
    }

    fn core_choices(&self) -> Vec<CoreChoice> {
        let Some(row) = self.rows.get(self.split.index) else {
            return vec![none_core()];
        };
        let name = row.name.clone();
        let id = row.id.clone();
        let extensions = row.extensions.clone();
        let current = row.core.clone();
        let cores = self.cores_for(self.split.index);
        let mut choices = vec![none_core()];
        if let Some(path) = current {
            if !cores.iter().any(|core| core.path == path) {
                let file = file_name(&path);
                choices.push(CoreChoice {
                    path: Some(path),
                    label: format!("Missing: {file}"),
                    detail: String::new(),
                });
            }
        }
        let mut ranked: Vec<&Core> = cores.iter().collect();
        ranked.sort_by(|left, right| {
            core_rank(left, &name, &id, &extensions)
                .cmp(&core_rank(right, &name, &id, &extensions))
                .then_with(|| cmp_label(&left.label, &right.label))
                .then_with(|| left.name.cmp(&right.name))
        });
        for core in ranked {
            choices.push(CoreChoice {
                path: Some(core.path.clone()),
                label: core.label.clone(),
                detail: file_name(&core.path),
            });
        }
        choices
    }

    fn cores_for(&self, index: usize) -> &[Core] {
        let Some(id) = self
            .rows
            .get(index)
            .and_then(|row| row.emulator_id.as_deref())
        else {
            return &[];
        };
        let Some(pos) = self.emulators.iter().position(|emulator| emulator.id == id) else {
            return &[];
        };
        self.catalogs
            .get(pos)
            .map(|catalog| catalog.cores.as_slice())
            .unwrap_or(&[])
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
        extensions: console.extensions.clone(),
        emulator_id: console.emulator.clone().filter(|id| !id.trim().is_empty()),
        core: console.core.clone(),
        args: LineEdit::plain(console.extra_args.clone()),
    }
}

fn core_label(cores: &[Core], path: Option<&Path>) -> (String, bool) {
    let Some(path) = path else {
        return ("None".to_string(), false);
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

struct EmulatorChoice {
    id: Option<String>,
    label: String,
}

struct CoreChoice {
    path: Option<PathBuf>,
    label: String,
    detail: String,
}

fn none_core() -> CoreChoice {
    CoreChoice {
        path: None,
        label: "None".to_string(),
        detail: String::new(),
    }
}

fn menu_from(items: Vec<Item>, cursor: usize) -> Menu {
    let cursor = if items.is_empty() {
        0
    } else {
        cursor.min(items.len() - 1)
    };
    Menu { cursor, items }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("core")
        .to_string()
}

fn cmp_label(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}

fn core_rank(core: &Core, name: &str, id: &str, extensions: &[String]) -> u8 {
    if core_matches(core, name, id, extensions) {
        0
    } else {
        1
    }
}

const GENERIC_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "bin", "rom", "iso", "cue", "img", "chd", "m3u", "toc", "ccd", "nrg",
];

fn core_matches(core: &Core, name: &str, id: &str, extensions: &[String]) -> bool {
    let system = alnum(&core.systemname);
    if !system.is_empty() && (same_name(&system, name) || same_name(&system, id)) {
        return true;
    }
    extensions.iter().any(|ext| {
        let ext = ext.trim().trim_start_matches('.');
        if ext.is_empty() || generic_extension(ext) {
            return false;
        }
        core.extensions
            .iter()
            .any(|core_ext| core_ext.eq_ignore_ascii_case(ext))
    })
}

fn generic_extension(ext: &str) -> bool {
    GENERIC_EXTENSIONS
        .iter()
        .any(|generic| ext.eq_ignore_ascii_case(generic))
}

fn same_name(system: &str, other: &str) -> bool {
    let other = alnum(other);
    if system.is_empty() || other.is_empty() {
        return false;
    }
    if system == other {
        return true;
    }
    let (short, long) = if system.len() < other.len() {
        (system, other.as_str())
    } else {
        (other.as_str(), system)
    };
    short.len() >= 4 && long.contains(short)
}

fn alnum(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picker::Jump;
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
            config: None,
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

    fn catalog() -> CoreCatalog {
        CoreCatalog {
            directory: PathBuf::from("/cores"),
            cores: vec![core()],
            cfg: PathBuf::from("/cfg/retroarch.cfg"),
            note: None,
        }
    }

    fn screen() -> Systems {
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.consoles.push(snes());
        Systems::open(&config, vec![catalog()])
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
        let mut screen = Systems::open(&config, vec![catalog()]);
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
        assert_eq!(screen.move_dir(NavDir::Right), Step::Stay);
        assert!(screen.panel().core_menu.is_some());
        assert_eq!(screen.move_dir(NavDir::Down), Step::Stay);
        assert_eq!(screen.confirm(), Step::Write);
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
            config: None,
        });
        screen.move_dir(NavDir::Right);
        assert_eq!(screen.move_dir(NavDir::Right), Step::Stay);
        assert_eq!(screen.move_dir(NavDir::Down), Step::Stay);
        assert_eq!(screen.confirm(), Step::Write);
        assert_eq!(screen.rows()[0].emulator_id.as_deref(), Some("dolphin"));
        assert!(screen.rows()[0].core.is_none());
        assert!(!screen.panel().core_enabled);
    }

    #[test]
    fn a_systems_core_list_follows_that_systems_emulator() {
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.emulators.push(Emulator {
            id: "retroarch-flatpak".into(),
            name: "RetroArch (Flatpak)".into(),
            kind: EmulatorKind::RetroArch,
            path: "flatpak run org.libretro.RetroArch".into(),
            global_args: String::new(),
            config: None,
        });
        config.consoles.push(snes());
        let flatpak = CoreCatalog {
            directory: PathBuf::from("/flatpak"),
            cores: vec![Core {
                name: "nestopia".into(),
                path: PathBuf::from("/flatpak/nestopia_libretro.so"),
                label: "Nestopia".into(),
                systemname: String::new(),
                extensions: Vec::new(),
            }],
            cfg: PathBuf::from("/flatpak/retroarch.cfg"),
            note: None,
        };
        let mut screen = Systems::open(&config, vec![catalog(), flatpak]);
        screen.move_dir(NavDir::Right);
        screen.move_dir(NavDir::Down);
        screen.move_dir(NavDir::Right);
        screen.move_dir(NavDir::Down);
        assert_eq!(screen.confirm(), Step::Write);
        assert_eq!(
            screen.rows()[0].core.as_deref(),
            Some(Path::new("/cores/snes9x_libretro.so"))
        );
        screen.move_dir(NavDir::Up);
        screen.move_dir(NavDir::Right);
        screen.move_dir(NavDir::Down);
        assert_eq!(screen.confirm(), Step::Write);
        assert_eq!(
            screen.rows()[0].emulator_id.as_deref(),
            Some("retroarch-flatpak")
        );
        assert_eq!(
            screen.rows()[0].core.as_deref(),
            Some(Path::new("/cores/snes9x_libretro.so"))
        );
        assert!(screen.panel().core_missing);
        screen.move_dir(NavDir::Down);
        screen.move_dir(NavDir::Right);
        screen.move_dir(NavDir::Down);
        assert_eq!(screen.confirm(), Step::Write);
        assert_eq!(
            screen.rows()[0].core.as_deref(),
            Some(Path::new("/flatpak/nestopia_libretro.so"))
        );
        assert!(!screen.panel().core_missing);
    }

    #[test]
    fn assignments_round_trip_and_leave_other_console_fields() {
        let mut screen = screen();
        screen.move_dir(NavDir::Right);
        screen.move_dir(NavDir::Down);
        screen.move_dir(NavDir::Right);
        screen.move_dir(NavDir::Down);
        screen.confirm();
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

    fn listed_core(name: &str, label: &str, systemname: &str, extensions: &[&str]) -> Core {
        Core {
            name: name.into(),
            path: PathBuf::from(format!("/cores/{name}_libretro.so")),
            label: label.into(),
            systemname: systemname.into(),
            extensions: extensions.iter().map(|ext| (*ext).to_string()).collect(),
        }
    }

    fn atari() -> Systems {
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.consoles.push(Console {
            id: "atari2600".into(),
            name: "Atari 2600".into(),
            rom_dirs: vec![PathBuf::from("/tmp/roms/atari2600")],
            extensions: vec!["a26".into(), "bin".into(), "zip".into()],
            emulator: Some("retroarch".into()),
            core: Some(PathBuf::from("/cores/zeta_libretro.so")),
            extra_args: String::new(),
            grid_art: GridArt::BoxArt,
            media: MediaToggles::default(),
        });
        Systems::open(
            &config,
            vec![CoreCatalog {
                directory: PathBuf::from("/cores"),
                cores: vec![
                    listed_core("zeta", "Zeta Match", "Nope", &["a26"]),
                    listed_core("alpha", "Alpha Match", "Atari - 2600", &["bin"]),
                    listed_core("binonly", "Bin Only", "Other", &["bin"]),
                    listed_core("fbneo", "Arcade (FinalBurn Neo)", "Arcade", &["zip"]),
                ],
                cfg: PathBuf::from("/cfg/retroarch.cfg"),
                note: None,
            }],
        )
    }

    fn open_core(screen: &mut Systems) {
        screen.move_dir(NavDir::Right);
        screen.move_dir(NavDir::Down);
        screen.confirm();
    }

    #[test]
    fn matching_cores_sort_first_and_the_current_core_is_highlighted() {
        let mut screen = atari();
        open_core(&mut screen);
        let menu = screen.panel().core_menu.expect("core menu");
        let labels: Vec<_> = menu.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "None",
                "Alpha Match",
                "Zeta Match",
                "Arcade (FinalBurn Neo)",
                "Bin Only",
            ]
        );
        assert_eq!(menu.items[1].detail, "alpha_libretro.so");
        assert_eq!(menu.items[2].detail, "zeta_libretro.so");
        assert_eq!(menu.cursor, 2);
        assert!(menu.items[0].detail.is_empty());
    }

    #[test]
    fn type_ahead_selects_and_escape_leaves_the_core_alone() {
        let mut screen = atari();
        open_core(&mut screen);
        screen.type_text("al");
        assert_eq!(screen.panel().core_menu.expect("open").cursor, 1);
        assert!(screen.dismiss());
        assert_eq!(
            screen.rows()[0].core.as_deref(),
            Some(Path::new("/cores/zeta_libretro.so"))
        );
        assert!(screen.panel().core_menu.is_none());
        assert!(!screen.dismiss());
        screen.confirm();
        screen.type_text("alpha");
        assert_eq!(screen.confirm(), Step::Write);
        assert_eq!(
            screen.rows()[0].core.as_deref(),
            Some(Path::new("/cores/alpha_libretro.so"))
        );
        assert_eq!(screen.rows()[0].args.text, "");
        screen.confirm();
        screen.type_text("no");
        assert_eq!(screen.confirm(), Step::Write);
        assert!(screen.rows()[0].core.is_none());
        assert_eq!(screen.panel().core, "None");
    }

    #[test]
    fn left_returns_to_the_list_and_left_while_open_only_closes() {
        let mut screen = atari();
        screen.move_dir(NavDir::Right);
        assert!(!screen.list_focused());
        screen.move_dir(NavDir::Left);
        assert!(screen.list_focused());
        assert_eq!(screen.rows()[0].emulator_id.as_deref(), Some("retroarch"));
        open_core(&mut screen);
        let before = screen.rows()[0].core.clone();
        screen.move_dir(NavDir::Down);
        screen.move_dir(NavDir::Left);
        assert!(!screen.list_focused());
        assert!(screen.panel().core_menu.is_none());
        assert_eq!(screen.rows()[0].core, before);
    }

    #[test]
    fn page_home_and_end_move_the_open_list() {
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.consoles.push(snes());
        let cores = (0..12)
            .map(|index| {
                listed_core(
                    &format!("c{index:02}"),
                    &format!("Core {index:02}"),
                    "Other",
                    &[],
                )
            })
            .collect();
        let mut screen = Systems::open(
            &config,
            vec![CoreCatalog {
                directory: PathBuf::from("/cores"),
                cores,
                cfg: PathBuf::from("/cfg/retroarch.cfg"),
                note: None,
            }],
        );
        open_core(&mut screen);
        assert!(screen.jump(Jump::End));
        assert_eq!(screen.panel().core_menu.expect("end").cursor, 13);
        assert!(screen.jump(Jump::Home));
        assert_eq!(screen.panel().core_menu.expect("home").cursor, 0);
        assert!(screen.jump(Jump::PageDown));
        assert_eq!(screen.panel().core_menu.expect("page").cursor, 8);
        assert!(screen.dismiss());
        assert!(!screen.jump(Jump::Home));
    }
}
