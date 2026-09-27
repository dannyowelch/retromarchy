//! ROM import steps from the GTK dialog: ES-DE root, or one system.
//! The shell paints this. Folder discovery and the library write live in [`crate::importer`].

use crate::catalog;
use crate::game_menu::LineEdit;
use crate::gamepad::NavDir;
use crate::importer::{self, FoundFolder, ImportChoice};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemName {
    id: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemLine {
    pub row: SystemRow,
    pub mapped: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemRow {
    pub include: bool,
    pub discovered: String,
    pub folder_name: String,
    pub path: PathBuf,
    pub file_count: usize,
    pub system_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChooseSlot {
    Esde,
    Single,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSlot {
    Path,
    Browse,
    Scan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemsSlot {
    Check(usize),
    Map(usize),
    Cancel,
    Import,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickSlot {
    Search,
    Row(usize),
    Choose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderSlot {
    Path,
    Browse,
    Add,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFocus {
    Choose(ChooseSlot),
    Root(RootSlot),
    Systems(SystemsSlot),
    Pick(PickSlot),
    Folder(FolderSlot),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Page {
    Choose {
        slot: ChooseSlot,
    },
    Root {
        edit: LineEdit,
        slot: RootSlot,
    },
    Systems {
        rows: Vec<SystemRow>,
        slot: SystemsSlot,
    },
    Pick {
        edit: LineEdit,
        slot: PickSlot,
        selected: usize,
    },
    Folder {
        system_id: String,
        name: String,
        edit: LineEdit,
        slot: FolderSlot,
    },
    Working {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportCommand {
    None,
    Close,
    Discover(PathBuf),
    Apply(Vec<ImportChoice>),
    Browse,
}

#[derive(Debug)]
pub enum View<'a> {
    Choose {
        slot: ChooseSlot,
    },
    Root {
        edit: &'a LineEdit,
        slot: RootSlot,
    },
    Systems {
        rows: &'a [SystemRow],
        slot: SystemsSlot,
    },
    Pick {
        edit: &'a LineEdit,
        slot: PickSlot,
    },
    Folder {
        name: &'a str,
        edit: &'a LineEdit,
        slot: FolderSlot,
    },
    Working {
        message: &'a str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportWizard {
    systems: Vec<SystemName>,
    page: Page,
    error: Option<String>,
    stash: Option<Page>,
}

/// Ctrl+I, including Ctrl+Shift+I. GTK checks the control mask and `i` / `I`.
pub fn import_key(key: &str, key_char: Option<&str>, control: bool) -> bool {
    if !control {
        return false;
    }
    key.eq_ignore_ascii_case("i") || key_char.is_some_and(|ch| ch.eq_ignore_ascii_case("i"))
}

impl ImportWizard {
    pub fn open() -> Self {
        Self {
            systems: catalog_names(),
            page: Page::Choose {
                slot: ChooseSlot::Esde,
            },
            error: None,
            stash: None,
        }
    }

    pub fn view(&self) -> View<'_> {
        match &self.page {
            Page::Choose { slot } => View::Choose { slot: *slot },
            Page::Root { edit, slot } => View::Root { edit, slot: *slot },
            Page::Systems { rows, slot } => View::Systems { rows, slot: *slot },
            Page::Pick { edit, slot, .. } => View::Pick { edit, slot: *slot },
            Page::Folder { name, edit, slot, .. } => View::Folder {
                name,
                edit,
                slot: *slot,
            },
            Page::Working { message } => View::Working { message },
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn blocks_escape(&self) -> bool {
        matches!(self.page, Page::Working { .. })
    }

    pub fn accepts_text(&self) -> bool {
        matches!(
            self.page,
            Page::Root {
                slot: RootSlot::Path,
                ..
            } | Page::Pick {
                slot: PickSlot::Search,
                ..
            } | Page::Folder {
                slot: FolderSlot::Path,
                ..
            }
        )
    }

    pub fn systems_row(&self) -> Option<usize> {
        match &self.page {
            Page::Systems {
                slot: SystemsSlot::Check(index) | SystemsSlot::Map(index),
                ..
            } => Some(*index),
            _ => None,
        }
    }

    pub fn pick_row(&self) -> Option<usize> {
        match &self.page {
            Page::Pick {
                slot: PickSlot::Row(index),
                ..
            } => Some(*index),
            _ => None,
        }
    }

    pub fn mapped_name(&self, id: Option<&str>) -> &str {
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            return "Unmapped";
        };
        self.systems
            .iter()
            .find(|system| system.id == id)
            .map(|system| system.name.as_str())
            .unwrap_or("Unmapped")
    }

    pub fn system_lines(&self) -> Option<(Vec<SystemLine>, SystemsSlot)> {
        let copied = match &self.page {
            Page::Systems { rows, slot } => Some((rows.clone(), *slot)),
            _ => None,
        };
        let (rows, slot) = copied?;
        let lines = rows
            .into_iter()
            .map(|row| {
                let mapped = self.mapped_name(row.system_id.as_deref()).to_string();
                SystemLine { row, mapped }
            })
            .collect();
        Some((lines, slot))
    }

    pub fn pick_matches(&self) -> Vec<(&str, &str)> {
        let Page::Pick { edit, .. } = &self.page else {
            return Vec::new();
        };
        self.matches(&edit.text)
            .iter()
            .map(|system| (system.id.as_str(), system.name.as_str()))
            .collect()
    }

    pub fn aim(&mut self, focus: ImportFocus) {
        self.error = None;
        match (&mut self.page, focus) {
            (Page::Choose { slot }, ImportFocus::Choose(next)) => *slot = next,
            (Page::Root { slot, .. }, ImportFocus::Root(next)) => *slot = next,
            (Page::Systems { slot, .. }, ImportFocus::Systems(next)) => *slot = next,
            (Page::Pick { slot, selected, .. }, ImportFocus::Pick(next)) => {
                if let PickSlot::Row(index) = next {
                    *selected = index;
                }
                *slot = next;
            }
            (Page::Folder { slot, .. }, ImportFocus::Folder(next)) => *slot = next,
            _ => {}
        }
    }

    pub fn move_dir(&mut self, dir: NavDir) {
        if self.accepts_text() && matches!(dir, NavDir::Left | NavDir::Right) {
            let delta = if dir == NavDir::Left { -1 } else { 1 };
            if let Some(edit) = self.edit_mut() {
                edit.move_caret(delta);
            }
            return;
        }
        if matches!(dir, NavDir::Left | NavDir::Right) && self.on_map() {
            let delta = if dir == NavDir::Left { -1 } else { 1 };
            self.cycle(delta);
            return;
        }
        let delta = match dir {
            NavDir::Up | NavDir::Left => -1,
            NavDir::Down | NavDir::Right => 1,
        };
        self.step(delta);
    }

    pub fn tab(&mut self, backward: bool) {
        self.step(if backward { -1 } else { 1 });
    }

    pub fn type_text(&mut self, text: &str) {
        if let Some(edit) = self.edit_mut() {
            edit.insert(text);
        }
        self.clamp_pick();
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if let Some(edit) = self.edit_mut() {
            edit.backspace();
        }
        self.clamp_pick();
    }

    pub fn delete_forward(&mut self) {
        if let Some(edit) = self.edit_mut() {
            edit.delete_forward();
        }
        self.clamp_pick();
    }

    /// Space toggles the focused checkbox. Other controls use Enter.
    pub fn toggle_focused(&mut self) -> bool {
        let Page::Systems {
            slot: SystemsSlot::Check(index),
            ..
        } = &self.page
        else {
            return false;
        };
        let index = *index;
        self.toggle(index);
        true
    }

    pub fn set_included(&mut self, index: usize, include: bool) {
        if let Page::Systems { rows, slot, .. } = &mut self.page {
            if let Some(row) = rows.get_mut(index) {
                row.include = include;
            }
            *slot = SystemsSlot::Check(index);
        }
    }

    pub fn cycle_focused(&mut self, delta: isize) {
        if self.on_map() {
            self.cycle(delta);
        }
    }

    pub fn set_path(&mut self, path: PathBuf) {
        let text = path.display().to_string();
        match &mut self.page {
            Page::Root { edit, slot } => {
                *edit = LineEdit::plain(text);
                *slot = RootSlot::Path;
            }
            Page::Folder { edit, slot, .. } => {
                *edit = LineEdit::plain(text);
                *slot = FolderSlot::Path;
            }
            _ => {}
        }
    }

    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
    }

    pub fn show_folders(&mut self, found: Vec<FoundFolder>) {
        self.stash = None;
        self.error = None;
        let rows = found.into_iter().map(|folder| self.row_for(folder)).collect();
        self.page = Page::Systems {
            rows,
            slot: SystemsSlot::Import,
        };
    }

    pub fn note_progress(&mut self, message: String) {
        if let Page::Working { message: slot } = &mut self.page {
            *slot = message;
        }
    }

    pub fn fail(&mut self, message: String) {
        if let Some(page) = self.stash.take() {
            self.page = page;
        }
        self.error = Some(message);
    }

    pub fn confirm(&mut self) -> ImportCommand {
        if matches!(self.page, Page::Working { .. }) {
            return ImportCommand::None;
        }
        self.error = None;
        match self.page.clone() {
            Page::Choose { slot } => match slot {
                ChooseSlot::Esde => {
                    self.page = Page::Root {
                        edit: LineEdit::plain(importer::default_roms_root().display().to_string()),
                        slot: RootSlot::Scan,
                    };
                    ImportCommand::None
                }
                ChooseSlot::Single => {
                    self.page = Page::Pick {
                        edit: LineEdit::plain(String::new()),
                        slot: PickSlot::Search,
                        selected: 0,
                    };
                    ImportCommand::None
                }
            },
            Page::Root { edit, slot } => match slot {
                RootSlot::Browse => ImportCommand::Browse,
                RootSlot::Path | RootSlot::Scan => self.begin_discover(edit.text),
            },
            Page::Systems { rows, slot } => match slot {
                SystemsSlot::Cancel => ImportCommand::Close,
                SystemsSlot::Check(index) => {
                    self.toggle(index);
                    ImportCommand::None
                }
                SystemsSlot::Map(_) => ImportCommand::None,
                SystemsSlot::Import => {
                    let choices = import_choices(&rows);
                    let message = if choices.is_empty() {
                        "Saving…".into()
                    } else {
                        "Scanning games…".into()
                    };
                    self.start_work(message);
                    ImportCommand::Apply(choices)
                }
            },
            Page::Pick { edit, slot, selected } => match slot {
                PickSlot::Search => {
                    if self.matches(&edit.text).is_empty() {
                        self.error = Some("Choose a system.".into());
                        return ImportCommand::None;
                    }
                    if let Page::Pick { slot, selected, .. } = &mut self.page {
                        *slot = PickSlot::Row(0);
                        *selected = 0;
                    }
                    ImportCommand::None
                }
                PickSlot::Row(_) | PickSlot::Choose => self.open_folder(&edit.text, selected),
            },
            Page::Folder {
                system_id,
                name,
                edit,
                slot,
            } => match slot {
                FolderSlot::Browse => ImportCommand::Browse,
                FolderSlot::Path | FolderSlot::Add => {
                    let path = PathBuf::from(edit.text.trim());
                    if path.as_os_str().is_empty() {
                        self.error = Some("Choose a folder.".into());
                        return ImportCommand::None;
                    }
                    self.start_work(format!("Scanning {name}…"));
                    ImportCommand::Apply(vec![ImportChoice { system_id, path }])
                }
            },
            Page::Working { .. } => ImportCommand::None,
        }
    }

    fn begin_discover(&mut self, text: String) -> ImportCommand {
        let path = PathBuf::from(text.trim());
        if path.as_os_str().is_empty() {
            self.error = Some("Enter a ROMs folder.".into());
            return ImportCommand::None;
        }
        self.start_work("Scanning folders…".into());
        ImportCommand::Discover(path)
    }

    fn open_folder(&mut self, query: &str, selected: usize) -> ImportCommand {
        let chosen = self
            .matches(query)
            .get(selected)
            .map(|system| (system.id.clone(), system.name.clone()));
        let Some((system_id, name)) = chosen else {
            self.error = Some("Choose a system.".into());
            return ImportCommand::None;
        };
        self.page = Page::Folder {
            system_id,
            name,
            edit: LineEdit::plain(String::new()),
            slot: FolderSlot::Path,
        };
        ImportCommand::None
    }

    fn start_work(&mut self, message: String) {
        self.error = None;
        let previous = std::mem::replace(&mut self.page, Page::Working { message });
        self.stash = Some(previous);
    }

    fn step(&mut self, delta: isize) {
        let Some(next) = self.neighbor(delta) else {
            return;
        };
        self.aim(next);
    }

    fn neighbor(&self, delta: isize) -> Option<ImportFocus> {
        let picked = match &self.page {
            Page::Pick { edit, slot, .. } => Some((edit.text.clone(), *slot)),
            _ => None,
        };
        if let Some((query, slot)) = picked {
            let slots = pick_slots(self.matches(&query).len());
            let pos = slots.iter().position(|item| *item == slot).unwrap_or(0);
            return Some(ImportFocus::Pick(
                slots[nudge_index(pos, slots.len(), delta)],
            ));
        }
        match &self.page {
            Page::Choose { slot } => {
                let slots = [ChooseSlot::Esde, ChooseSlot::Single];
                Some(ImportFocus::Choose(nudge_copy(*slot, &slots, delta)))
            }
            Page::Root { slot, .. } => {
                let slots = [RootSlot::Path, RootSlot::Browse, RootSlot::Scan];
                Some(ImportFocus::Root(nudge_copy(*slot, &slots, delta)))
            }
            Page::Folder { slot, .. } => {
                let slots = [FolderSlot::Path, FolderSlot::Browse, FolderSlot::Add];
                Some(ImportFocus::Folder(nudge_copy(*slot, &slots, delta)))
            }
            Page::Systems { rows, slot } => {
                let slots = system_slots(rows.len());
                let pos = slots.iter().position(|item| item == slot).unwrap_or(0);
                Some(ImportFocus::Systems(
                    slots[nudge_index(pos, slots.len(), delta)],
                ))
            }
            Page::Pick { .. } | Page::Working { .. } => None,
        }
    }

    fn on_map(&self) -> bool {
        matches!(
            self.page,
            Page::Systems {
                slot: SystemsSlot::Map(_),
                ..
            }
        )
    }

    fn cycle(&mut self, delta: isize) {
        let index = match &self.page {
            Page::Systems {
                slot: SystemsSlot::Map(index),
                ..
            } => *index,
            _ => return,
        };
        let current = match &self.page {
            Page::Systems { rows, .. } => rows.get(index).and_then(|row| row.system_id.clone()),
            _ => None,
        };
        let len = self.systems.len();
        if len == 0 {
            return;
        }
        let pos = self
            .systems
            .iter()
            .position(|system| {
                if system.id.is_empty() {
                    current.is_none()
                } else {
                    current.as_deref() == Some(system.id.as_str())
                }
            })
            .unwrap_or(0);
        let next = (pos as isize + delta).rem_euclid(len as isize) as usize;
        let new_id = {
            let id = &self.systems[next].id;
            if id.is_empty() {
                None
            } else {
                Some(id.clone())
            }
        };
        if let Page::Systems { rows, .. } = &mut self.page {
            if let Some(row) = rows.get_mut(index) {
                row.system_id = new_id;
            }
        }
    }

    fn toggle(&mut self, index: usize) {
        if let Page::Systems { rows, .. } = &mut self.page {
            if let Some(row) = rows.get_mut(index) {
                row.include = !row.include;
            }
        }
    }

    fn edit_mut(&mut self) -> Option<&mut LineEdit> {
        match &mut self.page {
            Page::Root { edit, slot } if *slot == RootSlot::Path => Some(edit),
            Page::Pick { edit, slot, .. } if *slot == PickSlot::Search => Some(edit),
            Page::Folder { edit, slot, .. } if *slot == FolderSlot::Path => Some(edit),
            _ => None,
        }
    }

    fn clamp_pick(&mut self) {
        let query = match &self.page {
            Page::Pick { edit, .. } => edit.text.clone(),
            _ => return,
        };
        let len = self.matches(&query).len();
        if let Page::Pick { slot, selected, .. } = &mut self.page {
            if len == 0 {
                *selected = 0;
                if matches!(slot, PickSlot::Row(_)) {
                    *slot = PickSlot::Choose;
                }
            } else if *selected >= len {
                *selected = len - 1;
                if let PickSlot::Row(index) = slot {
                    *index = len - 1;
                }
            }
        }
    }

    fn matches(&self, query: &str) -> Vec<&SystemName> {
        let needle = query.trim().to_ascii_lowercase();
        self.systems
            .iter()
            .filter(|system| !system.id.is_empty())
            .filter(|system| {
                needle.is_empty()
                    || system.name.to_ascii_lowercase().contains(&needle)
                    || system.id.contains(&needle)
            })
            .collect()
    }

    fn row_for(&self, folder: FoundFolder) -> SystemRow {
        let discovered = match folder.matched_id.as_deref() {
            Some(id) => self
                .systems
                .iter()
                .find(|system| system.id == id)
                .map(|system| system.name.clone())
                .unwrap_or_else(|| "Unmatched".into()),
            None => "Unmatched".into(),
        };
        SystemRow {
            include: folder.matched_id.is_some(),
            discovered,
            folder_name: folder.folder_name,
            path: folder.path,
            file_count: folder.file_count,
            system_id: folder.matched_id,
        }
    }
}

fn catalog_names() -> Vec<SystemName> {
    let mut systems = vec![SystemName {
        id: String::new(),
        name: "Unmapped".into(),
    }];
    for system in catalog::ordered() {
        systems.push(SystemName {
            id: system.folder_id.clone(),
            name: system.display_name.clone(),
        });
    }
    systems
}

fn import_choices(rows: &[SystemRow]) -> Vec<ImportChoice> {
    rows.iter()
        .filter(|row| row.include)
        .filter_map(|row| {
            row.system_id.as_ref().map(|id| ImportChoice {
                system_id: id.clone(),
                path: row.path.clone(),
            })
        })
        .collect()
}

fn system_slots(len: usize) -> Vec<SystemsSlot> {
    let mut slots = Vec::with_capacity(len * 2 + 2);
    for index in 0..len {
        slots.push(SystemsSlot::Check(index));
        slots.push(SystemsSlot::Map(index));
    }
    slots.push(SystemsSlot::Cancel);
    slots.push(SystemsSlot::Import);
    slots
}

fn pick_slots(len: usize) -> Vec<PickSlot> {
    let mut slots = Vec::with_capacity(len + 2);
    slots.push(PickSlot::Search);
    for index in 0..len {
        slots.push(PickSlot::Row(index));
    }
    slots.push(PickSlot::Choose);
    slots
}

fn nudge_index(pos: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (pos as isize + delta).clamp(0, len as isize - 1) as usize
}

fn nudge_copy<T: Copy + PartialEq>(current: T, slots: &[T], delta: isize) -> T {
    let pos = slots.iter().position(|item| *item == current).unwrap_or(0);
    slots[nudge_index(pos, slots.len(), delta)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::discover_root;
    use std::fs;
    use std::path::Path;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"not a rom").unwrap();
    }

    #[test]
    fn ctrl_i_matches_the_gtk_chord() {
        assert!(import_key("i", None, true));
        assert!(import_key("I", None, true));
        assert!(import_key("i", Some("I"), true));
        assert!(!import_key("i", None, false));
        assert!(!import_key("o", Some("o"), true));
    }

    #[test]
    fn three_enters_import_matched_esde_folders() {
        let root = tempfile::tempdir().unwrap();
        touch(&root.path().join("snes/One.sfc"));
        touch(&root.path().join("nes/Mario.nes"));
        touch(&root.path().join("mystery/note.txt"));

        let mut wizard = ImportWizard::open();
        assert!(matches!(
            wizard.view(),
            View::Choose {
                slot: ChooseSlot::Esde
            }
        ));
        assert_eq!(wizard.confirm(), ImportCommand::None);
        assert!(matches!(
            wizard.view(),
            View::Root {
                slot: RootSlot::Scan,
                ..
            }
        ));
        wizard.set_path(root.path().to_path_buf());
        match wizard.confirm() {
            ImportCommand::Discover(path) => assert_eq!(path, root.path()),
            other => panic!("{other:?}"),
        }
        assert!(matches!(wizard.view(), View::Working { .. }));
        assert!(wizard.blocks_escape());
        wizard.show_folders(discover_root(root.path()));
        let View::Systems { rows, slot } = wizard.view() else {
            panic!("expected the checklist");
        };
        assert_eq!(slot, SystemsSlot::Import);
        assert!(rows.iter().any(|row| row.folder_name == "nes" && row.include));
        assert!(rows.iter().any(|row| row.folder_name == "snes" && row.include));
        assert!(rows
            .iter()
            .any(|row| row.folder_name == "mystery" && !row.include && row.system_id.is_none()));
        match wizard.confirm() {
            ImportCommand::Apply(choices) => {
                let ids: Vec<_> = choices.iter().map(|choice| choice.system_id.as_str()).collect();
                assert_eq!(ids, ["nes", "snes"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn remap_and_uncheck_change_the_import() {
        let root = tempfile::tempdir().unwrap();
        touch(&root.path().join("mystery/note.txt"));
        let mut wizard = ImportWizard::open();
        wizard.confirm();
        wizard.set_path(root.path().to_path_buf());
        wizard.confirm();
        wizard.show_folders(discover_root(root.path()));
        wizard.aim(ImportFocus::Systems(SystemsSlot::Map(0)));
        wizard.move_dir(NavDir::Right);
        assert_eq!(
            match wizard.view() {
                View::Systems { rows, .. } => rows[0].system_id.clone(),
                _ => None,
            }
            .as_deref(),
            Some("nes")
        );
        wizard.aim(ImportFocus::Systems(SystemsSlot::Check(0)));
        assert!(wizard.toggle_focused());
        wizard.aim(ImportFocus::Systems(SystemsSlot::Import));
        match wizard.confirm() {
            ImportCommand::Apply(choices) => {
                assert_eq!(choices.len(), 1);
                assert_eq!(choices[0].system_id, "nes");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn one_system_needs_a_folder_before_it_scans() {
        let mut wizard = ImportWizard::open();
        wizard.move_dir(NavDir::Down);
        wizard.confirm();
        assert!(matches!(
            wizard.view(),
            View::Pick {
                slot: PickSlot::Search,
                ..
            }
        ));
        wizard.type_text("snes");
        let matches = wizard.pick_matches();
        assert!(matches.iter().any(|(id, _)| *id == "snes"));
        wizard.confirm();
        assert!(matches!(
            wizard.view(),
            View::Pick {
                slot: PickSlot::Row(0),
                ..
            }
        ));
        // The filtered list starts at popular systems. Land on snes if it is not first.
        let index = wizard
            .pick_matches()
            .iter()
            .position(|(id, _)| *id == "snes")
            .unwrap();
        wizard.aim(ImportFocus::Pick(PickSlot::Row(index)));
        assert_eq!(wizard.confirm(), ImportCommand::None);
        let View::Folder { name, slot, .. } = wizard.view() else {
            panic!("expected the folder page");
        };
        assert_eq!(slot, FolderSlot::Path);
        assert!(name.contains("SNES") || name.contains("Super Nintendo"));
        assert_eq!(wizard.confirm(), ImportCommand::None);
        assert_eq!(wizard.error(), Some("Choose a folder."));
        wizard.set_path(PathBuf::from("/tmp/snes"));
        match wizard.confirm() {
            ImportCommand::Apply(choices) => {
                assert_eq!(choices.len(), 1);
                assert_eq!(choices[0].system_id, "snes");
                assert_eq!(choices[0].path, PathBuf::from("/tmp/snes"));
            }
            other => panic!("{other:?}"),
        }
    }
}
