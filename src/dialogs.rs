use crate::catalog;
use crate::config::{self, Config};
use crate::importer::{self, FoundFolder, ImportChoice};
use crate::types::EmulatorProfile;
use gtk4::prelude::*;
use gtk4::{
    Align, Box, Button, CheckButton, ComboBoxText, Entry, FileChooserAction, FileChooserDialog,
    Label, ListBox, Orientation, PolicyType, ResponseType, ScrolledWindow, Window,
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

    let label = Label::new(Some("ROMs root (immediate subfolders are matched to systems)"));
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
            &[("Cancel", ResponseType::Cancel), ("Select", ResponseType::Accept)],
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
        show_checklist(&parent_next, found, config.clone(), conn.clone(), on_done.clone());
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

    let label = Label::new(Some("Check systems to import. Unmatched folders stay listed so you can remap them."));
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

        let system = folder.matched_id.as_deref().and_then(|id| {
            catalog::systems().iter().find(|s| s.folder_id == id)
        });
        let name = system.map(|s| s.display_name.as_str()).unwrap_or("Unmatched");
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
                row.system_id = combo.active_id().filter(|id| !id.is_empty()).map(|id| id.to_string());
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
                let label = Label::new(Some(&format!("{} ({})", system.display_name, system.folder_id)));
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
        *selected_search.borrow_mut() = row.and_then(|row| row.child()).map(|child| child.widget_name().to_string()).filter(|id| !id.is_empty());
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
            let label = Label::new(Some(&format!("{} ({})", system.display_name, system.folder_id)));
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
        show_single_folder(&parent_next, system_id, config.clone(), conn.clone(), on_done.clone());
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
            &[("Cancel", ResponseType::Cancel), ("Select", ResponseType::Accept)],
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

pub fn open_emulators(parent: &adw::ApplicationWindow, config: Rc<RefCell<Config>>, on_done: Rc<dyn Fn()>) {
    let window = Window::builder()
        .title("Manage Emulators")
        .modal(true)
        .transient_for(parent)
        .default_width(640)
        .default_height(560)
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

    if which_retroarch() {
        let note = Label::new(Some("retroarch is on PATH. Pick a core file to make a RetroArch profile. Cores are not downloaded."));
        note.set_wrap(true);
        note.set_halign(Align::Start);
        note.add_css_class("dim-label");
        page.append(&note);
    }

    let profiles_scroll = ScrolledWindow::builder()
        .vexpand(true)
        .min_content_height(140)
        .hscrollbar_policy(PolicyType::Never)
        .build();
    let profiles_list = ListBox::new();
    profiles_list.add_css_class("boxed-list");
    profiles_scroll.set_child(Some(&profiles_list));
    page.append(&profiles_scroll);

    let consoles_scroll = ScrolledWindow::builder()
        .vexpand(true)
        .min_content_height(140)
        .hscrollbar_policy(PolicyType::Never)
        .build();
    let consoles_list = ListBox::new();
    consoles_list.add_css_class("boxed-list");
    consoles_scroll.set_child(Some(&consoles_list));

    let profiles_list_r = profiles_list.clone();
    let consoles_list_r = consoles_list.clone();
    let config_r = config.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        fill_profiles(&profiles_list_r, &config_r);
        fill_consoles(&consoles_list_r, &config_r);
    });
    refresh();

    let id_entry = Entry::builder().placeholder_text("Profile id").build();
    let detail_entry = Entry::builder().placeholder_text("Command with {rom}, or core path").build();
    let kind = ComboBoxText::new();
    kind.append(Some("standalone"), "Standalone");
    kind.append(Some("retroarch"), "RetroArch");
    kind.set_active_id(Some("standalone"));

    let add = Button::with_label("Add profile");
    add.add_css_class("suggested-action");
    let config_add = config.clone();
    let refresh_add = refresh.clone();
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
            EmulatorProfile::Standalone { id, command: detail }
        };
        config_add.borrow_mut().profiles.push(profile);
        let _ = config::save_config(&config_add.borrow());
        id_add.set_text("");
        detail_add.set_text("");
        refresh_add();
    });

    page.append(&kind);
    page.append(&id_entry);
    page.append(&detail_entry);
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

fn fill_profiles(list: &ListBox, config: &Rc<RefCell<Config>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let cfg = config.borrow();
    for profile in &cfg.profiles {
        let row = Box::new(Orientation::Horizontal, 8);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(8);
        row.set_margin_end(8);
        let text = match profile {
            EmulatorProfile::RetroArch { id, core, .. } => format!("{id} · RetroArch · {}", core.display()),
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
        let config_del = config.clone();
        let list_del = list.clone();
        delete.connect_clicked(move |_| {
            config_del.borrow_mut().profiles.retain(|p| p.id() != &id);
            let _ = config::save_config(&config_del.borrow());
            fill_profiles(&list_del, &config_del);
        });
        row.append(&delete);
        list.append(&row);
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
                console.profile = combo.active_id().filter(|id| !id.is_empty()).map(|id| id.to_string());
                let _ = config::save_config(&cfg);
            }
        });
        row.append(&combo);
        list.append(&row);
        if console.id == cfg.consoles.first().map(|c| c.id.as_str()).unwrap_or("") && !cfg.profiles.is_empty() {
            combo.grab_focus();
        }
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
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join("retroarch").is_file())
        })
        .unwrap_or(false)
}
