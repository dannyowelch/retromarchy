use crate::catalog;
use crate::config::{self, Config};
use crate::cores::{self, CoreProfile, DiscoveredCore};
use crate::importer::{self, FoundFolder, ImportChoice};
use crate::types::{EmulatorProfile, ProviderEntry, ScraperConfig, ScraperCredentials};
use gtk4::prelude::*;
use gtk4::{
    Align, Box, Button, CheckButton, ComboBoxText, Entry, FileChooserAction, FileChooserDialog,
    FileFilter, Label, ListBox, Orientation, PolicyType, ResponseType, ScrolledWindow, Window,
};
use libadwaita as adw;
use rusqlite::Connection;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub fn open_import(
    parent: &adw::ApplicationWindow,
    config: Rc<RefCell<Config>>,
    conn: Rc<RefCell<Connection>>,
    on_done: Rc<dyn Fn()>,
) {
    let window = Window::builder()
        .title("Import ROMs")
        .modal(true)
        .transient_for(parent)
        .default_width(560)
        .default_height(420)
        .build();

    let page = Box::new(Orientation::Vertical, 12);
    page.set_margin_top(24);
    page.set_margin_bottom(24);
    page.set_margin_start(24);
    page.set_margin_end(24);

    let title = Label::new(Some("How would you like to import ROMs?"));
    title.add_css_class("title-2");
    page.append(&title);

    let hint = Label::new(Some("Paths are stored. ROM files are not copied."));
    hint.add_css_class("dim-label");
    hint.set_halign(Align::Start);
    page.append(&hint);

    let esde = Button::with_label("Import ES-DE / EmulationStation library");
    esde.add_css_class("suggested-action");
    esde.set_receives_default(true);
    let win = window.clone();
    let config_es = config.clone();
    let conn_es = conn.clone();
    let done_es = on_done.clone();
    esde.connect_clicked(move |_| {
        show_esde(&win, config_es.clone(), conn_es.clone(), done_es.clone());
    });
    page.append(&esde);

    let single = Button::with_label("Add one system");
    let win = window.clone();
    let config_one = config.clone();
    let conn_one = conn;
    single.connect_clicked(move |_| {
        show_single(&win, config_one.clone(), conn_one.clone(), on_done.clone());
    });
    page.append(&single);

    window.set_child(Some(&page));
    window.set_default_widget(Some(&esde));
    close_on_escape(&window);
    window.present();
}

