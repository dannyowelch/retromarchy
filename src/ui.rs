use crate::config::Config;
use crate::database;
use crate::launcher;
use crate::scanner;
use crate::scraper::{self, pick_kind, present_kinds};
use crate::types::{Game, GridArt, Media, MediaKind};
use anyhow::Result;
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::prelude::*;
use gtk4::{gdk, glib};
use libadwaita as adw;
use libadwaita::prelude::AdwApplicationWindowExt;
use rusqlite::Connection;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

pub struct App {
    config: Rc<RefCell<Config>>,
    conn: Rc<RefCell<Connection>>,
    window: adw::ApplicationWindow,
    console_list: gtk4::ListBox,
    game_grid: gtk4::FlowBox,
    search_bar: gtk4::SearchBar,
    search_entry: gtk4::SearchEntry,
    detail_pane: gtk4::ScrolledWindow,
    detail_content: gtk4::Box,
    current_console: Rc<RefCell<Option<String>>>,
    games: Rc<RefCell<Vec<Game>>>,
    all_games: Rc<RefCell<Vec<Game>>>,
    selected_game: Rc<RefCell<Option<usize>>>,
    details_visible: Rc<RefCell<bool>>,
    theme_css_provider: gtk4::CssProvider,
    center_stack: gtk4::Stack,
    grid_art_combo: gtk4::ComboBoxText,
    status_label: gtk4::Label,
    scrape_running: Rc<RefCell<bool>>,
    updating_grid_art: Rc<RefCell<bool>>,
}

impl App {
    pub fn new(app: &adw::Application, config: Config, conn: Rc<RefCell<Connection>>) -> Result<Self> {
        let base_css_provider = gtk4::CssProvider::new();
        base_css_provider.load_from_data(
            "flowboxchild:selected { 
                background: alpha(@accent_bg_color, 0.3); 
                border-radius: 6px; 
                border: 2px solid @accent_color;
             }
             .navigation-sidebar row:selected { background: @accent_bg_color; }
             flowboxchild:focus { outline: 2px solid @accent_color; outline-offset: 2px; }
             .console-subtitle { opacity: 0.65; font-size: 0.9em; }
             .rom-tile { background: #2c2c2c; border-radius: 8px; padding: 8px; }
             .rom-art { background: #444444; color: #f2f2f2; }
             .empty-state { background: transparent; }
             .scrape-status { padding: 6px 12px; background: alpha(@accent_bg_color, 0.35); }"
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().expect("Could not get default display"),
            &base_css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let theme_css_provider = gtk4::CssProvider::new();

        let header_bar = adw::HeaderBar::new();

        let import_button = gtk4::Button::with_label("Import ROMs");
        import_button.add_css_class("flat");
        header_bar.pack_start(&import_button);

        let emulator_button = gtk4::Button::with_label("Manage Emulators");
        emulator_button.add_css_class("flat");
        header_bar.pack_start(&emulator_button);

        let scraper_button = gtk4::Button::with_label("Scraper");
        scraper_button.add_css_class("flat");
        scraper_button.set_tooltip_text(Some("Scraper settings (Ctrl+G)"));
        header_bar.pack_start(&scraper_button);

        let scrape_button = gtk4::Button::with_label("Scrape");
        scrape_button.add_css_class("flat");
        scrape_button.set_tooltip_text(Some("Scrape artwork for the selected game (S)"));
        header_bar.pack_start(&scrape_button);

        let scrape_missing_button = gtk4::Button::with_label("Scrape Missing");
        scrape_missing_button.add_css_class("flat");
        scrape_missing_button.set_tooltip_text(Some("Scrape missing artwork for this system (Shift+S)"));
        header_bar.pack_start(&scrape_missing_button);

        let grid_art_combo = gtk4::ComboBoxText::new();
        grid_art_combo.append(Some(GridArt::BoxArt.as_str()), GridArt::BoxArt.label());
        grid_art_combo.append(Some(GridArt::TitleScreen.as_str()), GridArt::TitleScreen.label());
        grid_art_combo.append(Some(GridArt::Screenshot.as_str()), GridArt::Screenshot.label());
        grid_art_combo.set_active_id(Some(GridArt::BoxArt.as_str()));
        grid_art_combo.set_tooltip_text(Some("Grid artwork for this system (1 box, 2 title, 3 screenshot)"));

        let details_toggle = gtk4::ToggleButton::builder()
            .icon_name("sidebar-show-right-symbolic")
            .tooltip_text("Toggle Details Panel")
            .build();
        details_toggle.set_active(config.details_visible);
        header_bar.pack_end(&details_toggle);

        let theme_toggle = gtk4::Button::builder()
            .icon_name("weather-clear-night-symbolic")
            .tooltip_text("Toggle Theme")
            .build();
        header_bar.pack_end(&theme_toggle);
        header_bar.pack_end(&grid_art_combo);

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Retromarchy")
            .default_width(1200)
            .default_height(700)
            .build();

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.append(&header_bar);

        let status_label = gtk4::Label::new(None);
        status_label.set_widget_name("scrape-status");
        status_label.add_css_class("scrape-status");
        status_label.set_halign(gtk4::Align::Fill);
        status_label.set_xalign(0.0);
        status_label.set_wrap(true);
        status_label.set_margin_start(12);
        status_label.set_margin_end(12);
        status_label.set_margin_top(6);
        status_label.set_margin_bottom(6);
        status_label.set_visible(false);
        vbox.append(&status_label);

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

        let center_stack = gtk4::Stack::new();
        center_stack.set_vexpand(true);
        center_stack.add_named(&scrolled, Some("grid"));

        let empty = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        empty.set_valign(gtk4::Align::Center);
        empty.set_halign(gtk4::Align::Center);
        empty.set_vexpand(true);
        empty.set_margin_start(24);
        empty.set_margin_end(24);
        empty.add_css_class("empty-state");
        let empty_title = gtk4::Label::new(Some("No games found"));
        empty_title.add_css_class("title-1");
        let empty_copy = gtk4::Label::new(Some(
            "Import a ROM folder to add systems to the sidebar and scan games. Only paths are stored.",
        ));
        empty_copy.set_wrap(true);
        empty_copy.set_max_width_chars(42);
        empty_copy.set_justify(gtk4::Justification::Center);
        empty_copy.add_css_class("dim-label");
        let empty_import = gtk4::Button::with_label("Import ROMs");
        empty_import.add_css_class("suggested-action");
        empty_import.add_css_class("pill");
        let empty_emulators = gtk4::Button::with_label("Manage Emulators");
        empty_emulators.add_css_class("pill");
        empty.append(&empty_title);
        empty.append(&empty_copy);
        empty.append(&empty_import);
        empty.append(&empty_emulators);
        center_stack.add_named(&empty, Some("empty"));
        center_stack.set_visible_child_name("empty");
        center_box.append(&center_stack);

        let detail_pane = gtk4::ScrolledWindow::builder()
            .width_request(280)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();

        let detail_content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        detail_content.set_margin_top(12);
        detail_content.set_margin_bottom(12);
        detail_content.set_margin_start(12);
        detail_content.set_margin_end(12);
        detail_pane.set_child(Some(&detail_content));

        main_box.append(&sidebar);
        main_box.append(&center_box);
        main_box.append(&detail_pane);

        vbox.append(&main_box);
        window.set_content(Some(&vbox));

        let config_rc = Rc::new(RefCell::new(config.clone()));
        let details_visible = Rc::new(RefCell::new(config.details_visible));
        detail_pane.set_visible(config.details_visible);

        let app_instance = Self {
            config: config_rc.clone(),
            conn,
            window: window.clone(),
            console_list: console_list.clone(),
            game_grid: game_grid.clone(),
            search_bar: search_bar.clone(),
            search_entry: search_entry.clone(),
            detail_pane: detail_pane.clone(),
            detail_content: detail_content.clone(),
            current_console: Rc::new(RefCell::new(None)),
            games: Rc::new(RefCell::new(Vec::new())),
            all_games: Rc::new(RefCell::new(Vec::new())),
            selected_game: Rc::new(RefCell::new(None)),
            details_visible,
            theme_css_provider: theme_css_provider.clone(),
            center_stack: center_stack.clone(),
            grid_art_combo: grid_art_combo.clone(),
            status_label: status_label.clone(),
            scrape_running: Rc::new(RefCell::new(false)),
            updating_grid_art: Rc::new(RefCell::new(false)),
        };

        app_instance.apply_theme(&config.theme);
        app_instance.setup_console_list();
        app_instance.setup_search();
        app_instance.setup_keyboard_navigation();
        app_instance.setup_game_selection();
        app_instance.setup_details_toggle(details_toggle);
        app_instance.setup_theme_toggle(theme_toggle);
        app_instance.wire_library_actions(&import_button, &emulator_button, &empty_import, &empty_emulators);
        app_instance.wire_scrape_actions(&scraper_button, &scrape_button, &scrape_missing_button);
        app_instance.setup_grid_art();

        Ok(app_instance)
    }
    
    fn setup_search(&self) {
        let search_entry = self.search_entry.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let game_grid = self.game_grid.clone();
        let center_stack = self.center_stack.clone();
        let config = self.config.clone();

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
            Self::update_game_grid(&game_grid, &center_stack, &games.borrow(), &config.borrow());
        });
    }

