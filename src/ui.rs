use crate::config::Config;
use crate::database;
use crate::launcher;
use crate::scanner;
use crate::types::{Game, MediaKind};
use anyhow::Result;
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::prelude::*;
use gtk4::{gdk, glib};
use libadwaita as adw;
use libadwaita::prelude::AdwApplicationWindowExt;
use rusqlite::Connection;
use std::cell::RefCell;
use std::rc::Rc;

pub struct App {
    config: Config,
    conn: Rc<RefCell<Connection>>,
    window: adw::ApplicationWindow,
    console_list: gtk4::ListBox,
    game_grid: gtk4::FlowBox,
    search_bar: gtk4::SearchBar,
    search_entry: gtk4::SearchEntry,
    current_console: Rc<RefCell<Option<String>>>,
    games: Rc<RefCell<Vec<Game>>>,
    all_games: Rc<RefCell<Vec<Game>>>,
    selected_game: Rc<RefCell<Option<usize>>>,
}

impl App {
    pub fn new(app: &adw::Application, config: Config, conn: Rc<RefCell<Connection>>) -> Result<Self> {
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_data(
            "flowboxchild:selected { background: alpha(@accent_bg_color, 0.3); border-radius: 6px; }
             .navigation-sidebar row:selected { background: @accent_bg_color; }
             flowboxchild:focus { outline: 2px solid @accent_color; outline-offset: 2px; }"
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().expect("Could not get default display"),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Retromarchy")
            .default_width(1200)
            .default_height(700)
            .build();

        let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

        let sidebar = gtk4::ScrolledWindow::builder()
            .width_request(200)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();

        let console_list = gtk4::ListBox::new();
        console_list.add_css_class("navigation-sidebar");
        sidebar.set_child(Some(&console_list));

        let center_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        center_box.set_hexpand(true);

        let search_bar = gtk4::SearchBar::new();
        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Filter games..."));
        search_bar.set_child(Some(&search_entry));
        search_bar.set_search_mode(false);
        center_box.append(&search_bar);

        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vexpand(true)
            .build();

        let game_grid = gtk4::FlowBox::new();
        game_grid.set_valign(gtk4::Align::Start);
        game_grid.set_max_children_per_line(6);
        game_grid.set_selection_mode(gtk4::SelectionMode::Single);
        game_grid.set_homogeneous(true);
        game_grid.set_column_spacing(12);
        game_grid.set_row_spacing(12);
        game_grid.set_margin_top(12);
        game_grid.set_margin_bottom(12);
        game_grid.set_margin_start(12);
        game_grid.set_margin_end(12);
        scrolled.set_child(Some(&game_grid));

        center_box.append(&scrolled);

        main_box.append(&sidebar);
        main_box.append(&center_box);

        window.set_content(Some(&main_box));

        let app_instance = Self {
            config,
            conn,
            window: window.clone(),
            console_list: console_list.clone(),
            game_grid: game_grid.clone(),
            search_bar: search_bar.clone(),
            search_entry: search_entry.clone(),
            current_console: Rc::new(RefCell::new(None)),
            games: Rc::new(RefCell::new(Vec::new())),
            all_games: Rc::new(RefCell::new(Vec::new())),
            selected_game: Rc::new(RefCell::new(None)),
        };

        app_instance.setup_console_list();
        app_instance.setup_search();
        app_instance.setup_keyboard_navigation();
        app_instance.setup_game_selection();

        Ok(app_instance)
    }
    
    fn setup_search(&self) {
        let search_entry = self.search_entry.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let game_grid = self.game_grid.clone();
        
        search_entry.connect_search_changed(move |entry| {
            let text = entry.text().to_string().to_lowercase();
            let all = all_games.borrow();
            let filtered: Vec<Game> = if text.is_empty() {
                all.clone()
            } else {
                all.iter()
                    .filter(|g| g.title.to_lowercase().contains(&text))
                    .cloned()
                    .collect()
            };
            *games.borrow_mut() = filtered;
            Self::update_game_grid(&game_grid, &games.borrow());
        });
    }

    fn setup_console_list(&self) {
        for console in &self.config.consoles {
            let row = gtk4::Label::new(Some(&console.name));
            row.set_halign(gtk4::Align::Start);
            row.set_margin_top(8);
            row.set_margin_bottom(8);
            row.set_margin_start(12);
            row.set_margin_end(12);

            self.console_list.append(&row);
        }

        let current_console = self.current_console.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let config = self.config.clone();
        let conn = self.conn.clone();
        let game_grid = self.game_grid.clone();

        self.console_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let idx = row.index() as usize;
                if idx < config.consoles.len() {
                    let console = &config.consoles[idx];
                    *current_console.borrow_mut() = Some(console.id.clone());

                    if let Ok(loaded_games) = database::load_games(&conn.borrow(), Some(&console.id)) {
                        *all_games.borrow_mut() = loaded_games.clone();
                        *games.borrow_mut() = loaded_games;
                        Self::update_game_grid(&game_grid, &games.borrow());
                    }
                }
            }
        });

        if !self.config.consoles.is_empty() {
            self.console_list.select_row(self.console_list.row_at_index(0).as_ref());
        }
    }

    fn update_game_grid(grid: &gtk4::FlowBox, games: &[Game]) {
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }

        for game in games {
            let game_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);

            let frame = gtk4::Frame::new(None);
            frame.set_size_request(150, 150);

            if let Some(media) = game.media.iter().find(|m| m.kind == MediaKind::BoxArt) {
                if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&media.path, 150, 150, true) {
                    let picture = gtk4::Picture::for_pixbuf(&pixbuf);
                    frame.set_child(Some(&picture));
                } else {
                    let label = gtk4::Label::new(Some("No Art"));
                    label.add_css_class("dim-label");
                    frame.set_child(Some(&label));
                }
            } else {
                let label = gtk4::Label::new(Some("No Art"));
                label.add_css_class("dim-label");
                frame.set_child(Some(&label));
            }

            let title = gtk4::Label::new(Some(&game.title));
            title.set_wrap(true);
            title.set_max_width_chars(20);
            title.set_lines(2);
            title.set_ellipsize(gtk4::pango::EllipsizeMode::End);

            game_box.append(&frame);
            game_box.append(&title);

            grid.insert(&game_box, -1);
        }
    }

    fn setup_game_selection(&self) {
        let selected_game = self.selected_game.clone();

        self.game_grid.connect_child_activated(move |_, child| {
            let idx = child.index() as usize;
            *selected_game.borrow_mut() = Some(idx);
        });

        let selected_game2 = self.selected_game.clone();
        let game_grid = self.game_grid.clone();

        game_grid.connect_selected_children_changed(move |grid| {
            if let Some(child) = grid.selected_children().first() {
                let idx = child.index() as usize;
                *selected_game2.borrow_mut() = Some(idx);
            }
        });
    }


    fn setup_keyboard_navigation(&self) {
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

        let game_grid = self.game_grid.clone();
        let search_bar = self.search_bar.clone();
        let search_entry = self.search_entry.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let selected_game = self.selected_game.clone();
        let config = self.config.clone();
        let conn = self.conn.clone();
        let current_console = self.current_console.clone();

        key_controller.connect_key_pressed(move |_, key, _, _| {
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if let Some(idx) = *selected_game.borrow() {
                        let games = games.borrow();
                        if let Some(game) = games.get(idx) {
                            if let Some(profile) = Self::resolve_profile(&config, game) {
                                if launcher::launch_game(&profile, &game.rom).is_ok() {
                                    let _ = database::update_last_played(&conn.borrow(), &game.id);
                                }
                            }
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::r => {
                    if let Some(console_id) = current_console.borrow().clone() {
                        if let Some(console) = config.consoles.iter().find(|c| c.id == console_id) {
                            if let Ok(scanned) = scanner::scan_console(console) {
                                let ids: Vec<_> = scanned.iter().map(|g| g.id.clone()).collect();
                                for game in &scanned {
                                    let _ = database::upsert_game(&conn.borrow(), game);
                                }
                                let _ = database::remove_missing_games(&conn.borrow(), &ids);

                                if let Ok(loaded) = database::load_games(&conn.borrow(), Some(&console_id)) {
                                    *all_games.borrow_mut() = loaded.clone();
                                    *games.borrow_mut() = loaded;
                                    Self::update_game_grid(&game_grid, &games.borrow());
                                }
                            }
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Left | gdk::Key::h => {
                    let selected = game_grid.selected_children().first().cloned();
                    if let Some(selected) = selected {
                        let idx = selected.index();
                        if idx > 0 {
                            if let Some(prev) = game_grid.child_at_index(idx - 1) {
                                game_grid.select_child(&prev);
                                *selected_game.borrow_mut() = Some((idx - 1) as usize);
                            }
                        }
                    } else if let Some(first) = game_grid.child_at_index(0) {
                        game_grid.select_child(&first);
                        *selected_game.borrow_mut() = Some(0);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Right | gdk::Key::l => {
                    let selected = game_grid.selected_children().first().cloned();
                    if let Some(selected) = selected {
                        let idx = selected.index();
                        if let Some(next) = game_grid.child_at_index(idx + 1) {
                            game_grid.select_child(&next);
                            *selected_game.borrow_mut() = Some((idx + 1) as usize);
                        }
                    } else if let Some(first) = game_grid.child_at_index(0) {
                        game_grid.select_child(&first);
                        *selected_game.borrow_mut() = Some(0);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Up | gdk::Key::k => {
                    let selected = game_grid.selected_children().first().cloned();
                    if let Some(selected) = selected {
                        let idx = selected.index();
                        if idx >= 6 {
                            if let Some(prev) = game_grid.child_at_index(idx - 6) {
                                game_grid.select_child(&prev);
                                *selected_game.borrow_mut() = Some((idx - 6) as usize);
                            }
                        }
                    } else if let Some(first) = game_grid.child_at_index(0) {
                        game_grid.select_child(&first);
                        *selected_game.borrow_mut() = Some(0);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Down | gdk::Key::j => {
                    let selected = game_grid.selected_children().first().cloned();
                    if let Some(selected) = selected {
                        let idx = selected.index();
                        if let Some(next) = game_grid.child_at_index(idx + 6) {
                            game_grid.select_child(&next);
                            *selected_game.borrow_mut() = Some((idx + 6) as usize);
                        }
                    } else if let Some(first) = game_grid.child_at_index(0) {
                        game_grid.select_child(&first);
                        *selected_game.borrow_mut() = Some(0);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Tab => {
                    game_grid.grab_focus();
                    if game_grid.selected_children().is_empty() {
                        if let Some(first) = game_grid.child_at_index(0) {
                            game_grid.select_child(&first);
                            *selected_game.borrow_mut() = Some(0);
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::slash => {
                    search_bar.set_search_mode(!search_bar.is_search_mode());
                    if search_bar.is_search_mode() {
                        search_entry.grab_focus();
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });

        self.window.add_controller(key_controller);
    }

    fn resolve_profile(config: &Config, game: &Game) -> Option<crate::types::EmulatorProfile> {
        if let Some(profile_id) = &game.profile {
            config.profiles.iter().find(|p| p.id() == profile_id).cloned()
        } else {
            let console = config.consoles.iter().find(|c| c.id == game.console)?;
            config.profiles.iter().find(|p| p.id() == &console.profile).cloned()
        }
    }

    pub fn show(&self) {
        self.window.present();
    }
}