fn show_esde(
    parent: &Window,
    config: Rc<RefCell<Config>>,
    conn: Rc<RefCell<Connection>>,
    on_done: Rc<dyn Fn()>,
) {
    let page = Box::new(Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let label = Label::new(Some(
        "ROMs root (immediate subfolders are matched to systems)",
    ));
    label.set_halign(Align::Start);
    label.set_wrap(true);
    page.append(&label);

    let entry = Entry::new();
    let suggested = importer::default_roms_root();
    entry.set_text(&suggested.display().to_string());
    page.append(&entry);

    let browse = Button::with_label("Browse…");
    let entry_browse = entry.clone();
    let parent_browse = parent.clone();
    browse.connect_clicked(move |_| {
        let dialog = FileChooserDialog::new(
            Some("Choose ROMs folder"),
            Some(&parent_browse),
            FileChooserAction::SelectFolder,
            &[
                ("Cancel", ResponseType::Cancel),
                ("Select", ResponseType::Accept),
            ],
        );
        let entry = entry_browse.clone();
        dialog.connect_response(move |dialog, response| {
            if response == ResponseType::Accept {
                if let Some(file) = dialog.file() {
                    if let Some(path) = file.path() {
                        entry.set_text(&path.display().to_string());
                    }
                }
            }
            dialog.close();
        });
        dialog.show();
    });
    page.append(&browse);

    let next = Button::with_label("Scan folders");
    next.add_css_class("suggested-action");
    next.set_receives_default(true);
    let parent_next = parent.clone();
    next.connect_clicked(move |_| {
        let root = PathBuf::from(entry.text().as_str());
        let found = importer::discover_root(&root);
        show_checklist(
            &parent_next,
            found,
            config.clone(),
            conn.clone(),
            on_done.clone(),
        );
    });
    page.append(&next);
    parent.set_child(Some(&page));
    parent.set_default_widget(Some(&next));
    next.grab_focus();
}

struct ChecklistRow {
    include: bool,
    path: PathBuf,
    system_id: Option<String>,
}

fn show_checklist(
    parent: &Window,
    found: Vec<FoundFolder>,
    config: Rc<RefCell<Config>>,
    conn: Rc<RefCell<Connection>>,
    on_done: Rc<dyn Fn()>,
) {
    let page = Box::new(Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let label = Label::new(Some(
        "Check systems to import. Unmatched folders stay listed so you can remap them.",
    ));
    label.set_wrap(true);
    label.set_halign(Align::Start);
    page.append(&label);

    let scrolled = ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(PolicyType::Never)
        .min_content_height(280)
        .build();
    let list = ListBox::new();
    list.add_css_class("boxed-list");
    scrolled.set_child(Some(&list));

    let rows: Rc<RefCell<Vec<ChecklistRow>>> = Rc::new(RefCell::new(
        found
            .iter()
            .map(|f| ChecklistRow {
                include: f.matched_id.is_some(),
                path: f.path.clone(),
                system_id: f.matched_id.clone(),
            })
            .collect(),
    ));

    for (index, folder) in found.iter().enumerate() {
        let row_box = Box::new(Orientation::Horizontal, 8);
        row_box.set_margin_top(6);
        row_box.set_margin_bottom(6);
        row_box.set_margin_start(6);
        row_box.set_margin_end(6);

        let check = CheckButton::new();
        check.set_active(folder.matched_id.is_some());
        let rows_check = rows.clone();
        check.connect_toggled(move |btn| {
            if let Some(row) = rows_check.borrow_mut().get_mut(index) {
                row.include = btn.is_active();
            }
        });
        row_box.append(&check);

        let system = folder
            .matched_id
            .as_deref()
            .and_then(|id| catalog::systems().iter().find(|s| s.folder_id == id));
        let name = system
            .map(|s| s.display_name.as_str())
            .unwrap_or("Unmatched");
        let text = Label::new(Some(&format!(
            "{name}\n{} · {} files",
            folder.folder_name, folder.file_count
        )));
        text.set_halign(Align::Start);
        text.set_hexpand(true);
        text.set_wrap(true);
        row_box.append(&text);

        let combo = ComboBoxText::new();
        combo.append(Some(""), "Unmapped");
        for system in catalog::ordered() {
            combo.append(Some(&system.folder_id), &system.display_name);
        }
        if let Some(id) = &folder.matched_id {
            combo.set_active_id(Some(id));
        } else {
            combo.set_active_id(Some(""));
        }
        let rows_combo = rows.clone();
        combo.connect_changed(move |combo| {
            if let Some(row) = rows_combo.borrow_mut().get_mut(index) {
                row.system_id = combo
                    .active_id()
                    .filter(|id| !id.is_empty())
                    .map(|id| id.to_string());
            }
        });
        row_box.append(&combo);
        list.append(&row_box);
    }

    page.append(&scrolled);

    let actions = Box::new(Orientation::Horizontal, 8);
    actions.set_halign(Align::End);
    let cancel = Button::with_label("Cancel");
    let parent_cancel = parent.clone();
    cancel.connect_clicked(move |_| parent_cancel.close());
    actions.append(&cancel);

    let import = Button::with_label("Import");
    import.add_css_class("suggested-action");
    import.set_receives_default(true);
    let parent_import = parent.clone();
    import.connect_clicked(move |_| {
        let choices: Vec<ImportChoice> = rows
            .borrow()
            .iter()
            .filter(|row| row.include)
            .filter_map(|row| {
                row.system_id.as_ref().map(|id| ImportChoice {
                    system_id: id.clone(),
                    path: row.path.clone(),
                })
            })
            .collect();
        let mut cfg = config.borrow_mut();
        if importer::apply_imports(&mut cfg, &conn.borrow(), &choices).is_ok() {
            drop(cfg);
            on_done();
            parent_import.close();
        }
    });
    actions.append(&import);
    page.append(&actions);
    parent.set_child(Some(&page));
    parent.set_default_widget(Some(&import));
    import.grab_focus();
}

fn show_single(
    parent: &Window,
    config: Rc<RefCell<Config>>,
    conn: Rc<RefCell<Connection>>,
    on_done: Rc<dyn Fn()>,
) {
    let page = Box::new(Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search systems"));
    page.append(&search);

    let scrolled = ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(PolicyType::Never)
        .min_content_height(280)
        .build();
    let list = ListBox::new();
    list.add_css_class("boxed-list");
    scrolled.set_child(Some(&list));
    page.append(&scrolled);

    let selected: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let systems = Rc::new(catalog::ordered());

    let fill = {
        let list = list.clone();
        let systems = systems.clone();
        move |query: &str| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let q = query.to_ascii_lowercase();
            for system in systems.iter() {
                if !q.is_empty()
                    && !system.display_name.to_ascii_lowercase().contains(&q)
                    && !system.folder_id.contains(&q)
                {
                    continue;
                }
                let label = Label::new(Some(&format!(
                    "{} ({})",
                    system.display_name, system.folder_id
                )));
                label.set_widget_name(&system.folder_id);
                label.set_halign(Align::Start);
                label.set_margin_top(6);
                label.set_margin_bottom(6);
                label.set_margin_start(8);
                label.set_margin_end(8);
                list.append(&label);
            }
        }
    };
    fill("");

    let selected_search = selected.clone();
    list.connect_row_selected(move |_, row| {
        *selected_search.borrow_mut() = row
            .and_then(|row| row.child())
            .map(|child| child.widget_name().to_string())
            .filter(|id| !id.is_empty());
    });

    let list_filter = list.clone();
    let systems_filter = systems.clone();
    search.connect_search_changed(move |entry| {
        let q = entry.text().to_string();
        while let Some(child) = list_filter.first_child() {
            list_filter.remove(&child);
        }
        let needle = q.to_ascii_lowercase();
        for system in systems_filter.iter() {
            if !needle.is_empty()
                && !system.display_name.to_ascii_lowercase().contains(&needle)
                && !system.folder_id.contains(&needle)
            {
                continue;
            }
            let label = Label::new(Some(&format!(
                "{} ({})",
                system.display_name, system.folder_id
            )));
            label.set_widget_name(&system.folder_id);
            label.set_halign(Align::Start);
            label.set_margin_top(6);
            label.set_margin_bottom(6);
            label.set_margin_start(8);
            label.set_margin_end(8);
            list_filter.append(&label);
        }
    });

    let next = Button::with_label("Choose folder");
    next.add_css_class("suggested-action");
    let parent_next = parent.clone();
    next.connect_clicked(move |_| {
        let Some(system_id) = selected.borrow().clone() else {
            return;
        };
        show_single_folder(
            &parent_next,
            system_id,
            config.clone(),
            conn.clone(),
            on_done.clone(),
        );
    });
    page.append(&next);
    parent.set_child(Some(&page));
}

fn show_single_folder(
    parent: &Window,
    system_id: String,
    config: Rc<RefCell<Config>>,
    conn: Rc<RefCell<Connection>>,
    on_done: Rc<dyn Fn()>,
) {
    let page = Box::new(Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let name = catalog::systems()
        .iter()
        .find(|s| s.folder_id == system_id)
        .map(|s| s.display_name.clone())
        .unwrap_or_else(|| system_id.clone());
    let label = Label::new(Some(&format!(
        "Folder for {name}. Individual ROM files are not copied; the parent folder is stored and filtered by extension."
    )));
    label.set_wrap(true);
    label.set_halign(Align::Start);
    page.append(&label);

    let entry = Entry::new();
    page.append(&entry);
    let browse = Button::with_label("Browse…");
    let entry_b = entry.clone();
    let parent_b = parent.clone();
    browse.connect_clicked(move |_| {
        let dialog = FileChooserDialog::new(
            Some("Choose system folder"),
            Some(&parent_b),
            FileChooserAction::SelectFolder,
            &[
                ("Cancel", ResponseType::Cancel),
                ("Select", ResponseType::Accept),
            ],
        );
        let entry = entry_b.clone();
        dialog.connect_response(move |dialog, response| {
            if response == ResponseType::Accept {
                if let Some(file) = dialog.file() {
                    if let Some(path) = file.path() {
                        entry.set_text(&path.display().to_string());
                    }
                }
            }
            dialog.close();
        });
        dialog.show();
    });
    page.append(&browse);

    let add = Button::with_label("Add system");
    add.add_css_class("suggested-action");
    let parent_add = parent.clone();
    add.connect_clicked(move |_| {
        let path = PathBuf::from(entry.text().as_str());
        if entry.text().is_empty() {
            return;
        }
        let choice = ImportChoice {
            system_id: system_id.clone(),
            path,
        };
        let mut cfg = config.borrow_mut();
        if importer::apply_imports(&mut cfg, &conn.borrow(), &[choice]).is_ok() {
            drop(cfg);
            on_done();
            parent_add.close();
        }
    });
    page.append(&add);
    parent.set_child(Some(&page));
}

#[derive(Clone)]
struct EmulatorDialog {
    profiles: ListBox,
    profiles_scroll: ScrolledWindow,
    consoles: ListBox,
    cores: ListBox,
    cores_scroll: ScrolledWindow,
    cores_empty: Label,
    config: Rc<RefCell<Config>>,
    show_cores: bool,
}

pub fn open_emulators(
    parent: &adw::ApplicationWindow,
    config: Rc<RefCell<Config>>,
    on_done: Rc<dyn Fn()>,
) {
    let window = Window::builder()
        .title("Manage Emulators")
        .modal(true)
        .transient_for(parent)
        .default_width(860)
        .default_height(760)
        .build();

    let page = Box::new(Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let heading = Label::new(Some("Emulator profiles"));
    heading.add_css_class("title-3");
    heading.set_halign(Align::Start);
    page.append(&heading);

    let show_cores = which_retroarch();
    let cores_heading = Label::new(Some("Discovered cores"));
    cores_heading.add_css_class("title-4");
    cores_heading.set_halign(Align::Start);
    cores_heading.set_visible(show_cores);
    let cores_empty = Label::new(Some(
        "No cores found in the usual directories. Paste a core path or browse for a .so file. Cores are not downloaded.",
    ));
    cores_empty.set_wrap(true);
    cores_empty.set_halign(Align::Start);
    cores_empty.add_css_class("dim-label");
    cores_empty.set_visible(false);
    let cores_scroll = ScrolledWindow::builder()
        .vexpand(true)
        .min_content_height(220)
        .hscrollbar_policy(PolicyType::Never)
        .build();
    let cores = ListBox::new();
    cores.add_css_class("boxed-list");
    cores.set_selection_mode(gtk4::SelectionMode::None);
    cores_scroll.set_child(Some(&cores));

    if show_cores {
        let note = Label::new(Some(
            "retroarch is on PATH. Pick a discovered core or a core file to make a RetroArch profile. Cores are not downloaded.",
        ));
        note.set_wrap(true);
        note.set_halign(Align::Start);
        note.add_css_class("dim-label");
        page.append(&note);
        page.append(&cores_heading);
        page.append(&cores_empty);
        page.append(&cores_scroll);
    }

    let profiles_scroll = ScrolledWindow::builder()
        .min_content_height(48)
        .hscrollbar_policy(PolicyType::Never)
        .build();
    let profiles = ListBox::new();
    profiles.add_css_class("boxed-list");
    profiles_scroll.set_child(Some(&profiles));
    page.append(&profiles_scroll);

    let consoles_scroll = ScrolledWindow::builder()
        .vexpand(true)
        .min_content_height(120)
        .hscrollbar_policy(PolicyType::Never)
        .build();
    let consoles = ListBox::new();
    consoles.add_css_class("boxed-list");
    consoles_scroll.set_child(Some(&consoles));

    let dialog = EmulatorDialog {
        profiles,
        profiles_scroll,
        consoles,
        cores,
        cores_scroll,
        cores_empty,
        config: config.clone(),
        show_cores,
    };
    dialog.reload();

    let id_entry = Entry::builder().placeholder_text("Profile id").build();
    let detail_entry = Entry::builder()
        .placeholder_text("Command with {rom}, or core path")
        .hexpand(true)
        .build();
    let kind = ComboBoxText::new();
    kind.append(Some("standalone"), "Standalone");
    kind.append(Some("retroarch"), "RetroArch");
    kind.set_active_id(Some("standalone"));

    let path_row = Box::new(Orientation::Horizontal, 8);
    path_row.append(&detail_entry);
    let browse = Button::with_label("Browse…");
    let window_browse = window.clone();
    let detail_browse = detail_entry.clone();
    let id_browse = id_entry.clone();
    let kind_browse = kind.clone();
    browse.connect_clicked(move |_| {
        let chooser = FileChooserDialog::new(
            Some("Choose libretro core"),
            Some(&window_browse),
            FileChooserAction::Open,
            &[
                ("Cancel", ResponseType::Cancel),
                ("Select", ResponseType::Accept),
            ],
        );
        let filter = FileFilter::new();
        filter.set_name(Some("Libretro cores (*.so)"));
        filter.add_pattern("*.so");
        chooser.set_filter(&filter);
        let detail = detail_browse.clone();
        let id_entry = id_browse.clone();
        let kind = kind_browse.clone();
        chooser.connect_response(move |chooser, response| {
            if response == ResponseType::Accept {
                if let Some(file) = chooser.file() {
                    if let Some(path) = file.path() {
                        detail.set_text(&path.display().to_string());
                        kind.set_active_id(Some("retroarch"));
                        if id_entry.text().is_empty() {
                            if let Some(core) = DiscoveredCore::from_path(path) {
                                id_entry.set_text(&core.name);
                            }
                        }
                    }
                }
            }
            chooser.close();
        });
        chooser.show();
    });
    path_row.append(&browse);

    let add = Button::with_label("Add profile");
    add.add_css_class("suggested-action");
    let dialog_add = dialog.clone();
    let kind_add = kind.clone();
    let id_add = id_entry.clone();
    let detail_add = detail_entry.clone();
    add.connect_clicked(move |_| {
        let id = id_add.text().to_string();
        let detail = detail_add.text().to_string();
        if id.is_empty() || detail.is_empty() {
            return;
        }
        let profile = if kind_add.active_id().as_deref() == Some("retroarch") {
            EmulatorProfile::RetroArch {
                id,
                core: PathBuf::from(detail),
                config: None,
            }
        } else {
            EmulatorProfile::Standalone {
                id,
                command: detail,
            }
        };
        dialog_add.config.borrow_mut().profiles.push(profile);
        let _ = config::save_config(&dialog_add.config.borrow());
        id_add.set_text("");
        detail_add.set_text("");
        dialog_add.reload();
    });

    page.append(&kind);
    page.append(&id_entry);
    page.append(&path_row);
    page.append(&add);

    let assign = Label::new(Some("Default profile per system"));
    assign.add_css_class("title-4");
    assign.set_halign(Align::Start);
    assign.set_margin_top(8);
    page.append(&assign);
    page.append(&consoles_scroll);

    let close = Button::with_label("Close");
    close.set_halign(Align::End);
    let window_close = window.clone();
    let on_done_close = on_done;
    close.connect_clicked(move |_| {
        on_done_close();
        window_close.close();
    });
    page.append(&close);

    window.set_child(Some(&page));
    id_entry.grab_focus();
    close_on_escape(&window);
    window.present();
}

impl EmulatorDialog {
    fn reload(&self) {
        self.fill_profiles();
        fill_consoles(&self.consoles, &self.config);
        self.fill_cores();
    }

    fn add_core(&self, core: &DiscoveredCore, assign_console: Option<String>) {
        {
            let mut cfg = self.config.borrow_mut();
            let id = match core.to_profile(&cfg.profiles) {
                CoreProfile::AlreadyAdded { id } => id,
                CoreProfile::New(profile) => {
                    let id = profile.id().clone();
                    cfg.profiles.push(profile);
                    id
                }
            };
            if let Some(console_id) = assign_console {
                if let Some(console) = cfg
                    .consoles
                    .iter_mut()
                    .find(|console| console.id == console_id)
                {
                    console.profile = Some(id);
                }
            }
            let _ = config::save_config(&cfg);
        }
        self.reload();
    }

    fn fill_profiles(&self) {
        while let Some(child) = self.profiles.first_child() {
            self.profiles.remove(&child);
        }
        let profiles = self.config.borrow().profiles.clone();
        let empty = profiles.is_empty();
        self.profiles_scroll.set_vexpand(!empty || !self.show_cores);
        self.profiles_scroll
            .set_min_content_height(if empty { 48 } else { 100 });
        for profile in profiles {
            let row = Box::new(Orientation::Horizontal, 8);
            row.set_margin_top(6);
            row.set_margin_bottom(6);
            row.set_margin_start(8);
            row.set_margin_end(8);
            let text = match &profile {
                EmulatorProfile::RetroArch { id, core, .. } => {
                    format!("{id} · RetroArch · {}", core.display())
                }
                EmulatorProfile::Standalone { id, command } => format!("{id} · {command}"),
            };
            let label = Label::new(Some(&text));
            label.set_halign(Align::Start);
            label.set_hexpand(true);
            label.set_wrap(true);
            row.append(&label);
            let delete = Button::with_label("Delete");
            delete.add_css_class("destructive-action");
            let id = profile.id().clone();
            let dialog = self.clone();
            delete.connect_clicked(move |_| {
                dialog
                    .config
                    .borrow_mut()
                    .profiles
                    .retain(|profile| profile.id() != &id);
                let _ = config::save_config(&dialog.config.borrow());
                dialog.reload();
            });
            row.append(&delete);
            self.profiles.append(&row);
        }
    }

    fn fill_cores(&self) {
        while let Some(child) = self.cores.first_child() {
            self.cores.remove(&child);
        }
        if !self.show_cores {
            return;
        }
        let discovered = cores::discover_cores();
        self.cores_empty.set_visible(discovered.is_empty());
        self.cores_scroll.set_visible(!discovered.is_empty());
        let consoles: Vec<(String, String, Option<String>)> = self
            .config
            .borrow()
            .consoles
            .iter()
            .map(|console| {
                (
                    console.id.clone(),
                    console.name.clone(),
                    console.profile.clone(),
                )
            })
            .collect();
        let existing = self.config.borrow().profiles.clone();
        for core in discovered {
            self.cores
                .append(&self.core_row(&core, &consoles, &existing));
        }
    }

    fn core_row(
        &self,
        core: &DiscoveredCore,
        consoles: &[(String, String, Option<String>)],
        existing: &[EmulatorProfile],
    ) -> Box {
        let row = Box::new(Orientation::Horizontal, 8);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(8);
        row.set_margin_end(8);

        let text = Box::new(Orientation::Vertical, 2);
        text.set_hexpand(true);
        let name = Label::new(Some(&core.name));
        name.set_halign(Align::Start);
        name.set_xalign(0.0);
        let path_text = core.path.display().to_string();
        let path = Label::new(Some(&path_text));
        path.set_halign(Align::Start);
        path.set_xalign(0.0);
        path.set_hexpand(true);
        path.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        path.add_css_class("dim-label");
        path.add_css_class("caption");
        path.set_tooltip_text(Some(&path_text));
        text.append(&name);
        text.append(&path);
        row.append(&text);

        let actions = Box::new(Orientation::Horizontal, 6);
        actions.set_valign(Align::Center);
        let resolved = core.to_profile(existing);
        let add = match &resolved {
            CoreProfile::AlreadyAdded { .. } => {
                let button = Button::with_label("Added");
                button.set_sensitive(false);
                button
            }
            CoreProfile::New(_) => {
                let button = Button::with_label("Add profile");
                let dialog = self.clone();
                let core = core.clone();
                button.connect_clicked(move |_| dialog.add_core(&core, None));
                button
            }
        };
        actions.append(&add);

        let added_id = match &resolved {
            CoreProfile::AlreadyAdded { id } => Some(id.clone()),
            CoreProfile::New(_) => None,
        };
        for system_id in core.system_ids() {
            let Some((console_id, console_name, current)) =
                consoles.iter().find(|(id, _, _)| id == system_id)
            else {
                continue;
            };
            let assigned = added_id
                .as_ref()
                .is_some_and(|id| current.as_ref() == Some(id));
            let label = if assigned {
                format!("Assigned to {console_name}")
            } else if added_id.is_some() {
                format!("Assign to {console_name}")
            } else {
                format!("Add & assign to {console_name}")
            };
            let button_label = Label::new(Some(&label));
            button_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            button_label.set_max_width_chars(32);
            button_label.set_tooltip_text(Some(&label));
            let button = Button::new();
            button.set_child(Some(&button_label));
            button.set_tooltip_text(Some(&label));
            button.add_css_class("suggested-action");
            button.set_sensitive(!assigned);
            if !assigned {
                let dialog = self.clone();
                let core = core.clone();
                let console_id = console_id.clone();
                button.connect_clicked(move |_| dialog.add_core(&core, Some(console_id.clone())));
            }
            actions.append(&button);
        }
        row.append(&actions);
        row
    }
}

fn fill_consoles(list: &ListBox, config: &Rc<RefCell<Config>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let cfg = config.borrow();
    for console in cfg.consoles.clone() {
        let row = Box::new(Orientation::Horizontal, 8);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(8);
        row.set_margin_end(8);
        let label = Label::new(Some(&console.name));
        label.set_halign(Align::Start);
        label.set_hexpand(true);
        row.append(&label);
        let combo = ComboBoxText::new();
        combo.append(Some(""), "None");
        for profile in &cfg.profiles {
            combo.append(Some(profile.id()), profile.id());
        }
        match console.profile.as_deref() {
            Some(id) if !id.is_empty() => {
                combo.set_active_id(Some(id));
            }
            _ => {
                combo.set_active_id(Some(""));
            }
        }
        let console_id = console.id.clone();
        let config_set = config.clone();
        combo.connect_changed(move |combo| {
            let mut cfg = config_set.borrow_mut();
            if let Some(console) = cfg.consoles.iter_mut().find(|c| c.id == console_id) {
                console.profile = combo
                    .active_id()
                    .filter(|id| !id.is_empty())
                    .map(|id| id.to_string());
                let _ = config::save_config(&cfg);
            }
        });
        row.append(&combo);
        list.append(&row);
        if console.id == cfg.consoles.first().map(|c| c.id.as_str()).unwrap_or("")
            && !cfg.profiles.is_empty()
        {
            combo.grab_focus();
        }
    }
}

pub fn open_scraper(parent: &adw::ApplicationWindow, config: Rc<RefCell<Config>>) {
    let window = Window::builder()
        .title("Scraper settings")
        .modal(true)
        .transient_for(parent)
        .default_width(560)
        .default_height(640)
        .build();

    let page = Box::new(Orientation::Vertical, 12);
    page.set_margin_top(16);
    page.set_margin_bottom(16);
    page.set_margin_start(16);
    page.set_margin_end(16);

    let title = Label::new(Some("Artwork scraper"));
    title.add_css_class("title-2");
    title.set_halign(Align::Start);
    page.append(&title);

    let hint = Label::new(Some(
        "Downloads box art, title screens, and screenshots only. ROMs and BIOS are never downloaded.",
    ));
    hint.set_wrap(true);
    hint.set_halign(Align::Start);
    hint.add_css_class("dim-label");
    page.append(&hint);

    let scraper = config.borrow().scraper.clone();
    let box_art = CheckButton::with_label("Box art");
    box_art.set_active(scraper.box_art);
    let title_screen = CheckButton::with_label("Title screen");
    title_screen.set_active(scraper.title_screen);
    let screenshot = CheckButton::with_label("Screenshot");
    screenshot.set_active(scraper.screenshot);
    page.append(&box_art);
    page.append(&title_screen);
    page.append(&screenshot);

    let providers_label = Label::new(Some("Providers, highest priority first"));
    providers_label.add_css_class("title-4");
    providers_label.set_halign(Align::Start);
    providers_label.set_margin_top(8);
    page.append(&providers_label);

    let provider_note = Label::new(Some(
        "Each missing artwork type tries this list in order and stops at the first hit.",
    ));
    provider_note.set_wrap(true);
    provider_note.set_halign(Align::Start);
    provider_note.add_css_class("dim-label");
    page.append(&provider_note);

    let list = ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    let providers = Rc::new(RefCell::new(scraper.providers));
    refill_providers(&list, &providers);
    page.append(&list);

    let creds_label = Label::new(Some("Credentials"));
    creds_label.add_css_class("title-4");
    creds_label.set_halign(Align::Start);
    creds_label.set_margin_top(8);
    page.append(&creds_label);

    let creds_note = Label::new(Some(
        "Stored in config.toml. ScreenScraper needs your free member username and password. The application Softname is built in. TheGamesDB needs an API key.",
    ));
    creds_note.set_wrap(true);
    creds_note.set_halign(Align::Start);
    creds_note.add_css_class("dim-label");
    page.append(&creds_note);

    let user = Entry::builder()
        .placeholder_text("ScreenScraper username")
        .text(&scraper.credentials.screenscraper_user)
        .build();
    let password = Entry::builder()
        .placeholder_text("ScreenScraper password")
        .text(&scraper.credentials.screenscraper_password)
        .visibility(false)
        .build();
    let api_key = Entry::builder()
        .placeholder_text("TheGamesDB API key")
        .text(&scraper.credentials.thegamesdb_api_key)
        .visibility(false)
        .build();
    page.append(&user);
    page.append(&password);
    page.append(&api_key);

    let save = Button::with_label("Save");
    save.add_css_class("suggested-action");
    save.set_halign(Align::End);
    let window_save = window.clone();
    save.connect_clicked(move |_| {
        let mut cfg = config.borrow_mut();
        cfg.scraper = ScraperConfig {
            box_art: box_art.is_active(),
            title_screen: title_screen.is_active(),
            screenshot: screenshot.is_active(),
            providers: providers.borrow().clone(),
            credentials: ScraperCredentials {
                screenscraper_user: user.text().to_string(),
                screenscraper_password: password.text().to_string(),
                thegamesdb_api_key: api_key.text().to_string(),
            },
        };
        let _ = config::save_config(&cfg);
        window_save.close();
    });
    page.append(&save);

    let scrolled = ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(PolicyType::Never)
        .child(&page)
        .build();
    window.set_child(Some(&scrolled));
    window.set_default_widget(Some(&save));
    close_on_escape(&window);
    window.present();
}

fn refill_providers(list: &ListBox, providers: &Rc<RefCell<Vec<ProviderEntry>>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let entries = providers.borrow().clone();
    for (index, entry) in entries.iter().enumerate() {
        let row = Box::new(Orientation::Horizontal, 8);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(8);
        row.set_margin_end(8);
        let check = CheckButton::with_label(entry.id.label());
        check.set_active(entry.enabled);
        check.set_hexpand(true);
        check.set_halign(Align::Start);
        let providers_toggle = providers.clone();
        let list_toggle = list.clone();
        check.connect_toggled(move |btn| {
            if let Some(slot) = providers_toggle.borrow_mut().get_mut(index) {
                slot.enabled = btn.is_active();
            }
            let _ = &list_toggle;
        });
        row.append(&check);

        let up = Button::with_label("Up");
        up.set_sensitive(index > 0);
        let providers_up = providers.clone();
        let list_up = list.clone();
        up.connect_clicked(move |_| {
            let mut entries = providers_up.borrow_mut();
            if index > 0 && index < entries.len() {
                entries.swap(index, index - 1);
            }
            drop(entries);
            refill_providers(&list_up, &providers_up);
        });
        row.append(&up);

        let down = Button::with_label("Down");
        down.set_sensitive(index + 1 < entries.len());
        let providers_down = providers.clone();
        let list_down = list.clone();
        down.connect_clicked(move |_| {
            let mut entries = providers_down.borrow_mut();
            if index + 1 < entries.len() {
                entries.swap(index, index + 1);
            }
            drop(entries);
            refill_providers(&list_down, &providers_down);
        });
        row.append(&down);
        list.append(&row);
    }
}

fn close_on_escape(window: &Window) {
    let key = gtk4::EventControllerKey::new();
    let window_key = window.clone();
    key.connect_key_pressed(move |_, key, _, _| {
        if key == gtk4::gdk::Key::Escape {
            window_key.close();
            gtk4::glib::Propagation::Stop
        } else {
            gtk4::glib::Propagation::Proceed
        }
    });
    window.add_controller(key);
}

fn which_retroarch() -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join("retroarch").is_file()))
        .unwrap_or(false)
}
