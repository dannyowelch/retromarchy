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
    conn: Connection,
    window: adw::ApplicationWindow,
    console_list: gtk4::ListBox,
    game_grid: gtk4::FlowBox,
    detail_title: gtk4::Label,
    detail_path: gtk4::Label,
    detail_console: gtk4::Label,
    detail_crc: gtk4::Label,
    current_console: Rc<RefCell<Option<String>>>,
    games: Rc<RefCell<Vec<Game>>>,
    selected_game: Rc<RefCell<Option<usize>>>,
}

impl App {
    pub fn new(app: &adw::Application, config: Config, conn: Connection) -> Result<Self> {
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

        let detail_pane = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        detail_pane.set_width_request(250);
        detail_pane.set_margin_top(12);
        detail_pane.set_margin_bottom(12);
        detail_pane.set_margin_start(12);
        detail_pane.set_margin_end(12);

        let detail_title = gtk4::Label::new(Some("Select a game"));
        detail_title.set_wrap(true);
        detail_title.set_halign(gtk4::Align::Start);
        detail_title.add_css_class("title-2");

        let detail_console = gtk4::Label::new(None);
        detail_console.set_halign(gtk4::Align::Start);
        detail_console.add_css_class("dim-label");

        let detail_path = gtk4::Label::new(None);
        detail_path.set_wrap(true);
        detail_path.set_halign(gtk4::Align::Start);
        detail_path.add_css_class("caption");

        let detail_crc = gtk4::Label::new(None);
        detail_crc.set_halign(gtk4::Align::Start);
        detail_crc.add_css_class("caption");

        detail_pane.append(&detail_title);
        detail_pane.append(&detail_console);
        detail_pane.append(&detail_path);
        detail_pane.append(&detail_crc);

        main_box.append(&sidebar);
        main_box.append(&center_box);
        main_box.append(&detail_pane);

        window.set_content(Some(&main_box));

        let app_instance = Self {
            config,
            conn,
            window: window.clone(),
            console_list: console_list.clone(),
            game_grid: game_grid.clone(),
            detail_title: detail_title.clone(),
            detail_path: detail_path.clone(),
            detail_console: detail_console.clone(),
            detail_crc: detail_crc.clone(),
            current_console: Rc::new(RefCell::new(None)),
            games: Rc::new(RefCell::new(Vec::new())),
            selected_game: Rc::new(RefCell::new(None)),
        };

        app_instance.setup_console_list();
        app_instance.setup_keyboard_navigation();
        app_instance.setup_game_selection();

        Ok(app_instance)
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
        let config = self.config.clone();
        let conn_ptr = &self.conn as *const Connection as usize;
        let game_grid = self.game_grid.clone();

        self.console_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let idx = row.index() as usize;
                if idx < config.consoles.len() {
                    let console = &config.consoles[idx];
                    *current_console.borrow_mut() = Some(console.id.clone());

                    let conn = unsafe { &*(conn_ptr as *const Connection) };
                    if let Ok(loaded_games) = database::load_games(conn, Some(&console.id)) {
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
        let games = self.games.clone();
        let selected_game = self.selected_game.clone();
        let detail_title = self.detail_title.clone();
        let detail_path = self.detail_path.clone();
        let detail_console = self.detail_console.clone();
        let detail_crc = self.detail_crc.clone();
        let config = self.config.clone();

        self.game_grid.connect_child_activated(move |_, child| {
            let idx = child.index() as usize;
            *selected_game.borrow_mut() = Some(idx);

            let games = games.borrow();
            if let Some(game) = games.get(idx) {
                detail_title.set_text(&game.title);
                detail_path.set_text(&format!("ROM: {}", game.rom.display()));

                if let Some(console) = config.consoles.iter().find(|c| c.id == game.console) {
                    detail_console.set_text(&format!("Console: {}", console.name));
                }

                if let Some(crc) = game.crc32 {
                    detail_crc.set_text(&format!("CRC32: {:08x}", crc));
                } else {
                    detail_crc.set_text("");
                }
            }
        });
    }

    fn setup_keyboard_navigation(&self) {
        let key_controller = gtk4::EventControllerKey::new();

        let game_grid = self.game_grid.clone();
        let games = self.games.clone();
        let selected_game = self.selected_game.clone();
        let config = self.config.clone();
        let conn_ptr = &self.conn as *const Connection as usize;
        let current_console = self.current_console.clone();

        key_controller.connect_key_pressed(move |_, key, _, _| {
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if let Some(idx) = *selected_game.borrow() {
                        let games = games.borrow();
                        if let Some(game) = games.get(idx) {
                            if let Some(profile) = Self::resolve_profile(&config, game) {
                                let conn = unsafe { &*(conn_ptr as *const Connection) };
                                if launcher::launch_game(&profile, &game.rom).is_ok() {
                                    let _ = database::update_last_played(conn, &game.id);
                                }
                            }
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::r => {
                    if let Some(console_id) = current_console.borrow().clone() {
                        if let Some(console) = config.consoles.iter().find(|c| c.id == console_id) {
                            let conn = unsafe { &*(conn_ptr as *const Connection) };
                            if let Ok(scanned) = scanner::scan_console(console) {
                                let ids: Vec<_> = scanned.iter().map(|g| g.id.clone()).collect();
                                for game in scanned {
                                    let _ = database::upsert_game(conn, &game);
                                }
                                let _ = database::remove_missing_games(conn, &ids);

                                if let Ok(loaded) = database::load_games(conn, Some(&console_id)) {
                                    *games.borrow_mut() = loaded;
                                    Self::update_game_grid(&game_grid, &games.borrow());
                                }
                            }
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Left | gdk::Key::h => {
                    if let Some(selected) = game_grid.selected_children().first() {
                        let idx = selected.index();
                        if idx > 0 {
                            if let Some(prev) = game_grid.child_at_index(idx - 1) {
                                game_grid.select_child(&prev);
                            }
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Right | gdk::Key::l => {
                    if let Some(selected) = game_grid.selected_children().first() {
                        let idx = selected.index();
                        if let Some(next) = game_grid.child_at_index(idx + 1) {
                            game_grid.select_child(&next);
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Up | gdk::Key::k => {
                    if let Some(selected) = game_grid.selected_children().first() {
                        let idx = selected.index();
                        if idx >= 6 {
                            if let Some(prev) = game_grid.child_at_index(idx - 6) {
                                game_grid.select_child(&prev);
                            }
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Down | gdk::Key::j => {
                    if let Some(selected) = game_grid.selected_children().first() {
                        let idx = selected.index();
                        if let Some(next) = game_grid.child_at_index(idx + 6) {
                            game_grid.select_child(&next);
                        }
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
