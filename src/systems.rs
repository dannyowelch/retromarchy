use crate::catalog::{self, SystemEntry};
use crate::config::{self, Config};
use crate::cores::{Core, CoreCatalog};
use crate::game_menu::LineEdit;
use crate::gamepad::NavDir;
use crate::picker::{Item, Jump, Menu, Picker};
use crate::scanner;
use crate::split::{Side, Split};
use crate::types::{Console, Emulator, EmulatorKind, GridArt, MediaToggles};
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
    Path(usize),
    AddPath,
    Browse,
    Rescan,
    Delete,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemRow {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    pub emulator_id: Option<String>,
    pub core: Option<PathBuf>,
    pub args: LineEdit,
    pub rom_dirs: Vec<String>,
    pub path_draft: LineEdit,
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
pub struct PathLine {
    pub index: usize,
    pub path: String,
    pub aimed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmSlot {
    Cancel,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub name: String,
    pub slot: ConfirmSlot,
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
    pub paths: Vec<PathLine>,
    pub path_draft: LineEdit,
    pub path_draft_aimed: bool,
    pub browse_aimed: bool,
    pub rescan_aimed: bool,
    pub delete_aimed: bool,
    pub adding: bool,
    pub types_available: bool,
    pub emulator_menu: Option<Menu>,
    pub core_menu: Option<Menu>,
    pub type_menu: Option<Menu>,
    pub confirm: Option<Confirm>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Stay,
    Write,
    Rescan,
    Browse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Systems {
    emulators: Vec<Emulator>,
    rows: Vec<SystemRow>,
    catalogs: Vec<CoreCatalog>,
    split: Split,
    field: Field,
    picker: Picker,
    type_query: String,
    confirm: Option<Confirm>,
    dropped: Option<String>,
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
            type_query: String::new(),
            confirm: None,
            dropped: None,
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

    pub fn selected_id(&self) -> Option<String> {
        self.rows.get(self.split.index).map(|row| row.id.clone())
    }

    pub fn take_dropped(&mut self) -> Option<String> {
        self.dropped.take()
    }

    pub fn list_rows(&self) -> Vec<ListRow> {
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| ListRow {
                index,
                title: row.name.clone(),
                detail: self.emulator_label(row.emulator_id.as_deref()),
                selected: index == self.split.index,
                add: false,
            })
            .collect();
        rows.push(ListRow {
            index: self.rows.len(),
            title: "Add system".to_string(),
            detail: String::new(),
            selected: self.split.index == self.rows.len(),
            add: true,
        });
        rows
    }

    pub fn panel(&self) -> Panel {
        let editing = self.split.side == Side::Panel && self.confirm.is_none();
        if self.adding() {
            let types_available = !self.type_choices().is_empty();
            let type_menu = self.type_menu();
            let confirm = self.confirm.clone();
            return Panel {
                empty: false,
                adding: true,
                types_available,
                type_menu,
                confirm,
                ..empty_panel()
            };
        }
        let Some(row) = self.rows.get(self.split.index) else {
            return Panel {
                empty: true,
                confirm: self.confirm.clone(),
                ..empty_panel()
            };
        };
        let row = row.clone();
        let name = row.name.clone();
        let emulator = self.emulator_label(row.emulator_id.as_deref());
        let (core, missing) = core_label(self.cores_for(self.split.index), row.core.as_deref());
        let args = row.args.clone();
        let retroarch = self.retroarch_selected(self.split.index);
        let paths = row
            .rom_dirs
            .iter()
            .enumerate()
            .map(|(index, path)| PathLine {
                index,
                path: path.clone(),
                aimed: editing && self.field == Field::Path(index),
            })
            .collect();
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
            paths,
            path_draft: row.path_draft,
            path_draft_aimed: editing && self.field == Field::AddPath,
            browse_aimed: editing && self.field == Field::Browse,
            rescan_aimed: editing && self.field == Field::Rescan,
            delete_aimed: editing && self.field == Field::Delete,
            adding: false,
            types_available: false,
            emulator_menu,
            core_menu,
            type_menu: None,
            confirm: self.confirm.clone(),
        }
    }

    pub fn select(&mut self, index: usize) {
        if index <= self.rows.len() {
            self.picker.close();
            self.type_query.clear();
            self.confirm = None;
            self.split.select(index);
            self.field = if self.adding() {
                Field::Type
            } else {
                Field::Emulator
            };
        }
    }

    pub fn aim(&mut self, field: Field) {
        if self.adding() || self.rows.is_empty() || self.confirm.is_some() {
            return;
        }
        if field == Field::Core && !self.retroarch_selected(self.split.index) {
            return;
        }
        if !self.fields().contains(&field) {
            return;
        }
        self.picker.close();
        self.split.enter();
        self.field = field;
    }

    pub fn aim_confirm(&mut self, slot: ConfirmSlot) {
        if let Some(prompt) = &mut self.confirm {
            prompt.slot = slot;
        }
    }

    pub fn click_field(&mut self, field: Field) {
        if self.confirm.is_some() || self.adding() || self.rows.is_empty() {
            return;
        }
        if field == Field::Core && !self.retroarch_selected(self.split.index) {
            return;
        }
        match field {
            Field::Emulator | Field::Core => {
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
            Field::Args
            | Field::AddPath
            | Field::Path(_)
            | Field::Browse
            | Field::Rescan
            | Field::Delete => {
                self.picker.close();
                self.split.enter();
                self.field = field;
            }
            Field::Type => {}
        }
    }

    pub fn choose(&mut self, index: usize) -> Step {
        if self.adding() {
            return self.add_type(index);
        }
        if !self.picker.is_open() {
            return Step::Stay;
        }
        self.picker.close();
        match self.field {
            Field::Emulator => self.set_emulator(index),
            Field::Core => self.set_core(index),
            _ => Step::Stay,
        }
    }

    pub fn dismiss(&mut self) -> bool {
        if self.confirm.take().is_some() {
            return true;
        }
        if !self.picker.is_open() {
            return false;
        }
        self.picker.close();
        self.type_query.clear();
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
        let field = if self.adding() {
            Field::Type
        } else {
            self.field
        };
        Some((field, cursor.min(len - 1)))
    }

    pub fn move_dir(&mut self, dir: NavDir) -> Step {
        if self.confirm.is_some() {
            let slot = match dir {
                NavDir::Left | NavDir::Up => ConfirmSlot::Cancel,
                NavDir::Right | NavDir::Down => ConfirmSlot::Delete,
            };
            self.aim_confirm(slot);
            return Step::Stay;
        }
        if self.split.side == Side::List {
            match dir {
                NavDir::Up => self.split.move_list(-1, self.list_len()),
                NavDir::Down => self.split.move_list(1, self.list_len()),
                NavDir::Right => self.enter_panel(),
                NavDir::Left => {}
            }
            return Step::Stay;
        }
        if self.adding() {
            let len = self.type_choices().len();
            if !self.picker.is_open() && len > 0 {
                self.picker.open_at(0);
            }
            match dir {
                NavDir::Up => {
                    self.type_query.clear();
                    self.picker.move_by(-1, len);
                }
                NavDir::Down => {
                    self.type_query.clear();
                    self.picker.move_by(1, len);
                }
                NavDir::Left => {
                    self.picker.close();
                    self.type_query.clear();
                    self.split.leave();
                }
                NavDir::Right => {}
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
        if self.confirm.is_some() {
            let slot = if backward {
                ConfirmSlot::Cancel
            } else {
                ConfirmSlot::Delete
            };
            self.aim_confirm(slot);
            return;
        }
        self.picker.close();
        if self.split.side == Side::List {
            if !backward {
                self.enter_panel();
            }
            return;
        }
        if backward {
            self.type_query.clear();
            self.split.leave();
            return;
        }
        if self.adding() {
            return;
        }
        self.shift_field(1);
    }

    pub fn confirm(&mut self) -> Step {
        if let Some(prompt) = self.confirm.clone() {
            self.confirm = None;
            if prompt.slot == ConfirmSlot::Delete {
                return self.remove_selected();
            }
            return Step::Stay;
        }
        if self.split.side == Side::List {
            self.enter_panel();
            return Step::Stay;
        }
        if self.adding() {
            if !self.picker.is_open() {
                self.enter_panel();
                return Step::Stay;
            }
            let index = self.picker.cursor().unwrap_or(0);
            return self.add_type(index);
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
            Field::Path(index) => self.remove_path(index),
            Field::AddPath => self.commit_draft(),
            Field::Browse => Step::Browse,
            Field::Rescan => {
                self.commit_draft();
                Step::Rescan
            }
            Field::Delete => self.ask_delete(),
            Field::Type => Step::Stay,
        }
    }

    pub fn accepts_text(&self) -> bool {
        if self.confirm.is_some() {
            return false;
        }
        if self.adding() && self.split.side == Side::Panel {
            return true;
        }
        self.split.side == Side::Panel
            && matches!(self.field, Field::Args | Field::AddPath)
            && !self.picker.is_open()
    }

    pub fn type_text(&mut self, text: &str) {
        if self.confirm.is_some() {
            return;
        }
        if self.adding() && self.split.side == Side::Panel {
            self.push_type_query(text);
            return;
        }
        if self.picker.is_open() {
            let labels = self.choice_labels();
            self.picker.type_ahead(text, &labels);
            return;
        }
        if self.field == Field::AddPath {
            if let Some(edit) = self.draft_mut() {
                edit.insert(text);
            }
            return;
        }
        if self.accepts_text() {
            if let Some(edit) = self.args_mut() {
                edit.insert(text);
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.confirm.is_some() {
            return;
        }
        if self.adding() && self.split.side == Side::Panel {
            self.pop_type_query();
            return;
        }
        if self.picker.is_open() || !self.accepts_text() {
            return;
        }
        if self.field == Field::AddPath {
            if let Some(edit) = self.draft_mut() {
                edit.backspace();
            }
            return;
        }
        if let Some(edit) = self.args_mut() {
            edit.backspace();
        }
    }

    pub fn delete_forward(&mut self) {
        if self.confirm.is_some() || self.adding() || self.picker.is_open() || !self.accepts_text()
        {
            return;
        }
        if self.field == Field::AddPath {
            if let Some(edit) = self.draft_mut() {
                edit.delete_forward();
            }
            return;
        }
        if let Some(edit) = self.args_mut() {
            edit.delete_forward();
        }
    }

    pub fn add_rom_dir(&mut self, path: PathBuf) -> Step {
        if self.adding() || self.confirm.is_some() {
            return Step::Stay;
        }
        let Some(path) = normalize_path(&path.display().to_string()) else {
            return Step::Stay;
        };
        let Some(row) = self.rows.get(self.split.index) else {
            return Step::Stay;
        };
        if row.rom_dirs.iter().any(|have| same_path(have, &path)) {
            return Step::Stay;
        }
        if let Some(row) = self.rows.get_mut(self.split.index) {
            row.rom_dirs.push(path);
        }
        self.picker.close();
        self.split.enter();
        self.field = Field::AddPath;
        Step::Write
    }

    pub fn set_catalogs(&mut self, catalogs: Vec<CoreCatalog>) {
        self.catalogs = catalogs;
    }

    fn enter_panel(&mut self) {
        self.confirm = None;
        if self.adding() {
            self.split.enter();
            self.field = Field::Type;
            self.type_query.clear();
            let len = self.type_choices().len();
            if len == 0 {
                self.picker.close();
            } else {
                self.picker.open_at(0);
            }
            return;
        }
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
        let count = self
            .rows
            .get(self.split.index)
            .map(|row| row.rom_dirs.len())
            .unwrap_or(0);
        for index in 0..count {
            fields.push(Field::Path(index));
        }
        fields.push(Field::AddPath);
        fields.push(Field::Browse);
        fields.push(Field::Rescan);
        fields.push(Field::Delete);
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
            Field::Args | Field::AddPath => {
                let Some(edit) = (if self.field == Field::AddPath {
                    self.draft_mut()
                } else {
                    self.args_mut()
                }) else {
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
            Field::Path(_) | Field::Browse | Field::Rescan | Field::Delete | Field::Type => {
                if dir == NavDir::Left {
                    self.split.leave();
                }
                Step::Stay
            }
        }
    }

    fn args_mut(&mut self) -> Option<&mut LineEdit> {
        let index = self.split.index;
        self.rows.get_mut(index).map(|row| &mut row.args)
    }

    fn draft_mut(&mut self) -> Option<&mut LineEdit> {
        let index = self.split.index;
        self.rows.get_mut(index).map(|row| &mut row.path_draft)
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
            _ => 0,
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
        if self.adding() {
            return self.type_choices().len();
        }
        match self.field {
            Field::Emulator => self.emulator_choices().len(),
            Field::Core => self.core_choices().len(),
            _ => 0,
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
            _ => Vec::new(),
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
            _ => (None, None),
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

    fn adding(&self) -> bool {
        self.split.index >= self.rows.len()
    }

    fn list_len(&self) -> usize {
        self.rows.len() + 1
    }

    fn type_choices(&self) -> Vec<&'static SystemEntry> {
        let taken: Vec<_> = self.rows.iter().map(|row| row.id.as_str()).collect();
        catalog::ordered()
            .into_iter()
            .filter(|entry| !taken.iter().any(|id| *id == entry.folder_id))
            .collect()
    }

    fn type_menu(&self) -> Option<Menu> {
        if self.split.side != Side::Panel {
            return None;
        }
        let cursor = self.picker.cursor()?;
        let items = self
            .type_choices()
            .into_iter()
            .map(|entry| Item {
                label: entry.display_name.clone(),
                detail: type_detail(entry),
            })
            .collect();
        Some(menu_from(items, cursor))
    }

    fn add_type(&mut self, index: usize) -> Step {
        let choices = self.type_choices();
        let Some(entry) = choices.get(index) else {
            return Step::Stay;
        };
        if self.rows.iter().any(|row| row.id == entry.folder_id) {
            return Step::Stay;
        }
        self.rows.push(SystemRow {
            id: entry.folder_id.clone(),
            name: entry.display_name.clone(),
            extensions: entry.extensions.clone(),
            emulator_id: None,
            core: None,
            args: LineEdit::plain(String::new()),
            rom_dirs: Vec::new(),
            path_draft: LineEdit::plain(String::new()),
        });
        let index = self.rows.len() - 1;
        self.picker.close();
        self.type_query.clear();
        self.split.select(index);
        self.split.enter();
        self.field = Field::AddPath;
        Step::Write
    }

    fn ask_delete(&mut self) -> Step {
        let Some(row) = self.rows.get(self.split.index) else {
            return Step::Stay;
        };
        self.picker.close();
        self.confirm = Some(Confirm {
            name: row.name.clone(),
            slot: ConfirmSlot::Cancel,
        });
        Step::Stay
    }

    fn remove_selected(&mut self) -> Step {
        if self.adding() || self.rows.is_empty() {
            return Step::Stay;
        }
        let index = self.split.index.min(self.rows.len() - 1);
        let id = self.rows[index].id.clone();
        self.rows.remove(index);
        self.dropped = Some(id);
        if self.rows.is_empty() {
            self.split.index = 0;
        } else if self.split.index >= self.rows.len() {
            self.split.index = self.rows.len() - 1;
        }
        self.picker.close();
        self.split.leave();
        self.field = Field::Emulator;
        Step::Write
    }

    fn remove_path(&mut self, index: usize) -> Step {
        let left = {
            let Some(row) = self.rows.get_mut(self.split.index) else {
                return Step::Stay;
            };
            if index >= row.rom_dirs.len() {
                return Step::Stay;
            }
            row.rom_dirs.remove(index);
            row.rom_dirs.len()
        };
        if left == 0 {
            self.field = Field::AddPath;
        } else if let Field::Path(current) = self.field {
            if current >= left {
                self.field = Field::Path(left - 1);
            }
        }
        Step::Write
    }

    fn commit_draft(&mut self) -> Step {
        let Some(row) = self.rows.get(self.split.index) else {
            return Step::Stay;
        };
        let Some(path) = normalize_path(&row.path_draft.text) else {
            return Step::Stay;
        };
        if row.rom_dirs.iter().any(|have| same_path(have, &path)) {
            if let Some(row) = self.rows.get_mut(self.split.index) {
                row.path_draft = LineEdit::plain(String::new());
            }
            return Step::Stay;
        }
        if let Some(row) = self.rows.get_mut(self.split.index) {
            row.rom_dirs.push(path);
            row.path_draft = LineEdit::plain(String::new());
        }
        Step::Write
    }

    fn push_type_query(&mut self, text: &str) {
        let Some(extra) = query_letters(text) else {
            return;
        };
        if !self.picker.is_open() {
            let len = self.type_choices().len();
            if len == 0 {
                return;
            }
            self.picker.open_at(0);
        }
        let choices = self.type_choices();
        let extended = format!("{}{extra}", self.type_query);
        if let Some(index) = find_type(&choices, &extended) {
            self.type_query = extended;
            self.picker.seek(index);
            return;
        }
        if let Some(index) = find_type(&choices, &extra) {
            self.type_query = extra;
            self.picker.seek(index);
        }
    }

    fn pop_type_query(&mut self) {
        if self.type_query.pop().is_none() {
            return;
        }
        let choices = self.type_choices();
        if let Some(index) = find_type(&choices, &self.type_query) {
            self.picker.seek(index);
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
}

pub fn apply_systems(config: &mut Config, rows: &[SystemRow]) {
    let mut next = Vec::with_capacity(rows.len());
    for row in rows {
        let mut console = config
            .consoles
            .iter()
            .find(|console| console.id == row.id)
            .cloned()
            .unwrap_or_else(|| Console {
                id: row.id.clone(),
                name: row.name.clone(),
                rom_dirs: Vec::new(),
                extensions: row.extensions.clone(),
                emulator: None,
                core: None,
                extra_args: String::new(),
                grid_art: GridArt::default(),
                media: MediaToggles::default(),
            });
        let retroarch = row.emulator_id.as_ref().is_some_and(|id| {
            config
                .emulators
                .iter()
                .any(|emulator| emulator.id == *id && emulator.kind == EmulatorKind::RetroArch)
        });
        console.name = row.name.clone();
        console.extensions = row.extensions.clone();
        console.rom_dirs = row.rom_dirs.iter().map(PathBuf::from).collect();
        console.emulator = row.emulator_id.clone().filter(|id| !id.is_empty());
        console.core = if retroarch { row.core.clone() } else { None };
        console.extra_args = row.args.text.clone();
        next.push(console);
    }
    config.consoles = next;
}

fn system_row(console: &Console) -> SystemRow {
    SystemRow {
        id: console.id.clone(),
        name: console.name.clone(),
        extensions: console.extensions.clone(),
        emulator_id: console.emulator.clone().filter(|id| !id.trim().is_empty()),
        core: console.core.clone(),
        args: LineEdit::plain(console.extra_args.clone()),
        rom_dirs: console
            .rom_dirs
            .iter()
            .map(|path| path.display().to_string())
            .filter(|path| !path.trim().is_empty())
            .collect(),
        path_draft: LineEdit::plain(String::new()),
    }
}

fn empty_panel() -> Panel {
    Panel {
        empty: false,
        name: String::new(),
        emulator: "None".to_string(),
        emulator_aimed: false,
        core: "No core".to_string(),
        core_missing: false,
        core_enabled: false,
        core_aimed: false,
        args: LineEdit::plain(String::new()),
        args_aimed: false,
        paths: Vec::new(),
        path_draft: LineEdit::plain(String::new()),
        path_draft_aimed: false,
        browse_aimed: false,
        rescan_aimed: false,
        delete_aimed: false,
        adding: false,
        types_available: false,
        emulator_menu: None,
        core_menu: None,
        type_menu: None,
        confirm: None,
    }
}

fn type_detail(entry: &SystemEntry) -> String {
    match config::console_year(&entry.folder_id) {
        Some(year) => format!("{} · {year}", entry.folder_id),
        None => entry.folder_id.clone(),
    }
}

fn normalize_path(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = scanner::expand_home(Path::new(trimmed));
    let shown = expanded.display().to_string();
    let shown = shown.trim();
    if shown.is_empty() {
        None
    } else {
        Some(shown.to_string())
    }
}

fn same_path(left: &str, right: &str) -> bool {
    scanner::expand_home(Path::new(left.trim())) == scanner::expand_home(Path::new(right.trim()))
}

fn query_letters(text: &str) -> Option<String> {
    let out: String = text
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn find_type(entries: &[&SystemEntry], query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let query = query.to_ascii_lowercase();
    entries
        .iter()
        .position(|entry| entry.folder_id.to_ascii_lowercase().starts_with(&query))
        .or_else(|| {
            entries.iter().position(|entry| {
                entry
                    .display_name
                    .to_ascii_lowercase()
                    .split(|ch: char| !ch.is_ascii_alphanumeric())
                    .filter(|word| !word.is_empty())
                    .any(|word| word.starts_with(&query))
            })
        })
        .or_else(|| {
            entries.iter().position(|entry| {
                entry.display_name.to_ascii_lowercase().contains(&query)
                    || entry.folder_id.to_ascii_lowercase().contains(&query)
            })
        })
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
        config.consoles[0].grid_art = GridArt::Screenshot;
        config.consoles[0].media.manual = true;
        apply_systems(&mut config, screen.rows());
        assert_eq!(
            config.consoles[0].core.as_deref(),
            Some(Path::new("/cores/snes9x_libretro.so"))
        );
        assert_eq!(config.consoles[0].extra_args, "--region ntsc");
        assert_eq!(
            config.consoles[0].rom_dirs,
            vec![PathBuf::from("/tmp/roms/snes")]
        );
        assert_eq!(config.consoles[0].grid_art, GridArt::Screenshot);
        assert!(config.consoles[0].media.manual);
    }

    #[test]
    fn add_skips_types_already_present_and_type_ahead_adds_one() {
        let mut screen = screen();
        screen.move_dir(NavDir::Down);
        assert!(screen.list_rows().last().unwrap().add);
        screen.move_dir(NavDir::Right);
        let menu = screen.panel().type_menu.expect("type list");
        assert!(menu
            .items
            .iter()
            .all(|item| !item.detail.starts_with("snes ")));
        screen.type_text("nes");
        let menu = screen.panel().type_menu.expect("type list");
        assert!(menu.items[menu.cursor].detail.starts_with("nes "));
        assert_eq!(screen.rows().len(), 1);
        assert_eq!(screen.confirm(), Step::Write);
        let added = screen.rows().last().unwrap();
        assert_eq!(added.id, "nes");
        assert!(added.extensions.iter().any(|ext| ext == "nes"));
        assert!(added.rom_dirs.is_empty());
        assert!(screen.panel().path_draft_aimed);
    }

    #[test]
    fn rom_paths_add_and_remove_and_tilde_expands() {
        let root = tempfile::tempdir().unwrap();
        let _env = crate::config::XdgEnv::sandbox(root.path());
        let mut screen = screen();
        screen.aim(Field::AddPath);
        screen.type_text("/roms/a");
        assert_eq!(screen.confirm(), Step::Write);
        screen.type_text("~/roms/b");
        assert_eq!(screen.confirm(), Step::Write);
        screen.type_text("/roms/a");
        assert_eq!(screen.confirm(), Step::Stay);
        let home = root.path().join("home").join("roms").join("b");
        assert_eq!(
            screen.rows()[0].rom_dirs,
            vec![
                "/tmp/roms/snes".to_string(),
                "/roms/a".to_string(),
                home.display().to_string()
            ]
        );
        screen.aim(Field::Path(1));
        assert_eq!(screen.confirm(), Step::Write);
        assert_eq!(
            screen.rows()[0].rom_dirs,
            vec!["/tmp/roms/snes".to_string(), home.display().to_string()]
        );
        let mut config = Config::default();
        config.emulators.push(retroarch());
        config.consoles.push(snes());
        apply_systems(&mut config, screen.rows());
        assert_eq!(config.consoles[0].rom_dirs[1], home);
    }

    #[test]
    fn delete_starts_on_cancel_and_drops_the_system_from_config() {
        let mut screen = screen();
        screen.aim(Field::Delete);
        assert_eq!(screen.confirm(), Step::Stay);
        assert_eq!(
            screen.panel().confirm.expect("dialog").slot,
            ConfirmSlot::Cancel
        );
        assert_eq!(screen.confirm(), Step::Stay);
        assert!(screen.panel().confirm.is_none());
        assert_eq!(screen.rows().len(), 1);
        screen.aim(Field::Delete);
        assert_eq!(screen.confirm(), Step::Stay);
        screen.move_dir(NavDir::Right);
        assert_eq!(
            screen.panel().confirm.expect("dialog").slot,
            ConfirmSlot::Delete
        );
        assert_eq!(screen.confirm(), Step::Write);
        assert!(screen.rows().is_empty());
        assert_eq!(screen.take_dropped().as_deref(), Some("snes"));
        let mut config = Config::default();
        config.consoles.push(snes());
        apply_systems(&mut config, screen.rows());
        assert!(config.consoles.is_empty());
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
