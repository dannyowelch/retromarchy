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
             .console-subtitle { opacity: 0.65; font-size: 0.9em; }"
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().expect("Could not get default display"),
            &base_css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let theme_css_provider = gtk4::CssProvider::new();

        let header_bar = adw::HeaderBar::new();
        
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

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Retromarchy")
            .default_width(1200)
            .default_height(700)
            .build();

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.append(&header_bar);

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
        };

        app_instance.apply_theme(&config.theme);
        app_instance.setup_console_list();
        app_instance.setup_search();
        app_instance.setup_keyboard_navigation();
        app_instance.setup_game_selection();
        app_instance.setup_details_toggle(details_toggle);
        app_instance.setup_theme_toggle(theme_toggle);

        Ok(app_instance)
    }
    
    fn setup_search(&self) {
        let search_entry = self.search_entry.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let game_grid = self.game_grid.clone();
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
            Self::update_game_grid(&game_grid, &games.borrow(), &config.borrow());
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
        let selected_game = self.selected_game.clone();
        let detail_content = self.detail_content.clone();

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
                        Self::update_game_grid(&game_grid, &games.borrow(), &config.borrow());
                    }

                    Self::update_console_details(&detail_content, &console.id, &conn.borrow(), &config.borrow());
                }
            }
        });

        if !self.config.borrow().consoles.is_empty() {
            self.console_list.select_row(self.console_list.row_at_index(0).as_ref());
        }
    }

    fn update_game_grid(grid: &gtk4::FlowBox, games: &[Game], config: &Config) {
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

        if let Some(media) = game.media.iter().find(|m| m.kind == MediaKind::BoxArt) {
            if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&media.path, 250, 250, true) {
                let picture = gtk4::Picture::for_pixbuf(&pixbuf);
                picture.set_can_shrink(true);
                picture.set_height_request(250);
                detail_content.append(&picture);
            }
        }

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
        
        play_button.connect_clicked(move |_| {
            if let Some(profile) = Self::resolve_profile(&config_clone, &game_clone) {
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
            let time_str = if hours > 0 {
                format!("Play time: {}h {}m", hours, minutes)
            } else {
                format!("Play time: {}m", minutes)
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
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

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

        key_controller.connect_key_pressed(move |_, key, _, _| {
            let focused = gtk4::prelude::RootExt::focus(&window);
            let search_has_focus = focused.as_ref().map_or(false, |w| {
                w.upcast_ref::<gtk4::Widget>() == search_entry.upcast_ref::<gtk4::Widget>() ||
                search_entry.is_ancestor(w)
            });
            
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
                        Self::update_game_grid(&game_grid, &games.borrow(), &config.borrow());
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
                                if let Some(profile) = Self::resolve_profile(&config.borrow(), game) {
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
                        }
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
                                    let _ = database::remove_missing_games(&conn.borrow(), &ids);

                                    if let Ok(loaded) = database::load_games(&conn.borrow(), Some(&console_id)) {
                                        *all_games.borrow_mut() = loaded.clone();
                                        *games.borrow_mut() = loaded;
                                        Self::update_game_grid(&game_grid, &games.borrow(), &config.borrow());
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
                    if !search_has_focus {
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
                    if !search_has_focus {
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
                    if !search_has_focus {
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
                    if !search_has_focus {
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