    fn apply_theme(&self, theme: &str) {
        if theme == "launchbox" {
            self.theme_css_provider.load_from_data(
                "window {
                    background-color: #0a0a0a;
                    color: #e8e8e8;
                }
                .navigation-sidebar {
                    background-color: #1a1a1a;
                    color: #e8e8e8;
                }
                .navigation-sidebar row {
                    color: #e8e8e8;
                }
                .navigation-sidebar row:selected {
                    background-color: #2a2a2a;
                }
                scrolledwindow {
                    background-color: #0a0a0a;
                }
                flowbox {
                    background-color: #0a0a0a;
                }
                flowboxchild {
                    background-color: transparent;
                }
                flowboxchild:selected {
                    background: rgba(100, 100, 100, 0.4);
                    border: 2px solid #6ab0ff;
                }
                label {
                    color: #e8e8e8;
                }
                .dim-label {
                    color: #999999;
                }
                .console-subtitle {
                    color: #999999;
                }
                entry {
                    background-color: #1a1a1a;
                    color: #e8e8e8;
                }
                headerbar {
                    background-color: #1a1a1a;
                    color: #e8e8e8;
                }
                button {
                    background-color: #2a2a2a;
                    color: #e8e8e8;
                }
                button:hover {
                    background-color: #3a3a3a;
                }"
            );
            gtk4::style_context_add_provider_for_display(
                &gtk4::gdk::Display::default().expect("Could not get default display"),
                &self.theme_css_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
        } else {
            gtk4::style_context_remove_provider_for_display(
                &gtk4::gdk::Display::default().expect("Could not get default display"),
                &self.theme_css_provider,
            );
        }
    }

    fn setup_details_toggle(&self, toggle: gtk4::ToggleButton) {
        let details_visible = self.details_visible.clone();
        let detail_pane = self.detail_pane.clone();
        let config = self.config.clone();

        toggle.connect_toggled(move |btn| {
            let visible = btn.is_active();
            detail_pane.set_visible(visible);
            *details_visible.borrow_mut() = visible;
            config.borrow_mut().details_visible = visible;
            let _ = crate::config::save_config(&config.borrow());
        });
    }

    fn setup_theme_toggle(&self, button: gtk4::Button) {
        let config = self.config.clone();
        let theme_css_provider = self.theme_css_provider.clone();

        button.connect_clicked(move |_| {
            let mut cfg = config.borrow_mut();
            let new_theme = if cfg.theme == "system" {
                "launchbox"
            } else {
                "system"
            };
            cfg.theme = new_theme.to_string();
            let _ = crate::config::save_config(&cfg);
            
            if new_theme == "launchbox" {
                theme_css_provider.load_from_data(
                    "window {
                        background-color: #0a0a0a;
                        color: #e8e8e8;
                    }
                    .navigation-sidebar {
                        background-color: #1a1a1a;
                        color: #e8e8e8;
                    }
                    .navigation-sidebar row {
                        color: #e8e8e8;
                    }
                    .navigation-sidebar row:selected {
                        background-color: #2a2a2a;
                    }
                    scrolledwindow {
                        background-color: #0a0a0a;
                    }
                    flowbox {
                        background-color: #0a0a0a;
                    }
                    flowboxchild {
                        background-color: transparent;
                    }
                    flowboxchild:selected {
                        background: rgba(100, 100, 100, 0.4);
                        border: 2px solid #6ab0ff;
                    }
                    label {
                        color: #e8e8e8;
                    }
                    .dim-label {
                        color: #999999;
                    }
                    .console-subtitle {
                        color: #999999;
                    }
                    entry {
                        background-color: #1a1a1a;
                        color: #e8e8e8;
                    }
                    headerbar {
                        background-color: #1a1a1a;
                        color: #e8e8e8;
                    }
                    button {
                        background-color: #2a2a2a;
                        color: #e8e8e8;
                    }
                    button:hover {
                        background-color: #3a3a3a;
                    }"
                );
                gtk4::style_context_add_provider_for_display(
                    &gtk4::gdk::Display::default().expect("Could not get default display"),
                    &theme_css_provider,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                );
            } else {
                gtk4::style_context_remove_provider_for_display(
                    &gtk4::gdk::Display::default().expect("Could not get default display"),
                    &theme_css_provider,
                );
            }
        });
    }

    fn setup_console_list(&self) {
        for console in &self.config.borrow().consoles {
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
        let center_stack = self.center_stack.clone();
        let selected_game = self.selected_game.clone();
        let detail_content = self.detail_content.clone();
        let grid_art_combo = self.grid_art_combo.clone();
        let updating_grid_art = self.updating_grid_art.clone();

        self.console_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let idx = row.index() as usize;
                if idx < config.borrow().consoles.len() {
                    let console = &config.borrow().consoles[idx];
                    *current_console.borrow_mut() = Some(console.id.clone());
                    *selected_game.borrow_mut() = None;

                    if let Ok(loaded_games) = database::load_games(&conn.borrow(), Some(&console.id)) {
                        *all_games.borrow_mut() = loaded_games.clone();
                        *games.borrow_mut() = loaded_games;
                        Self::update_game_grid(&game_grid, &center_stack, &games.borrow(), &config.borrow());
                    }
                    Self::sync_grid_art_combo(&grid_art_combo, &updating_grid_art, console.grid_art);

                    Self::update_console_details(&detail_content, &console.id, &conn.borrow(), &config.borrow());
                }
            }
        });

        if !self.config.borrow().consoles.is_empty() {
            self.console_list.select_row(self.console_list.row_at_index(0).as_ref());
        }
    }

    fn wire_library_actions(
        &self,
        import_button: &gtk4::Button,
        emulator_button: &gtk4::Button,
        empty_import: &gtk4::Button,
        empty_emulators: &gtk4::Button,
    ) {
        let open_import = {
            let window = self.window.clone();
            let config = self.config.clone();
            let conn = self.conn.clone();
            let done = Self::refresh_action(
                self.console_list.clone(),
                self.config.clone(),
                self.conn.clone(),
                self.games.clone(),
                self.all_games.clone(),
                self.game_grid.clone(),
                self.center_stack.clone(),
                self.current_console.clone(),
                self.detail_content.clone(),
            );
            move || {
                crate::dialogs::open_import(&window, config.clone(), conn.clone(), done.clone());
            }
        };
        let open_emulators = {
            let window = self.window.clone();
            let config = self.config.clone();
            let done = Self::refresh_action(
                self.console_list.clone(),
                self.config.clone(),
                self.conn.clone(),
                self.games.clone(),
                self.all_games.clone(),
                self.game_grid.clone(),
                self.center_stack.clone(),
                self.current_console.clone(),
                self.detail_content.clone(),
            );
            move || {
                crate::dialogs::open_emulators(&window, config.clone(), done.clone());
            }
        };

        let open_import = Rc::new(open_import);
        let open_emulators = Rc::new(open_emulators);

        let import_action = open_import.clone();
        import_button.connect_clicked(move |_| import_action());
        let empty_action = open_import;
        empty_import.connect_clicked(move |_| empty_action());

        let emulator_action = open_emulators.clone();
        emulator_button.connect_clicked(move |_| emulator_action());
        empty_emulators.connect_clicked(move |_| open_emulators());
    }

    fn refresh_action(
        console_list: gtk4::ListBox,
        config: Rc<RefCell<Config>>,
        _conn: Rc<RefCell<Connection>>,
        games: Rc<RefCell<Vec<Game>>>,
        all_games: Rc<RefCell<Vec<Game>>>,
        game_grid: gtk4::FlowBox,
        center_stack: gtk4::Stack,
        current_console: Rc<RefCell<Option<String>>>,
        detail_content: gtk4::Box,
    ) -> Rc<dyn Fn()> {
        Rc::new(move || {
            while let Some(child) = console_list.first_child() {
                console_list.remove(&child);
            }
            for console in &config.borrow().consoles {
                let row = gtk4::Label::new(Some(&console.name));
                row.set_halign(gtk4::Align::Start);
                row.set_margin_top(8);
                row.set_margin_bottom(8);
                row.set_margin_start(12);
                row.set_margin_end(12);
                console_list.append(&row);
            }
            if let Some(row) = console_list.row_at_index(0) {
                console_list.select_row(Some(&row));
            } else {
                *current_console.borrow_mut() = None;
                *games.borrow_mut() = Vec::new();
                *all_games.borrow_mut() = Vec::new();
                Self::update_game_grid(&game_grid, &center_stack, &[], &config.borrow());
                while let Some(child) = detail_content.first_child() {
                    detail_content.remove(&child);
                }
            }
        })
    }

    fn update_game_grid(grid: &gtk4::FlowBox, stack: &gtk4::Stack, games: &[Game], config: &Config) {
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }

        for game in games {
            let game_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            game_box.add_css_class("rom-tile");

            let frame = gtk4::Frame::new(None);
            frame.add_css_class("rom-art");
            frame.set_size_request(150, 150);
            Self::set_tile_art(&frame, game, config);

            let title = gtk4::Label::new(Some(&game.title));
            title.set_wrap(true);
            title.set_max_width_chars(20);
            title.set_lines(2);
            title.set_ellipsize(gtk4::pango::EllipsizeMode::End);

            if let Some(console) = config.consoles.iter().find(|c| c.id == game.console) {
                let console_label = gtk4::Label::new(Some(&console.name));
                console_label.set_wrap(false);
                console_label.set_max_width_chars(20);
                console_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
                console_label.add_css_class("caption");
                console_label.add_css_class("console-subtitle");

                game_box.append(&frame);
                game_box.append(&title);
                game_box.append(&console_label);
            } else {
                game_box.append(&frame);
                game_box.append(&title);
            }

            grid.insert(&game_box, -1);
        }

        if games.is_empty() {
            stack.set_visible_child_name("empty");
        } else {
            stack.set_visible_child_name("grid");
        }
    }

    fn update_console_details(detail_content: &gtk4::Box, console_id: &str, conn: &Connection, config: &Config) {
        while let Some(child) = detail_content.first_child() {
            detail_content.remove(&child);
        }

        if let Some(console_cfg) = config.consoles.iter().find(|c| c.id == console_id) {
            let name_label = gtk4::Label::new(Some(&console_cfg.name));
            name_label.set_wrap(true);
            name_label.set_halign(gtk4::Align::Start);
            name_label.add_css_class("title-2");
            detail_content.append(&name_label);

            if let Ok(metadata_list) = crate::config::load_console_metadata() {
                if let Some(meta) = metadata_list.iter().find(|m| m.id == console_id) {
                    let manufacturer_label = gtk4::Label::new(Some(&format!("Manufacturer: {}", meta.manufacturer)));
                    manufacturer_label.set_halign(gtk4::Align::Start);
                    manufacturer_label.add_css_class("caption");
                    detail_content.append(&manufacturer_label);

                    let year_label = gtk4::Label::new(Some(&format!("Year: {}", meta.year)));
                    year_label.set_halign(gtk4::Align::Start);
                    year_label.add_css_class("caption");
                    detail_content.append(&year_label);

                    let desc_label = gtk4::Label::new(Some(&meta.description));
                    desc_label.set_wrap(true);
                    desc_label.set_halign(gtk4::Align::Start);
                    desc_label.add_css_class("caption");
                    desc_label.set_margin_top(8);
                    detail_content.append(&desc_label);
                }
            }

            let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            separator.set_margin_top(12);
            separator.set_margin_bottom(12);
            detail_content.append(&separator);

            let stats_header = gtk4::Label::new(Some("Library Statistics"));
            stats_header.set_halign(gtk4::Align::Start);
            stats_header.add_css_class("title-4");
            detail_content.append(&stats_header);

            let console_id_string = console_id.to_string();
            if let Ok(stats) = database::get_library_stats(conn, &console_id_string) {
                let total_label = gtk4::Label::new(Some(&format!("Total games: {}", stats.total_games)));
                total_label.set_halign(gtk4::Align::Start);
                total_label.add_css_class("caption");
                detail_content.append(&total_label);

                if let Some(date) = stats.last_played_date {
                    let formatted = date.format("%Y-%m-%d %H:%M").to_string();
                    let last_played_label = gtk4::Label::new(Some(&format!("Last played: {}", formatted)));
                    last_played_label.set_halign(gtk4::Align::Start);
                    last_played_label.add_css_class("caption");
                    detail_content.append(&last_played_label);

                    if let Some(game_title) = stats.last_played_game {
                        let last_game_label = gtk4::Label::new(Some(&format!("  → {}", game_title)));
                        last_game_label.set_halign(gtk4::Align::Start);
                        last_game_label.add_css_class("caption");
                        last_game_label.add_css_class("dim-label");
                        detail_content.append(&last_game_label);
                    }
                }

                if stats.total_play_count > 0 {
                    let count_label = gtk4::Label::new(Some(&format!("Total plays: {}", stats.total_play_count)));
                    count_label.set_halign(gtk4::Align::Start);
                    count_label.add_css_class("caption");
                    detail_content.append(&count_label);
                }

                if stats.total_play_time > 0 {
                    let hours = stats.total_play_time / 3600;
                    let minutes = (stats.total_play_time % 3600) / 60;
                    let time_str = if hours > 0 {
                        format!("Total play time: {}h {}m", hours, minutes)
                    } else {
                        format!("Total play time: {}m", minutes)
                    };
                    let time_label = gtk4::Label::new(Some(&time_str));
                    time_label.set_halign(gtk4::Align::Start);
                    time_label.add_css_class("caption");
                    detail_content.append(&time_label);
                }

                if let Some(most_played) = stats.most_played_game {
                    let most_played_label = gtk4::Label::new(Some(&format!("Most played: {}", most_played)));
                    most_played_label.set_halign(gtk4::Align::Start);
                    most_played_label.add_css_class("caption");
                    detail_content.append(&most_played_label);

                    let count_label = gtk4::Label::new(Some(&format!("  → {} plays", stats.most_played_count)));
                    count_label.set_halign(gtk4::Align::Start);
                    count_label.add_css_class("caption");
                    count_label.add_css_class("dim-label");
                    detail_content.append(&count_label);
                }
            }
        }
    }

    fn update_game_details(detail_content: &gtk4::Box, game: &Game, config: &Config, conn: &Rc<RefCell<Connection>>) {
        while let Some(child) = detail_content.first_child() {
            detail_content.remove(&child);
        }

        if let Some(media) = game.media.iter().find(|m| m.kind == MediaKind::BoxArt && m.path.is_file()) {
            if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&media.path, 250, 180, true) {
                let picture = gtk4::Picture::for_pixbuf(&pixbuf);
                picture.set_can_shrink(true);
                picture.set_height_request(180);
                detail_content.append(&picture);
            }
        }

        Self::append_detail_media(detail_content, game, MediaKind::TitleScreen, "Title screen");
        Self::append_detail_media(detail_content, game, MediaKind::Screenshot, "Screenshot");

        let title_label = gtk4::Label::new(Some(&game.title));
        title_label.set_wrap(true);
        title_label.set_halign(gtk4::Align::Start);
        title_label.add_css_class("title-2");
        detail_content.append(&title_label);

        if let Some(console) = config.consoles.iter().find(|c| c.id == game.console) {
            let console_label = gtk4::Label::new(Some(&format!("Console: {}", console.name)));
            console_label.set_halign(gtk4::Align::Start);
            console_label.add_css_class("caption");
            detail_content.append(&console_label);
        }

        let rom_label = gtk4::Label::new(Some(&format!("ROM: {}", game.rom.display())));
        rom_label.set_wrap(true);
        rom_label.set_halign(gtk4::Align::Start);
        rom_label.add_css_class("caption");
        detail_content.append(&rom_label);

        if let Some(crc) = game.crc32 {
            let crc_label = gtk4::Label::new(Some(&format!("CRC32: {:08x}", crc)));
            crc_label.set_halign(gtk4::Align::Start);
            crc_label.add_css_class("caption");
            detail_content.append(&crc_label);
        }

        let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        separator.set_margin_top(8);
        separator.set_margin_bottom(8);
        detail_content.append(&separator);

        let play_button = gtk4::Button::with_label("Play");
        play_button.set_halign(gtk4::Align::Fill);
        
        let game_clone = game.clone();
        let config_clone = config.clone();
        let conn_clone = conn.clone();
        let detail_content_clone = detail_content.clone();

        play_button.connect_clicked(move |btn| {
            let Some(profile) = Self::resolve_profile(&config_clone, &game_clone) else {
                if let Some(root) = btn.root() {
                    if let Some(win) = root.downcast_ref::<adw::ApplicationWindow>() {
                        Self::alert_missing_profile(win);
                    }
                }
                return;
            };
            if let Ok(mut child) = launcher::launch_game_tracked(&profile, &game_clone.rom) {
                    let game_id = game_clone.id.clone();
                    let game_console = game_clone.console.clone();
                    let conn_for_update = conn_clone.clone();
                    let detail_for_update = detail_content_clone.clone();
                    let config_for_update = config_clone.clone();
                    let start_time = Instant::now();

                    let (sender, receiver) = std::sync::mpsc::channel();
                    
                    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                        if let Ok((game_id, game_console, elapsed)) = receiver.try_recv() {
                            let _ = database::increment_play_stats(&conn_for_update.borrow(), &game_id, elapsed);
                            let _ = database::update_last_played(&conn_for_update.borrow(), &game_id);
                            
                            if let Ok(updated_games) = database::load_games(&conn_for_update.borrow(), Some(&game_console)) {
                                if let Some(updated_game) = updated_games.iter().find(|g| g.id == game_id) {
                                    Self::update_game_details(&detail_for_update, updated_game, &config_for_update, &conn_for_update);
                                }
                            }
                            glib::ControlFlow::Break
                        } else {
                            glib::ControlFlow::Continue
                        }
                    });

                    std::thread::spawn(move || {
                        let _ = child.wait();
                        let elapsed = start_time.elapsed().as_secs() as u32;
                        let _ = sender.send((game_id, game_console, elapsed));
                    });
                }
        });
        
        detail_content.append(&play_button);

        if let Some(dt) = game.last_played {
            let formatted = dt.format("%Y-%m-%d %H:%M").to_string();
            let last_played_label = gtk4::Label::new(Some(&format!("Last played: {}", formatted)));
            last_played_label.set_halign(gtk4::Align::Start);
            last_played_label.add_css_class("caption");
            detail_content.append(&last_played_label);
        }

        if game.play_count > 0 {
            let count_label = gtk4::Label::new(Some(&format!("Play count: {}", game.play_count)));
            count_label.set_halign(gtk4::Align::Start);
            count_label.add_css_class("caption");
            detail_content.append(&count_label);
        }

        if game.play_time > 0 {
            let hours = game.play_time / 3600;
            let minutes = (game.play_time % 3600) / 60;
            let seconds = game.play_time % 60;
            let time_str = if hours > 0 {
                format!("Play time: {}h {}m", hours, minutes)
            } else if minutes > 0 {
                format!("Play time: {}m", minutes)
            } else {
                format!("Play time: {}s", seconds)
            };
            let time_label = gtk4::Label::new(Some(&time_str));
            time_label.set_halign(gtk4::Align::Start);
            time_label.add_css_class("caption");
            detail_content.append(&time_label);
        }
    }

    fn setup_game_selection(&self) {
        let selected_game = self.selected_game.clone();
        let games = self.games.clone();
        let detail_content = self.detail_content.clone();
        let config = self.config.clone();
        let conn = self.conn.clone();

        self.game_grid.connect_child_activated(move |_, child| {
            let idx = child.index() as usize;
            *selected_game.borrow_mut() = Some(idx);
            
            let games_borrow = games.borrow();
            if let Some(game) = games_borrow.get(idx) {
                Self::update_game_details(&detail_content, game, &config.borrow(), &conn);
            }
        });

        let selected_game2 = self.selected_game.clone();
        let game_grid = self.game_grid.clone();
        let games2 = self.games.clone();
        let detail_content2 = self.detail_content.clone();
        let config2 = self.config.clone();
        let conn2 = self.conn.clone();

        game_grid.connect_selected_children_changed(move |grid| {
            if let Some(child) = grid.selected_children().first() {
                let idx = child.index() as usize;
                *selected_game2.borrow_mut() = Some(idx);
                
                let games_borrow = games2.borrow();
                if let Some(game) = games_borrow.get(idx) {
                    Self::update_game_details(&detail_content2, game, &config2.borrow(), &conn2);
                }
            }
        });
    }

    fn setup_keyboard_navigation(&self) {
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Bubble);

        let window = self.window.clone();
        let game_grid = self.game_grid.clone();
        let search_bar = self.search_bar.clone();
        let search_entry = self.search_entry.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let selected_game = self.selected_game.clone();
        let config = self.config.clone();
        let conn = self.conn.clone();
        let current_console = self.current_console.clone();
        let detail_pane = self.detail_pane.clone();
        let details_visible = self.details_visible.clone();
        let detail_content = self.detail_content.clone();
        let theme_css_provider = self.theme_css_provider.clone();
        let center_stack = self.center_stack.clone();
        let console_list = self.console_list.clone();
        let status_label = self.status_label.clone();
        let scrape_running = self.scrape_running.clone();
        let grid_art_combo = self.grid_art_combo.clone();
        let updating_grid_art = self.updating_grid_art.clone();

        key_controller.connect_key_pressed(move |_, key, _, mods| {
            let focused = gtk4::prelude::RootExt::focus(&window);
            let search_has_focus = focused.as_ref().map_or(false, |w| {
                w.upcast_ref::<gtk4::Widget>() == search_entry.upcast_ref::<gtk4::Widget>() ||
                search_entry.is_ancestor(w)
            });
            let combo_has_focus = focused.as_ref().map_or(false, |w| {
                w.upcast_ref::<gtk4::Widget>() == grid_art_combo.upcast_ref::<gtk4::Widget>()
                    || grid_art_combo.is_ancestor(w)
            });
            
            if mods.contains(gdk::ModifierType::CONTROL_MASK) && !search_has_focus {
                if key == gdk::Key::g || key == gdk::Key::G {
                    crate::dialogs::open_scraper(&window, config.clone());
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::i || key == gdk::Key::I {
                    let done = Self::refresh_action(console_list.clone(), config.clone(), conn.clone(), games.clone(), all_games.clone(), game_grid.clone(), center_stack.clone(), current_console.clone(), detail_content.clone());
                    crate::dialogs::open_import(&window, config.clone(), conn.clone(), done);
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::e || key == gdk::Key::E || key == gdk::Key::m || key == gdk::Key::M {
                    let done = Self::refresh_action(console_list.clone(), config.clone(), conn.clone(), games.clone(), all_games.clone(), game_grid.clone(), center_stack.clone(), current_console.clone(), detail_content.clone());
                    crate::dialogs::open_emulators(&window, config.clone(), done);
                    return glib::Propagation::Stop;
                }
            }

            let update_details = |idx: usize| {
                let games = games.borrow();
                if let Some(game) = games.get(idx) {
                    Self::update_game_details(&detail_content, game, &config.borrow(), &conn);
                }
            };
            
            match key {
                gdk::Key::Escape => {
                    if search_bar.is_search_mode() {
                        search_bar.set_search_mode(false);
                        search_entry.set_text("");
                        let all = all_games.borrow();
                        *games.borrow_mut() = all.clone();
                        Self::update_game_grid(&game_grid, &center_stack, &games.borrow(), &config.borrow());
                    } else {
                        *selected_game.borrow_mut() = None;
                        game_grid.unselect_all();
                        if let Some(console_id) = current_console.borrow().as_ref() {
                            Self::update_console_details(&detail_content, console_id, &conn.borrow(), &config.borrow());
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if !search_has_focus {
                        if let Some(idx) = *selected_game.borrow() {
                            let games = games.borrow();
                            if let Some(game) = games.get(idx) {
                                let Some(profile) = Self::resolve_profile(&config.borrow(), game) else {
                                    Self::alert_missing_profile(&window);
                                    return glib::Propagation::Stop;
                                };
                                if let Ok(mut child) = launcher::launch_game_tracked(&profile, &game.rom) {
                                        let game_id = game.id.clone();
                                        let game_console = game.console.clone();
                                        let conn_for_update = conn.clone();
                                        let detail_for_update = detail_content.clone();
                                        let config_for_update = config.clone();
                                        let start_time = Instant::now();

                                        let (sender, receiver) = std::sync::mpsc::channel();
                                        
                                        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                                            if let Ok((game_id, game_console, elapsed)) = receiver.try_recv() {
                                                let _ = database::increment_play_stats(&conn_for_update.borrow(), &game_id, elapsed);
                                                let _ = database::update_last_played(&conn_for_update.borrow(), &game_id);
                                                
                                                if let Ok(updated_games) = database::load_games(&conn_for_update.borrow(), Some(&game_console)) {
                                                    if let Some(updated_game) = updated_games.iter().find(|g| g.id == game_id) {
                                                        Self::update_game_details(&detail_for_update, updated_game, &config_for_update.borrow(), &conn_for_update);
                                                    }
                                                }
                                                glib::ControlFlow::Break
                                            } else {
                                                glib::ControlFlow::Continue
                                            }
                                        });

                                        std::thread::spawn(move || {
                                            let _ = child.wait();
                                            let elapsed = start_time.elapsed().as_secs() as u32;
                                            let _ = sender.send((game_id, game_console, elapsed));
                                        });
                                    }
                                }
                            }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::s | gdk::Key::S => {
                    if !search_has_focus {
                        if mods.contains(gdk::ModifierType::SHIFT_MASK) || key == gdk::Key::S {
                            Self::request_scrape_missing(&status_label, &scrape_running, &config, &conn, &all_games, &games, &game_grid, &center_stack, &detail_content, &selected_game, &search_entry, &current_console);
                        } else {
                            Self::request_scrape_selected(&status_label, &scrape_running, &config, &conn, &games, &all_games, &game_grid, &center_stack, &detail_content, &selected_game, &search_entry);
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::_1 | gdk::Key::_2 | gdk::Key::_3 => {
                    if !search_has_focus {
                        let art = match key {
                            gdk::Key::_1 => GridArt::BoxArt,
                            gdk::Key::_2 => GridArt::TitleScreen,
                            _ => GridArt::Screenshot,
                        };
                        Self::set_current_grid_art(&config, &current_console, &grid_art_combo, &updating_grid_art, art);
                        Self::update_game_grid(&game_grid, &center_stack, &games.borrow(), &config.borrow());
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::r => {
                    if !search_has_focus {
                        if let Some(console_id) = current_console.borrow().clone() {
                            if let Some(console) = config.borrow().consoles.iter().find(|c| c.id == console_id) {
                                let console_clone = console.clone();
                                if let Ok(scanned) = scanner::scan_console(&console_clone) {
                                    let ids: Vec<_> = scanned.iter().map(|g| g.id.clone()).collect();
                                    for game in &scanned {
                                        let _ = database::upsert_game(&conn.borrow(), game);
                                    }
                                    let _ = database::remove_missing_games(&conn.borrow(), &console_id, &ids);

                                    if let Ok(loaded) = database::load_games(&conn.borrow(), Some(&console_id)) {
                                        *all_games.borrow_mut() = loaded.clone();
                                        *games.borrow_mut() = loaded;
                                        Self::update_game_grid(&game_grid, &center_stack, &games.borrow(), &config.borrow());
                                    }

                                    Self::update_console_details(&detail_content, &console_id, &conn.borrow(), &config.borrow());
                                }
                            }
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Left | gdk::Key::h => {
                    if combo_has_focus {
                        glib::Propagation::Proceed
                    } else if !search_has_focus {
                        let selected = game_grid.selected_children().first().cloned();
                        if let Some(selected) = selected {
                            let idx = selected.index();
                            if idx > 0 {
                                if let Some(prev) = game_grid.child_at_index(idx - 1) {
                                    game_grid.select_child(&prev);
                                    let new_idx = (idx - 1) as usize;
                                    *selected_game.borrow_mut() = Some(new_idx);
                                    update_details(new_idx);
                                }
                            }
                        } else if let Some(first) = game_grid.child_at_index(0) {
                            game_grid.select_child(&first);
                            *selected_game.borrow_mut() = Some(0);
                            update_details(0);
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Right | gdk::Key::l => {
                    if combo_has_focus {
                        glib::Propagation::Proceed
                    } else if !search_has_focus {
                        let selected = game_grid.selected_children().first().cloned();
                        if let Some(selected) = selected {
                            let idx = selected.index();
                            if let Some(next) = game_grid.child_at_index(idx + 1) {
                                game_grid.select_child(&next);
                                let new_idx = (idx + 1) as usize;
                                *selected_game.borrow_mut() = Some(new_idx);
                                update_details(new_idx);
                            }
                        } else if let Some(first) = game_grid.child_at_index(0) {
                            game_grid.select_child(&first);
                            *selected_game.borrow_mut() = Some(0);
                            update_details(0);
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Up | gdk::Key::k => {
                    if combo_has_focus {
                        glib::Propagation::Proceed
                    } else if !search_has_focus {
                        let selected = game_grid.selected_children().first().cloned();
                        if let Some(selected) = selected {
                            let idx = selected.index();
                            if idx >= 6 {
                                if let Some(prev) = game_grid.child_at_index(idx - 6) {
                                    game_grid.select_child(&prev);
                                    let new_idx = (idx - 6) as usize;
                                    *selected_game.borrow_mut() = Some(new_idx);
                                    update_details(new_idx);
                                }
                            }
                        } else if let Some(first) = game_grid.child_at_index(0) {
                            game_grid.select_child(&first);
                            *selected_game.borrow_mut() = Some(0);
                            update_details(0);
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Down | gdk::Key::j => {
                    if combo_has_focus {
                        glib::Propagation::Proceed
                    } else if !search_has_focus {
                        let selected = game_grid.selected_children().first().cloned();
                        if let Some(selected) = selected {
                            let idx = selected.index();
                            if let Some(next) = game_grid.child_at_index(idx + 6) {
                                game_grid.select_child(&next);
                                let new_idx = (idx + 6) as usize;
                                *selected_game.borrow_mut() = Some(new_idx);
                                update_details(new_idx);
                            }
                        } else if let Some(first) = game_grid.child_at_index(0) {
                            game_grid.select_child(&first);
                            *selected_game.borrow_mut() = Some(0);
                            update_details(0);
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Tab => {
                    game_grid.grab_focus();
                    if game_grid.selected_children().is_empty() {
                        if let Some(first) = game_grid.child_at_index(0) {
                            game_grid.select_child(&first);
                            *selected_game.borrow_mut() = Some(0);
                            update_details(0);
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
                gdk::Key::d => {
                    if !search_has_focus {
                        let visible = *details_visible.borrow();
                        detail_pane.set_visible(!visible);
                        *details_visible.borrow_mut() = !visible;
                        config.borrow_mut().details_visible = !visible;
                        let _ = crate::config::save_config(&config.borrow());
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::t => {
                    if !search_has_focus {
                        let mut cfg = config.borrow_mut();
                        let new_theme = if cfg.theme == "system" {
                            "launchbox"
                        } else {
                            "system"
                        };
                        cfg.theme = new_theme.to_string();
                        let _ = crate::config::save_config(&cfg);
                        
                        if new_theme == "launchbox" {
                            theme_css_provider.load_from_data(
                                "window {
                                    background-color: #0a0a0a;
                                    color: #e8e8e8;
                                }
                                .navigation-sidebar {
                                    background-color: #1a1a1a;
                                    color: #e8e8e8;
                                }
                                .navigation-sidebar row {
                                    color: #e8e8e8;
                                }
                                .navigation-sidebar row:selected {
                                    background-color: #2a2a2a;
                                }
                                scrolledwindow {
                                    background-color: #0a0a0a;
                                }
                                flowbox {
                                    background-color: #0a0a0a;
                                }
                                flowboxchild {
                                    background-color: transparent;
                                }
                                flowboxchild:selected {
                                    background: rgba(100, 100, 100, 0.4);
                                    border: 2px solid #6ab0ff;
                                }
                                label {
                                    color: #e8e8e8;
                                }
                                .dim-label {
                                    color: #999999;
                                }
                                .console-subtitle {
                                    color: #999999;
                                }
                                entry {
                                    background-color: #1a1a1a;
                                    color: #e8e8e8;
                                }
                                headerbar {
                                    background-color: #1a1a1a;
                                    color: #e8e8e8;
                                }
                                button {
                                    background-color: #2a2a2a;
                                    color: #e8e8e8;
                                }
                                button:hover {
                                    background-color: #3a3a3a;
                                }"
                            );
                            gtk4::style_context_add_provider_for_display(
                                &gtk4::gdk::Display::default().expect("Could not get default display"),
                                &theme_css_provider,
                                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                            );
                        } else {
                            gtk4::style_context_remove_provider_for_display(
                                &gtk4::gdk::Display::default().expect("Could not get default display"),
                                &theme_css_provider,
                            );
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                _ => glib::Propagation::Proceed,
            }
        });

        self.window.add_controller(key_controller);
    }

    fn resolve_profile(config: &Config, game: &Game) -> Option<crate::types::EmulatorProfile> {
        let from_game = game.profile.as_deref().filter(|id| !id.is_empty());
        if let Some(profile_id) = from_game {
            return config.profiles.iter().find(|p| p.id() == profile_id).cloned();
        }
        let console = config.consoles.iter().find(|c| c.id == game.console)?;
        let profile_id = console.profile.as_deref().filter(|id| !id.is_empty())?;
        config.profiles.iter().find(|p| p.id() == profile_id).cloned()
    }

    fn alert_missing_profile(window: &adw::ApplicationWindow) {
        let dialog = gtk4::MessageDialog::new(
            Some(window),
            gtk4::DialogFlags::MODAL,
            gtk4::MessageType::Info,
            gtk4::ButtonsType::Ok,
            "This system has no emulator profile. Use Manage Emulators to add one and assign it.",
        );
        dialog.set_title(Some("Configure an emulator"));
        dialog.connect_response(|dialog, _| dialog.close());
        dialog.present();
    }

    fn setup_grid_art(&self) {
        let config = self.config.clone();
        let current_console = self.current_console.clone();
        let game_grid = self.game_grid.clone();
        let center_stack = self.center_stack.clone();
        let games = self.games.clone();
        let updating = self.updating_grid_art.clone();
        self.grid_art_combo.connect_changed(move |combo| {
            if *updating.borrow() {
                return;
            }
            let Some(id) = combo.active_id() else { return };
            let Some(art) = GridArt::parse(id.as_str()) else { return };
            Self::set_current_grid_art(&config, &current_console, combo, &updating, art);
            Self::update_game_grid(&game_grid, &center_stack, &games.borrow(), &config.borrow());
        });
    }

    fn sync_grid_art_combo(combo: &gtk4::ComboBoxText, updating: &Rc<RefCell<bool>>, art: GridArt) {
        *updating.borrow_mut() = true;
        combo.set_active_id(Some(art.as_str()));
        *updating.borrow_mut() = false;
    }

    fn set_current_grid_art(
        config: &Rc<RefCell<crate::config::Config>>,
        current_console: &Rc<RefCell<Option<String>>>,
        combo: &gtk4::ComboBoxText,
        updating: &Rc<RefCell<bool>>,
        art: GridArt,
    ) {
        let Some(console_id) = current_console.borrow().clone() else { return };
        {
            let mut cfg = config.borrow_mut();
            if let Some(console) = cfg.consoles.iter_mut().find(|c| c.id == console_id) {
                console.grid_art = art;
            }
            let _ = crate::config::save_config(&cfg);
        }
        if combo.active_id().as_deref() != Some(art.as_str()) {
            Self::sync_grid_art_combo(combo, updating, art);
        }
    }

    fn grid_tile_media<'a>(game: &'a Game, config: &crate::config::Config) -> Option<&'a crate::types::Media> {
        let preferred = config
            .consoles
            .iter()
            .find(|console| console.id == game.console)
            .map(|console| console.grid_art)
            .unwrap_or_default();
        let have = present_kinds(&game.media);
        let kind = pick_kind(&have, preferred)?;
        game.media.iter().find(|media| media.kind == kind && media.path.is_file())
    }

    fn append_detail_media(detail_content: &gtk4::Box, game: &Game, kind: MediaKind, caption: &str) {
        let Some(media) = game.media.iter().find(|media| media.kind == kind && media.path.is_file()) else {
            return;
        };
        let Ok(pixbuf) = Pixbuf::from_file_at_scale(&media.path, 250, 120, true) else {
            return;
        };
        let label = gtk4::Label::new(Some(caption));
        label.set_halign(gtk4::Align::Start);
        label.add_css_class("title-4");
        detail_content.append(&label);
        let picture = gtk4::Picture::for_pixbuf(&pixbuf);
        picture.set_can_shrink(true);
        picture.set_height_request(120);
        detail_content.append(&picture);
    }

    fn wire_scrape_actions(
        &self,
        scraper_button: &gtk4::Button,
        scrape_button: &gtk4::Button,
        scrape_missing_button: &gtk4::Button,
    ) {
        let window = self.window.clone();
        let config = self.config.clone();
        scraper_button.connect_clicked(move |_| {
            crate::dialogs::open_scraper(&window, config.clone());
        });

        let status = self.status_label.clone();
        let running = self.scrape_running.clone();
        let config = self.config.clone();
        let conn = self.conn.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let grid = self.game_grid.clone();
        let stack = self.center_stack.clone();
        let detail = self.detail_content.clone();
        let selected = self.selected_game.clone();
        let search = self.search_entry.clone();
        scrape_button.connect_clicked(move |_| {
            Self::request_scrape_selected(&status, &running, &config, &conn, &games, &all_games, &grid, &stack, &detail, &selected, &search);
        });

        let status = self.status_label.clone();
        let running = self.scrape_running.clone();
        let config = self.config.clone();
        let conn = self.conn.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let grid = self.game_grid.clone();
        let stack = self.center_stack.clone();
        let detail = self.detail_content.clone();
        let selected = self.selected_game.clone();
        let search = self.search_entry.clone();
        let current = self.current_console.clone();
        scrape_missing_button.connect_clicked(move |_| {
            Self::request_scrape_missing(&status, &running, &config, &conn, &all_games, &games, &grid, &stack, &detail, &selected, &search, &current);
        });
    }

    fn request_scrape_selected(
        status: &gtk4::Label,
        running: &Rc<RefCell<bool>>,
        config: &Rc<RefCell<crate::config::Config>>,
        conn: &Rc<RefCell<rusqlite::Connection>>,
        games: &Rc<RefCell<Vec<Game>>>,
        all_games: &Rc<RefCell<Vec<Game>>>,
        grid: &gtk4::FlowBox,
        stack: &gtk4::Stack,
        detail: &gtk4::Box,
        selected: &Rc<RefCell<Option<usize>>>,
        search: &gtk4::SearchEntry,
    ) {
        let idx = *selected.borrow();
        let Some(idx) = idx else {
            Self::show_status(status, "Select a game to scrape.");
            return;
        };
        let Some(game) = games.borrow().get(idx).cloned() else {
            Self::show_status(status, "Select a game to scrape.");
            return;
        };
        Self::begin_scrape(status, running, config, conn, vec![game], all_games, games, grid, stack, detail, selected, search);
    }

    fn request_scrape_missing(
        status: &gtk4::Label,
        running: &Rc<RefCell<bool>>,
        config: &Rc<RefCell<crate::config::Config>>,
        conn: &Rc<RefCell<rusqlite::Connection>>,
        all_games: &Rc<RefCell<Vec<Game>>>,
        games: &Rc<RefCell<Vec<Game>>>,
        grid: &gtk4::FlowBox,
        stack: &gtk4::Stack,
        detail: &gtk4::Box,
        selected: &Rc<RefCell<Option<usize>>>,
        search: &gtk4::SearchEntry,
        current: &Rc<RefCell<Option<String>>>,
    ) {
        let Some(console_id) = current.borrow().clone() else {
            Self::show_status(status, "Select a system before scraping missing artwork.");
            return;
        };
        let targets: Vec<Game> = all_games
            .borrow()
            .iter()
            .filter(|game| game.console == console_id)
            .cloned()
            .collect();
        if targets.is_empty() {
            Self::show_status(status, "This system has no games to scrape.");
            return;
        }
        Self::begin_scrape(status, running, config, conn, targets, all_games, games, grid, stack, detail, selected, search);
    }

    fn begin_scrape(
        status: &gtk4::Label,
        running: &Rc<RefCell<bool>>,
        config: &Rc<RefCell<crate::config::Config>>,
        conn: &Rc<RefCell<rusqlite::Connection>>,
        targets: Vec<Game>,
        all_games: &Rc<RefCell<Vec<Game>>>,
        games: &Rc<RefCell<Vec<Game>>>,
        grid: &gtk4::FlowBox,
        _stack: &gtk4::Stack,
        detail: &gtk4::Box,
        selected: &Rc<RefCell<Option<usize>>>,
        _search: &gtk4::SearchEntry,
    ) {
        if *running.borrow() {
            Self::show_status(status, "A scrape is already running.");
            return;
        }
        *running.borrow_mut() = true;
        Self::show_status(status, "Scraping artwork…");
        let scraper = config.borrow().scraper.clone();
        let rx = scraper::spawn_scrape(targets, scraper, scraper::fixture_dir_from_env());
        let status = status.clone();
        let running = running.clone();
        let conn = conn.clone();
        let config = config.clone();
        let all_games = all_games.clone();
        let games = games.clone();
        let grid = grid.clone();
        let detail = detail.clone();
        let selected = selected.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            // One update per tick. A long scrape can queue many saves; draining
            // them here would decode images and stall input on the main loop.
            match rx.try_recv() {
                Ok(scraper::ScrapeUpdate::Status(text)) => {
                    Self::show_status(&status, &text);
                    glib::ControlFlow::Continue
                }
                Ok(scraper::ScrapeUpdate::Saved { game_id, media }) => {
                    Self::apply_saved_media(&conn, &config, &all_games, &games, &grid, &detail, &selected, &game_id, &media);
                    glib::ControlFlow::Continue
                }
                Ok(scraper::ScrapeUpdate::Done(text)) => {
                    Self::show_status(&status, &text);
                    *running.borrow_mut() = false;
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    *running.borrow_mut() = false;
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Persist one scraped file and patch that game's tile. The grid is not rebuilt,
    /// so FlowBox selection and scroll stay where the user left them.
    fn apply_saved_media(
        conn: &Rc<RefCell<rusqlite::Connection>>,
        config: &Rc<RefCell<crate::config::Config>>,
        all_games: &Rc<RefCell<Vec<Game>>>,
        games: &Rc<RefCell<Vec<Game>>>,
        grid: &gtk4::FlowBox,
        detail: &gtk4::Box,
        selected: &Rc<RefCell<Option<usize>>>,
        game_id: &str,
        media: &Media,
    ) {
        let id = game_id.to_string();
        if database::set_game_media(&conn.borrow(), &id, media).is_err() {
            return;
        }
        Self::remember_media(&mut all_games.borrow_mut(), game_id, media);
        Self::remember_media(&mut games.borrow_mut(), game_id, media);
        let selected_id = selected
            .borrow()
            .and_then(|idx| games.borrow().get(idx).map(|game| game.id.clone()));
        if let Some(index) = games.borrow().iter().position(|game| game.id == game_id) {
            let game = games.borrow()[index].clone();
            Self::refresh_grid_tile(grid, index, &game, &config.borrow());
        }
        if selected_id.as_deref() == Some(game_id) {
            if let Some(idx) = *selected.borrow() {
                if let Some(game) = games.borrow().get(idx).cloned() {
                    Self::update_game_details(detail, &game, &config.borrow(), conn);
                }
            }
        }
    }

    fn remember_media(games: &mut [Game], game_id: &str, media: &Media) {
        let Some(game) = games.iter_mut().find(|game| game.id == game_id) else {
            return;
        };
        if let Some(slot) = game.media.iter_mut().find(|item| item.kind == media.kind) {
            *slot = media.clone();
        } else {
            game.media.push(media.clone());
        }
    }

    fn set_tile_art(frame: &gtk4::Frame, game: &Game, config: &crate::config::Config) {
        if let Some(media) = Self::grid_tile_media(game, config) {
            if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&media.path, 150, 150, true) {
                frame.set_child(Some(&gtk4::Picture::for_pixbuf(&pixbuf)));
                return;
            }
        }
        let label = gtk4::Label::new(Some("No Art"));
        label.add_css_class("title-4");
        frame.set_child(Some(&label));
    }

    fn refresh_grid_tile(grid: &gtk4::FlowBox, index: usize, game: &Game, config: &crate::config::Config) {
        let Some(flow_child) = grid.child_at_index(index as i32) else {
            return;
        };
        let Some(tile) = flow_child.child().and_downcast::<gtk4::Box>() else {
            return;
        };
        let Some(frame) = tile.first_child().and_downcast::<gtk4::Frame>() else {
            return;
        };
        Self::set_tile_art(&frame, game, config);
    }

    fn show_status(status: &gtk4::Label, text: &str) {
        status.set_text(text);
        status.set_visible(!text.is_empty());
    }

    pub fn show(&self) {
        self.window.present();
    }
}
