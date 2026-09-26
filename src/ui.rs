use crate::config::Config;
use crate::database;
use crate::gamepad::{self, NavDir, PadAction};
use crate::input_repeat::{AxisHold, AxisSide, DirectionRepeat};
use crate::launcher;
use crate::scanner;
use crate::scraper::{self, pick_kind, present_kinds};
use crate::types::{visible_games, Game, GameAction, GridArt, GridFilter, Media, MediaKind};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusPane {
    Systems,
    Games,
}

enum PadTarget {
    Menu,
    Dialog(gtk4::Window),
    Grid,
}

/// Shared width of the details column. Console info and a selected game
/// both use it, so picking a game does not take columns from the grid.
const DETAIL_PANE_WIDTH: i32 = 280;

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
    grid_filter_combo: gtk4::ComboBoxText,
    grid_filter: Rc<RefCell<GridFilter>>,
    focus_pane: Rc<RefCell<FocusPane>>,
    status_label: gtk4::Label,
    scrape_running: Rc<RefCell<bool>>,
    updating_grid_art: Rc<RefCell<bool>>,
    game_menu: gtk4::Popover,
}

impl App {
    pub fn new(
        app: &adw::Application,
        config: Config,
        conn: Rc<RefCell<Connection>>,
    ) -> Result<Self> {
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
             .scrape-status { padding: 6px 12px; background: alpha(@accent_bg_color, 0.35); }
             .favorite-badge {
                color: #e01b24;
                background-color: transparent;
                border-radius: 0;
                padding: 0;
                font-family: \"DejaVu Sans\", sans-serif;
                font-size: 18px;
                font-weight: 700;
             }
             .favorite-toggle {
                min-width: 0;
                min-height: 0;
                padding: 4px 10px;
                font-family: \"DejaVu Sans\", sans-serif;
                font-size: 18px;
             }
             .favorite-toggle.is-favorite,
             .favorite-toggle.is-favorite label {
                color: #e01b24;
             }",
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

        let options_button = gtk4::Button::with_label("Options");
        options_button.add_css_class("flat");
        options_button.set_tooltip_text(Some("Input options"));
        header_bar.pack_start(&options_button);

        let scrape_button = gtk4::Button::with_label("Scrape");
        scrape_button.add_css_class("flat");
        scrape_button.set_tooltip_text(Some("Choose a match for the selected game (S)"));
        header_bar.pack_start(&scrape_button);

        let scrape_missing_button = gtk4::Button::with_label("Scrape Missing");
        scrape_missing_button.add_css_class("flat");
        scrape_missing_button
            .set_tooltip_text(Some("Scrape missing artwork for this system (Shift+S)"));
        header_bar.pack_start(&scrape_missing_button);

        let grid_art_combo = gtk4::ComboBoxText::new();
        grid_art_combo.append(Some(GridArt::BoxArt.as_str()), GridArt::BoxArt.label());
        grid_art_combo.append(
            Some(GridArt::Screenshot.as_str()),
            GridArt::Screenshot.label(),
        );
        grid_art_combo.set_active_id(Some(GridArt::BoxArt.as_str()));
        grid_art_combo.set_tooltip_text(Some("Grid artwork for this system (1 box, 2 screenshot)"));

        let grid_filter_combo = gtk4::ComboBoxText::new();
        grid_filter_combo.append(Some(GridFilter::All.as_str()), GridFilter::All.label());
        grid_filter_combo.append(
            Some(GridFilter::Favorites.as_str()),
            GridFilter::Favorites.label(),
        );
        grid_filter_combo.set_active_id(Some(GridFilter::All.as_str()));
        grid_filter_combo.set_tooltip_text(Some("Show all games or favorites"));

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
        header_bar.pack_end(&grid_filter_combo);
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

        let detail_pane = Self::detail_column();

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
            grid_filter_combo: grid_filter_combo.clone(),
            grid_filter: Rc::new(RefCell::new(GridFilter::All)),
            focus_pane: Rc::new(RefCell::new(FocusPane::Systems)),
            status_label: status_label.clone(),
            scrape_running: Rc::new(RefCell::new(false)),
            updating_grid_art: Rc::new(RefCell::new(false)),
            game_menu: gtk4::Popover::new(),
        };

        app_instance.apply_theme(&config.theme);
        app_instance.setup_console_list();
        app_instance.setup_search();
        let key_x = Rc::new(RefCell::new(AxisHold::default()));
        let key_y = Rc::new(RefCell::new(AxisHold::default()));
        let repeat_x = Rc::new(RefCell::new(DirectionRepeat::default()));
        let repeat_y = Rc::new(RefCell::new(DirectionRepeat::default()));
        let nav_started = Instant::now();
        app_instance.setup_game_menu();
        app_instance.setup_keyboard_navigation(
            key_x.clone(),
            key_y.clone(),
            repeat_x.clone(),
            repeat_y.clone(),
            nav_started,
        );
        app_instance.setup_game_selection();
        app_instance.setup_details_toggle(details_toggle);
        app_instance.setup_theme_toggle(theme_toggle);
        app_instance.wire_library_actions(
            &import_button,
            &emulator_button,
            &empty_import,
            &empty_emulators,
        );
        app_instance.wire_scrape_actions(&scraper_button, &scrape_button, &scrape_missing_button);
        app_instance.setup_grid_art();
        app_instance.setup_grid_filter();
        app_instance.setup_direction_repeat(key_x, key_y, repeat_x, repeat_y, nav_started);
        app_instance.setup_favorite_action();
        {
            let window = window.clone();
            let config = config_rc.clone();
            options_button.connect_clicked(move |_| {
                crate::dialogs::open_options(&window, config.clone());
            });
        }

        Ok(app_instance)
    }

    fn setup_search(&self) {
        let search_entry = self.search_entry.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let game_grid = self.game_grid.clone();
        let center_stack = self.center_stack.clone();
        let config = self.config.clone();
        let grid_filter = self.grid_filter.clone();
        let selected_game = self.selected_game.clone();
        let detail_content = self.detail_content.clone();
        let conn = self.conn.clone();
        let current_console = self.current_console.clone();

        search_entry.connect_search_changed(move |entry| {
            let text = entry.text().to_string();
            let all = all_games.borrow();
            let visible = visible_games(&all, &text, *grid_filter.borrow());
            let library_empty = all.is_empty();
            drop(all);
            Self::replace_visible_games(
                &game_grid,
                &center_stack,
                &config.borrow(),
                visible,
                library_empty,
                &games,
                &selected_game,
                &detail_content,
                &conn,
                &current_console.borrow(),
            );
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
                }
                .favorite-badge {
                    color: #e01b24;
                    background-color: transparent;
                    border-radius: 0;
                    padding: 0;
                    font-family: \"DejaVu Sans\", sans-serif;
                    font-size: 18px;
                    font-weight: 700;
                }
                .favorite-toggle {
                    min-width: 0;
                    min-height: 0;
                    padding: 4px 10px;
                    font-family: \"DejaVu Sans\", sans-serif;
                    font-size: 18px;
                }
                .favorite-toggle.is-favorite,
                .favorite-toggle.is-favorite label {
                    color: #e01b24;
                }",
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
                    }
                    .favorite-badge {
                        color: #e01b24;
                        background-color: transparent;
                        border-radius: 0;
                        padding: 0;
                        font-family: \"DejaVu Sans\", sans-serif;
                        font-size: 18px;
                        font-weight: 700;
                    }
                    .favorite-toggle {
                        min-width: 0;
                        min-height: 0;
                        padding: 4px 10px;
                        font-family: \"DejaVu Sans\", sans-serif;
                        font-size: 18px;
                    }
                    .favorite-toggle.is-favorite,
                    .favorite-toggle.is-favorite label {
                        color: #e01b24;
                    }",
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
        let grid_filter = self.grid_filter.clone();
        let focus_pane = self.focus_pane.clone();
        let search_entry = self.search_entry.clone();

        self.console_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let idx = row.index() as usize;
                if idx < config.borrow().consoles.len() {
                    let console = &config.borrow().consoles[idx];
                    *current_console.borrow_mut() = Some(console.id.clone());
                    *selected_game.borrow_mut() = None;
                    *focus_pane.borrow_mut() = FocusPane::Systems;

                    if let Ok(loaded_games) =
                        database::load_games(&conn.borrow(), Some(&console.id))
                    {
                        *all_games.borrow_mut() = loaded_games;
                        let visible = visible_games(
                            &all_games.borrow(),
                            &search_entry.text(),
                            *grid_filter.borrow(),
                        );
                        let library_empty = all_games.borrow().is_empty();
                        Self::replace_visible_games(
                            &game_grid,
                            &center_stack,
                            &config.borrow(),
                            visible,
                            library_empty,
                            &games,
                            &selected_game,
                            &detail_content,
                            &conn,
                            &current_console.borrow(),
                        );
                    }
                    Self::sync_grid_art_combo(
                        &grid_art_combo,
                        &updating_grid_art,
                        console.grid_art,
                    );
                }
            }
        });

        if !self.config.borrow().consoles.is_empty() {
            self.console_list
                .select_row(self.console_list.row_at_index(0).as_ref());
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
                Self::update_game_grid(&game_grid, &center_stack, &[], &config.borrow(), true);
                while let Some(child) = detail_content.first_child() {
                    detail_content.remove(&child);
                }
            }
        })
    }

    fn update_game_grid(
        grid: &gtk4::FlowBox,
        stack: &gtk4::Stack,
        games: &[Game],
        config: &Config,
        library_empty: bool,
    ) {
        // The menu is parented to a tile. Unparent it before those tiles are
        // removed, or GTK finalizes the tile with the popover still attached
        // and the next click aborts.
        Self::detach_popovers(grid.upcast_ref());

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
            title.set_widget_name("game-title");
            title.set_wrap(true);
            title.set_max_width_chars(20);
            title.set_lines(2);
            title.set_ellipsize(gtk4::pango::EllipsizeMode::End);

            game_box.append(&frame);
            game_box.append(&title);
            if let Some(console) = config.consoles.iter().find(|c| c.id == game.console) {
                let console_label = gtk4::Label::new(Some(&console.name));
                console_label.set_wrap(false);
                console_label.set_max_width_chars(20);
                console_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
                console_label.add_css_class("caption");
                console_label.add_css_class("console-subtitle");
                game_box.append(&console_label);
            }

            // Overlay the whole rom-tile so the heart sits in the title band, not on the art.
            let heart = gtk4::Label::new(Some("♥"));
            heart.set_widget_name("favorite-badge");
            heart.add_css_class("favorite-badge");
            heart.set_halign(gtk4::Align::End);
            heart.set_valign(gtk4::Align::End);
            heart.set_margin_bottom(6);
            heart.set_margin_end(6);
            heart.set_can_target(false);
            heart.set_visible(game.favorite);

            let card = gtk4::Overlay::new();
            card.set_child(Some(&game_box));
            card.add_overlay(&heart);

            grid.insert(&card, -1);
        }

        if games.is_empty() && library_empty {
            stack.set_visible_child_name("empty");
        } else {
            stack.set_visible_child_name("grid");
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn replace_visible_games(
        grid: &gtk4::FlowBox,
        stack: &gtk4::Stack,
        config: &Config,
        visible: Vec<Game>,
        library_empty: bool,
        games: &Rc<RefCell<Vec<Game>>>,
        selected: &Rc<RefCell<Option<usize>>>,
        detail: &gtk4::Box,
        conn: &Rc<RefCell<Connection>>,
        current_console: &Option<String>,
    ) {
        let keep = selected
            .borrow()
            .and_then(|idx| games.borrow().get(idx).map(|game| game.id.clone()));
        *games.borrow_mut() = visible;
        Self::update_game_grid(grid, stack, &games.borrow(), config, library_empty);
        if let Some(id) = keep.as_deref() {
            if let Some(idx) = games.borrow().iter().position(|game| game.id == id) {
                if let Some(child) = grid.child_at_index(idx as i32) {
                    grid.select_child(&child);
                    *selected.borrow_mut() = Some(idx);
                    return;
                }
            }
        }
        grid.unselect_all();
        *selected.borrow_mut() = None;
        if let Some(console_id) = current_console {
            Self::update_console_details(detail, console_id, &conn.borrow(), config);
        }
    }

    fn update_console_details(
        detail_content: &gtk4::Box,
        console_id: &str,
        conn: &Connection,
        config: &Config,
    ) {
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
                    let manufacturer_label =
                        gtk4::Label::new(Some(&format!("Manufacturer: {}", meta.manufacturer)));
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
                let total_label =
                    gtk4::Label::new(Some(&format!("Total games: {}", stats.total_games)));
                total_label.set_halign(gtk4::Align::Start);
                total_label.add_css_class("caption");
                detail_content.append(&total_label);

                if let Some(date) = stats.last_played_date {
                    let formatted = date.format("%Y-%m-%d %H:%M").to_string();
                    let last_played_label =
                        gtk4::Label::new(Some(&format!("Last played: {}", formatted)));
                    last_played_label.set_halign(gtk4::Align::Start);
                    last_played_label.add_css_class("caption");
                    detail_content.append(&last_played_label);

                    if let Some(game_title) = stats.last_played_game {
                        let last_game_label =
                            gtk4::Label::new(Some(&format!("  → {}", game_title)));
                        last_game_label.set_halign(gtk4::Align::Start);
                        last_game_label.add_css_class("caption");
                        last_game_label.add_css_class("dim-label");
                        detail_content.append(&last_game_label);
                    }
                }

                if stats.total_play_count > 0 {
                    let count_label =
                        gtk4::Label::new(Some(&format!("Total plays: {}", stats.total_play_count)));
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
                    let most_played_label =
                        gtk4::Label::new(Some(&format!("Most played: {}", most_played)));
                    most_played_label.set_halign(gtk4::Align::Start);
                    most_played_label.add_css_class("caption");
                    detail_content.append(&most_played_label);

                    let count_label =
                        gtk4::Label::new(Some(&format!("  → {} plays", stats.most_played_count)));
                    count_label.set_halign(gtk4::Align::Start);
                    count_label.add_css_class("caption");
                    count_label.add_css_class("dim-label");
                    detail_content.append(&count_label);
                }
            }
        }
    }

    /// Right-hand column. `hexpand` is set explicitly so a descendant
    /// (the Play button) cannot make the column absorb spare window width.
    fn detail_column() -> gtk4::ScrolledWindow {
        let pane = gtk4::ScrolledWindow::builder()
            .width_request(DETAIL_PANE_WIDTH)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();
        pane.set_hexpand(false);
        pane
    }

    fn update_game_details(
        detail_content: &gtk4::Box,
        game: &Game,
        config: &Config,
        conn: &Rc<RefCell<Connection>>,
    ) {
        while let Some(child) = detail_content.first_child() {
            detail_content.remove(&child);
        }

        let play_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let play_button = gtk4::Button::with_label("Play");
        play_button.set_hexpand(true);
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
                        let _ = database::increment_play_stats(
                            &conn_for_update.borrow(),
                            &game_id,
                            elapsed,
                        );
                        let _ = database::update_last_played(&conn_for_update.borrow(), &game_id);

                        if let Ok(updated_games) =
                            database::load_games(&conn_for_update.borrow(), Some(&game_console))
                        {
                            if let Some(updated_game) =
                                updated_games.iter().find(|g| g.id == game_id)
                            {
                                Self::update_game_details(
                                    &detail_for_update,
                                    updated_game,
                                    &config_for_update,
                                    &conn_for_update,
                                );
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

        play_row.append(&play_button);
        play_row.append(&Self::favorite_toggle_button(game.favorite));
        detail_content.append(&play_row);

        if let Some(media) = game
            .media
            .iter()
            .find(|m| m.kind == MediaKind::BoxArt && m.path.is_file())
        {
            if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&media.path, 250, 180, true) {
                let picture = gtk4::Picture::for_pixbuf(&pixbuf);
                picture.set_can_shrink(true);
                picture.set_height_request(180);
                detail_content.append(&picture);
            }
        }

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

        let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        separator.set_margin_top(8);
        separator.set_margin_bottom(8);
        detail_content.append(&separator);

        let rom_label = gtk4::Label::new(Some(&format!("ROM: {}", game.rom.display())));
        rom_label.set_wrap(true);
        // Paths are one long token. Break inside it so the line cannot
        // set the column's minimum width.
        rom_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        rom_label.set_halign(gtk4::Align::Start);
        rom_label.add_css_class("caption");
        detail_content.append(&rom_label);

        if let Some(crc) = game.crc32 {
            let crc_label = gtk4::Label::new(Some(&format!("CRC32: {:08x}", crc)));
            crc_label.set_halign(gtk4::Align::Start);
            crc_label.add_css_class("caption");
            detail_content.append(&crc_label);
        }
    }

    fn setup_game_selection(&self) {
        let selected_game = self.selected_game.clone();
        let games = self.games.clone();
        let detail_content = self.detail_content.clone();
        let config = self.config.clone();
        let conn = self.conn.clone();
        let focus_pane = self.focus_pane.clone();

        self.game_grid.connect_child_activated(move |_, child| {
            *focus_pane.borrow_mut() = FocusPane::Games;
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
        let focus_pane2 = self.focus_pane.clone();

        game_grid.connect_selected_children_changed(move |grid| {
            if let Some(child) = grid.selected_children().first() {
                if child.has_focus() || grid.has_focus() {
                    *focus_pane2.borrow_mut() = FocusPane::Games;
                }
                let idx = child.index() as usize;
                *selected_game2.borrow_mut() = Some(idx);

                let games_borrow = games2.borrow();
                if let Some(game) = games_borrow.get(idx) {
                    Self::update_game_details(&detail_content2, game, &config2.borrow(), &conn2);
                }
            }
        });
    }

    fn setup_keyboard_navigation(
        &self,
        key_x: Rc<RefCell<AxisHold>>,
        key_y: Rc<RefCell<AxisHold>>,
        repeat_x: Rc<RefCell<DirectionRepeat<NavDir>>>,
        repeat_y: Rc<RefCell<DirectionRepeat<NavDir>>>,
        nav_started: Instant,
    ) {
        let key_controller = gtk4::EventControllerKey::new();
        // Capture so arrow keys are ours. Bubble would let the systems list
        // move once and then this handler move again.
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
        let center_stack = self.center_stack.clone();
        let console_list = self.console_list.clone();
        let status_label = self.status_label.clone();
        let scrape_running = self.scrape_running.clone();
        let grid_art_combo = self.grid_art_combo.clone();
        let grid_filter_combo = self.grid_filter_combo.clone();
        let grid_filter = self.grid_filter.clone();
        let focus_pane = self.focus_pane.clone();
        let updating_grid_art = self.updating_grid_art.clone();
        let game_menu = self.game_menu.clone();

        let key_x_release = key_x.clone();
        let key_y_release = key_y.clone();
        let repeat_x_release = repeat_x.clone();
        let repeat_y_release = repeat_y.clone();
        let config_release = self.config.clone();
        key_controller.connect_key_pressed(move |_, key, _, mods| {
            if game_menu.is_visible() {
                return match key {
                    gdk::Key::Escape => {
                        game_menu.popdown();
                        glib::Propagation::Stop
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        Self::activate_menu(&game_menu);
                        glib::Propagation::Stop
                    }
                    gdk::Key::Up | gdk::Key::k => {
                        Self::move_menu(&game_menu, NavDir::Up);
                        glib::Propagation::Stop
                    }
                    gdk::Key::Down | gdk::Key::j => {
                        Self::move_menu(&game_menu, NavDir::Down);
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Stop,
                };
            }
            let focused = gtk4::prelude::RootExt::focus(&window);
            let search_has_focus = focused.as_ref().is_some_and(|w| {
                w.upcast_ref::<gtk4::Widget>() == search_entry.upcast_ref::<gtk4::Widget>()
                    || search_entry.is_ancestor(w)
            });
            let combo_has_focus = focused.as_ref().is_some_and(|w| {
                w.upcast_ref::<gtk4::Widget>() == grid_art_combo.upcast_ref::<gtk4::Widget>()
                    || grid_art_combo.is_ancestor(w)
                    || w.upcast_ref::<gtk4::Widget>()
                        == grid_filter_combo.upcast_ref::<gtk4::Widget>()
                    || grid_filter_combo.is_ancestor(w)
            });

            if mods.contains(gdk::ModifierType::CONTROL_MASK) && !search_has_focus {
                if key == gdk::Key::g || key == gdk::Key::G {
                    crate::dialogs::open_scraper(&window, config.clone());
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::i || key == gdk::Key::I {
                    let done = Self::refresh_action(
                        console_list.clone(),
                        config.clone(),
                        conn.clone(),
                        games.clone(),
                        all_games.clone(),
                        game_grid.clone(),
                        center_stack.clone(),
                        current_console.clone(),
                        detail_content.clone(),
                    );
                    crate::dialogs::open_import(&window, config.clone(), conn.clone(), done);
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::e
                    || key == gdk::Key::E
                    || key == gdk::Key::m
                    || key == gdk::Key::M
                {
                    let done = Self::refresh_action(
                        console_list.clone(),
                        config.clone(),
                        conn.clone(),
                        games.clone(),
                        all_games.clone(),
                        game_grid.clone(),
                        center_stack.clone(),
                        current_console.clone(),
                        detail_content.clone(),
                    );
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
                    } else {
                        *selected_game.borrow_mut() = None;
                        game_grid.unselect_all();
                        if let Some(console_id) = current_console.borrow().as_ref() {
                            Self::update_console_details(
                                &detail_content,
                                console_id,
                                &conn.borrow(),
                                &config.borrow(),
                            );
                        }
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if !search_has_focus {
                        Self::launch_selected(
                            &window,
                            &config,
                            &conn,
                            &detail_content,
                            &games,
                            &selected_game,
                        );
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::s | gdk::Key::S => {
                    if !search_has_focus {
                        if mods.contains(gdk::ModifierType::SHIFT_MASK) || key == gdk::Key::S {
                            Self::request_scrape_missing(
                                &status_label,
                                &scrape_running,
                                &config,
                                &conn,
                                &all_games,
                                &games,
                                &game_grid,
                                &center_stack,
                                &detail_content,
                                &selected_game,
                                &search_entry,
                                &current_console,
                            );
                        } else {
                            Self::request_scrape_selected(
                                &window,
                                &status_label,
                                &scrape_running,
                                &config,
                                &conn,
                                &games,
                                &all_games,
                                &game_grid,
                                &detail_content,
                                &selected_game,
                            );
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::_1 | gdk::Key::_2 => {
                    if !search_has_focus {
                        let art = match key {
                            gdk::Key::_1 => GridArt::BoxArt,
                            _ => GridArt::Screenshot,
                        };
                        Self::set_current_grid_art(
                            &config,
                            &current_console,
                            &grid_art_combo,
                            &updating_grid_art,
                            art,
                        );
                        Self::redraw_current_games(
                            &game_grid,
                            &center_stack,
                            &config,
                            &games,
                            &all_games,
                            &selected_game,
                            &detail_content,
                            &conn,
                            &current_console,
                        );
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::r => {
                    if !search_has_focus {
                        if let Some(console_id) = current_console.borrow().clone() {
                            if let Some(console) =
                                config.borrow().consoles.iter().find(|c| c.id == console_id)
                            {
                                let console_clone = console.clone();
                                if let Ok(scanned) = scanner::scan_console(&console_clone) {
                                    let ids: Vec<_> =
                                        scanned.iter().map(|g| g.id.clone()).collect();
                                    for game in &scanned {
                                        let _ = database::upsert_game(&conn.borrow(), game);
                                    }
                                    let _ = database::remove_missing_games(
                                        &conn.borrow(),
                                        &console_id,
                                        &ids,
                                    );

                                    if let Ok(loaded) =
                                        database::load_games(&conn.borrow(), Some(&console_id))
                                    {
                                        *all_games.borrow_mut() = loaded;
                                        let visible = visible_games(
                                            &all_games.borrow(),
                                            &search_entry.text(),
                                            *grid_filter.borrow(),
                                        );
                                        let library_empty = all_games.borrow().is_empty();
                                        Self::replace_visible_games(
                                            &game_grid,
                                            &center_stack,
                                            &config.borrow(),
                                            visible,
                                            library_empty,
                                            &games,
                                            &selected_game,
                                            &detail_content,
                                            &conn,
                                            &current_console.borrow(),
                                        );
                                    }

                                    Self::update_console_details(
                                        &detail_content,
                                        &console_id,
                                        &conn.borrow(),
                                        &config.borrow(),
                                    );
                                }
                            }
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Left => Self::press_dir(
                    &key_x,
                    AxisSide::Negative,
                    NavDir::Left,
                    &repeat_x,
                    nav_started,
                    &config,
                    combo_has_focus || search_has_focus,
                    |dir| {
                        Self::step_nav(
                            &console_list,
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                            &detail_content,
                            &config,
                            &conn,
                            dir,
                        )
                    },
                ),
                gdk::Key::Right => Self::press_dir(
                    &key_x,
                    AxisSide::Positive,
                    NavDir::Right,
                    &repeat_x,
                    nav_started,
                    &config,
                    combo_has_focus || search_has_focus,
                    |dir| {
                        Self::step_nav(
                            &console_list,
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                            &detail_content,
                            &config,
                            &conn,
                            dir,
                        )
                    },
                ),
                gdk::Key::Up => Self::press_dir(
                    &key_y,
                    AxisSide::Negative,
                    NavDir::Up,
                    &repeat_y,
                    nav_started,
                    &config,
                    combo_has_focus || search_has_focus,
                    |dir| {
                        Self::step_nav(
                            &console_list,
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                            &detail_content,
                            &config,
                            &conn,
                            dir,
                        )
                    },
                ),
                gdk::Key::Down => Self::press_dir(
                    &key_y,
                    AxisSide::Positive,
                    NavDir::Down,
                    &repeat_y,
                    nav_started,
                    &config,
                    combo_has_focus || search_has_focus,
                    |dir| {
                        Self::step_nav(
                            &console_list,
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                            &detail_content,
                            &config,
                            &conn,
                            dir,
                        )
                    },
                ),
                gdk::Key::h => {
                    if combo_has_focus {
                        glib::Propagation::Proceed
                    } else if !search_has_focus {
                        Self::move_grid(
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                            NavDir::Left,
                            &update_details,
                        );
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::l => {
                    if combo_has_focus {
                        glib::Propagation::Proceed
                    } else if !search_has_focus {
                        Self::move_grid(
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                            NavDir::Right,
                            &update_details,
                        );
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::k => {
                    if combo_has_focus {
                        glib::Propagation::Proceed
                    } else if !search_has_focus {
                        Self::move_grid(
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                            NavDir::Up,
                            &update_details,
                        );
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::j => {
                    if combo_has_focus {
                        glib::Propagation::Proceed
                    } else if !search_has_focus {
                        Self::move_grid(
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                            NavDir::Down,
                            &update_details,
                        );
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::f => {
                    if !search_has_focus && !combo_has_focus {
                        Self::toggle_favorite(
                            &conn,
                            &games,
                            &all_games,
                            &selected_game,
                            &grid_filter,
                            &game_grid,
                            &center_stack,
                            &config,
                            &detail_content,
                            &current_console,
                            &search_entry,
                        );
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Tab => {
                    *focus_pane.borrow_mut() = FocusPane::Games;
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
                gdk::Key::Menu => {
                    if !search_has_focus && !combo_has_focus {
                        Self::popup_selected_menu(
                            &game_menu,
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                        );
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::F10 if mods.contains(gdk::ModifierType::SHIFT_MASK) => {
                    if !search_has_focus && !combo_has_focus {
                        Self::popup_selected_menu(
                            &game_menu,
                            &game_grid,
                            &games,
                            &selected_game,
                            &focus_pane,
                        );
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
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
                                }
                                .favorite-badge {
                                    color: #e01b24;
                                    background-color: transparent;
                                    border-radius: 0;
                                    padding: 0;
                                    font-family: \"DejaVu Sans\", sans-serif;
                                    font-size: 18px;
                                    font-weight: 700;
                                }
                                .favorite-toggle {
                                    min-width: 0;
                                    min-height: 0;
                                    padding: 4px 10px;
                                    font-family: \"DejaVu Sans\", sans-serif;
                                    font-size: 18px;
                                }
                                .favorite-toggle.is-favorite,
                                .favorite-toggle.is-favorite label {
                                    color: #e01b24;
                                }",
                            );
                            gtk4::style_context_add_provider_for_display(
                                &gtk4::gdk::Display::default()
                                    .expect("Could not get default display"),
                                &theme_css_provider,
                                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                            );
                        } else {
                            gtk4::style_context_remove_provider_for_display(
                                &gtk4::gdk::Display::default()
                                    .expect("Could not get default display"),
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

        key_controller.connect_key_released(move |_, key, _, _| {
            let (axis, side, repeat) = match key {
                gdk::Key::Left => (&key_x_release, AxisSide::Negative, &repeat_x_release),
                gdk::Key::Right => (&key_x_release, AxisSide::Positive, &repeat_x_release),
                gdk::Key::Up => (&key_y_release, AxisSide::Negative, &repeat_y_release),
                gdk::Key::Down => (&key_y_release, AxisSide::Positive, &repeat_y_release),
                _ => return,
            };
            axis.borrow_mut().set(side, false);
            if axis.borrow().held().is_none() {
                let settings = config_release.borrow().input.sanitized();
                let now = Self::monotonic_ms(nav_started);
                repeat.borrow_mut().poll(&settings, None, now);
            }
        });

        self.window.add_controller(key_controller);
    }

    fn monotonic_ms(started: Instant) -> u64 {
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Arrow press. The first event steps immediately; later auto-repeat events
    /// only keep the key held so the shared clock can emit the slow and fast steps.
    fn press_dir(
        axis: &RefCell<AxisHold>,
        side: AxisSide,
        dir: NavDir,
        repeat: &RefCell<DirectionRepeat<NavDir>>,
        started: Instant,
        config: &Rc<RefCell<Config>>,
        pass_through: bool,
        step: impl Fn(NavDir),
    ) -> glib::Propagation {
        if pass_through {
            return glib::Propagation::Proceed;
        }
        let fresh = !axis.borrow().is_down(side);
        axis.borrow_mut().set(side, true);
        if fresh {
            let settings = config.borrow().input.sanitized();
            let now = Self::monotonic_ms(started);
            let steps = repeat.borrow_mut().poll(&settings, Some(dir), now);
            for _ in 0..steps {
                step(dir);
            }
        }
        glib::Propagation::Stop
    }

    fn axis_nav(hold: AxisHold, negative: NavDir, positive: NavDir) -> Option<NavDir> {
        match hold.held() {
            Some(AxisSide::Negative) => Some(negative),
            Some(AxisSide::Positive) => Some(positive),
            None => None,
        }
    }

    fn resolve_profile(config: &Config, game: &Game) -> Option<crate::types::EmulatorProfile> {
        let from_game = game.profile.as_deref().filter(|id| !id.is_empty());
        if let Some(profile_id) = from_game {
            return config
                .profiles
                .iter()
                .find(|p| p.id() == profile_id)
                .cloned();
        }
        let console = config.consoles.iter().find(|c| c.id == game.console)?;
        let profile_id = console.profile.as_deref().filter(|id| !id.is_empty())?;
        config
            .profiles
            .iter()
            .find(|p| p.id() == profile_id)
            .cloned()
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
        let all_games = self.all_games.clone();
        let selected_game = self.selected_game.clone();
        let detail_content = self.detail_content.clone();
        let conn = self.conn.clone();
        let updating = self.updating_grid_art.clone();
        self.grid_art_combo.connect_changed(move |combo| {
            if *updating.borrow() {
                return;
            }
            let Some(id) = combo.active_id() else { return };
            let Some(art) = GridArt::parse(id.as_str()) else {
                return;
            };
            Self::set_current_grid_art(&config, &current_console, combo, &updating, art);
            Self::redraw_current_games(
                &game_grid,
                &center_stack,
                &config,
                &games,
                &all_games,
                &selected_game,
                &detail_content,
                &conn,
                &current_console,
            );
        });
    }

    fn setup_grid_filter(&self) {
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let game_grid = self.game_grid.clone();
        let center_stack = self.center_stack.clone();
        let config = self.config.clone();
        let grid_filter = self.grid_filter.clone();
        let selected_game = self.selected_game.clone();
        let detail_content = self.detail_content.clone();
        let conn = self.conn.clone();
        let current_console = self.current_console.clone();
        let search_entry = self.search_entry.clone();
        self.grid_filter_combo.connect_changed(move |combo| {
            let Some(id) = combo.active_id() else { return };
            let Some(filter) = GridFilter::parse(id.as_str()) else {
                return;
            };
            *grid_filter.borrow_mut() = filter;
            let visible = visible_games(&all_games.borrow(), &search_entry.text(), filter);
            let library_empty = all_games.borrow().is_empty();
            Self::replace_visible_games(
                &game_grid,
                &center_stack,
                &config.borrow(),
                visible,
                library_empty,
                &games,
                &selected_game,
                &detail_content,
                &conn,
                &current_console.borrow(),
            );
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
        let Some(console_id) = current_console.borrow().clone() else {
            return;
        };
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

    fn grid_tile_media<'a>(
        game: &'a Game,
        config: &crate::config::Config,
    ) -> Option<&'a crate::types::Media> {
        let preferred = config
            .consoles
            .iter()
            .find(|console| console.id == game.console)
            .map(|console| console.grid_art)
            .unwrap_or_default();
        let have = present_kinds(&game.media);
        let kind = pick_kind(&have, preferred)?;
        game.media
            .iter()
            .find(|media| media.kind == kind && media.path.is_file())
    }

    fn append_detail_media(
        detail_content: &gtk4::Box,
        game: &Game,
        kind: MediaKind,
        caption: &str,
    ) {
        let Some(media) = game
            .media
            .iter()
            .find(|media| media.kind == kind && media.path.is_file())
        else {
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
        let detail = self.detail_content.clone();
        let selected = self.selected_game.clone();
        let window_scrape = self.window.clone();
        scrape_button.connect_clicked(move |_| {
            Self::request_scrape_selected(
                &window_scrape,
                &status,
                &running,
                &config,
                &conn,
                &games,
                &all_games,
                &grid,
                &detail,
                &selected,
            );
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
            Self::request_scrape_missing(
                &status, &running, &config, &conn, &all_games, &games, &grid, &stack, &detail,
                &selected, &search, &current,
            );
        });
    }

    fn request_scrape_selected(
        window: &adw::ApplicationWindow,
        status: &gtk4::Label,
        running: &Rc<RefCell<bool>>,
        config: &Rc<RefCell<crate::config::Config>>,
        conn: &Rc<RefCell<rusqlite::Connection>>,
        games: &Rc<RefCell<Vec<Game>>>,
        all_games: &Rc<RefCell<Vec<Game>>>,
        grid: &gtk4::FlowBox,
        detail: &gtk4::Box,
        selected: &Rc<RefCell<Option<usize>>>,
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
        Self::open_scrape_dialog(
            window, status, running, config, conn, &game, all_games, games, grid, detail, selected,
        );
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
        Self::begin_scrape(
            status, running, config, conn, targets, all_games, games, grid, stack, detail,
            selected, search,
        );
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
        Self::watch_scrape(
            rx,
            status.clone(),
            running.clone(),
            conn.clone(),
            config.clone(),
            all_games.clone(),
            games.clone(),
            grid.clone(),
            detail.clone(),
            selected.clone(),
        );
    }

    fn watch_scrape(
        rx: std::sync::mpsc::Receiver<scraper::ScrapeUpdate>,
        status: gtk4::Label,
        running: Rc<RefCell<bool>>,
        conn: Rc<RefCell<rusqlite::Connection>>,
        config: Rc<RefCell<crate::config::Config>>,
        all_games: Rc<RefCell<Vec<Game>>>,
        games: Rc<RefCell<Vec<Game>>>,
        grid: gtk4::FlowBox,
        detail: gtk4::Box,
        selected: Rc<RefCell<Option<usize>>>,
    ) {
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            // One update per tick. A long scrape can queue many saves; draining
            // them here would decode images and stall input on the main loop.
            match rx.try_recv() {
                Ok(scraper::ScrapeUpdate::Status(text)) => {
                    Self::show_status(&status, &text);
                    glib::ControlFlow::Continue
                }
                Ok(scraper::ScrapeUpdate::Saved { game_id, media }) => {
                    Self::apply_saved_media(
                        &conn, &config, &all_games, &games, &grid, &detail, &selected, &game_id,
                        &media,
                    );
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

    fn open_scrape_dialog(
        window: &adw::ApplicationWindow,
        status: &gtk4::Label,
        running: &Rc<RefCell<bool>>,
        config: &Rc<RefCell<crate::config::Config>>,
        conn: &Rc<RefCell<rusqlite::Connection>>,
        game: &Game,
        all_games: &Rc<RefCell<Vec<Game>>>,
        games: &Rc<RefCell<Vec<Game>>>,
        grid: &gtk4::FlowBox,
        detail: &gtk4::Box,
        selected: &Rc<RefCell<Option<usize>>>,
    ) {
        let query = Self::scrape_query(game);
        let console_id = game.console.clone();
        let game = game.clone();
        let status = status.clone();
        let running = running.clone();
        let config = config.clone();
        let conn = conn.clone();
        let all_games = all_games.clone();
        let games = games.clone();
        let grid = grid.clone();
        let detail = detail.clone();
        let selected = selected.clone();
        crate::dialogs::open_game_scrape(
            window,
            &query,
            &console_id,
            config.clone(),
            Rc::new(move |candidate| {
                Self::begin_apply(
                    &status,
                    &running,
                    &config,
                    &conn,
                    game.clone(),
                    candidate,
                    &all_games,
                    &games,
                    &grid,
                    &detail,
                    &selected,
                );
            }),
        );
    }

    fn scrape_query(game: &Game) -> String {
        let title = game.title.trim();
        if title.is_empty() {
            scanner::derive_title(&game.rom)
        } else {
            title.to_string()
        }
    }

    fn begin_apply(
        status: &gtk4::Label,
        running: &Rc<RefCell<bool>>,
        config: &Rc<RefCell<crate::config::Config>>,
        conn: &Rc<RefCell<rusqlite::Connection>>,
        game: Game,
        candidate: scraper::ScrapeCandidate,
        all_games: &Rc<RefCell<Vec<Game>>>,
        games: &Rc<RefCell<Vec<Game>>>,
        grid: &gtk4::FlowBox,
        detail: &gtk4::Box,
        selected: &Rc<RefCell<Option<usize>>>,
    ) {
        if *running.borrow() {
            Self::show_status(status, "A scrape is already running.");
            return;
        }
        *running.borrow_mut() = true;
        Self::show_status(status, "Scraping artwork…");
        let scraper_config = config.borrow().scraper.clone();
        let rx = scraper::spawn_apply_candidate(
            game,
            candidate,
            scraper_config,
            scraper::fixture_dir_from_env(),
        );
        Self::watch_scrape(
            rx,
            status.clone(),
            running.clone(),
            conn.clone(),
            config.clone(),
            all_games.clone(),
            games.clone(),
            grid.clone(),
            detail.clone(),
            selected.clone(),
        );
    }

    fn setup_game_menu(&self) {
        let on_action = {
            let window = self.window.clone();
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
            let grid_filter = self.grid_filter.clone();
            let current = self.current_console.clone();
            move |action| {
                Self::perform_game_action(
                    action,
                    &window,
                    &status,
                    &running,
                    &config,
                    &conn,
                    &games,
                    &all_games,
                    &grid,
                    &stack,
                    &detail,
                    &selected,
                    &search,
                    &grid_filter,
                    &current,
                );
            }
        };
        let built = Self::game_menu_popover(Rc::new(on_action));
        if let Some(list) = built.child() {
            built.set_child(None::<&gtk4::Widget>);
            self.game_menu.set_child(Some(&list));
        }
        self.game_menu.set_has_arrow(false);

        let gesture = gtk4::GestureClick::new();
        gesture.set_button(gtk4::gdk::BUTTON_SECONDARY);
        let grid = self.game_grid.clone();
        let menu = self.game_menu.clone();
        let games = self.games.clone();
        let selected = self.selected_game.clone();
        let focus = self.focus_pane.clone();
        let detail = self.detail_content.clone();
        let config = self.config.clone();
        let conn = self.conn.clone();
        gesture.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let Some(child) = grid.child_at_pos(x as i32, y as i32) else {
                return;
            };
            let index = child.index();
            Self::select_game_index(
                &grid, &games, &selected, &focus, &detail, &config, &conn, index,
            );
            Self::popup_game_menu(&menu, &grid, index);
        });
        self.game_grid.add_controller(gesture);
    }

    fn game_menu_popover(on_action: Rc<dyn Fn(GameAction)>) -> gtk4::Popover {
        let popover = gtk4::Popover::new();
        popover.set_has_arrow(false);
        let list = gtk4::ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::Single);
        list.set_width_request(180);
        for action in GameAction::ALL {
            let label = gtk4::Label::new(Some(action.label()));
            label.set_halign(gtk4::Align::Start);
            label.set_margin_top(8);
            label.set_margin_bottom(8);
            label.set_margin_start(12);
            label.set_margin_end(12);
            list.append(&label);
        }
        list.connect_row_activated(move |list, row| {
            if let Some(popover) = list
                .ancestor(gtk4::Popover::static_type())
                .and_then(|widget| widget.downcast::<gtk4::Popover>().ok())
            {
                popover.popdown();
            }
            if let Some(action) = GameAction::from_index(row.index()) {
                on_action(action);
            }
        });
        popover.set_child(Some(&list));
        popover
    }

    fn perform_game_action(
        action: GameAction,
        window: &adw::ApplicationWindow,
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
        grid_filter: &Rc<RefCell<GridFilter>>,
        current: &Rc<RefCell<Option<String>>>,
    ) {
        let Some(index) = *selected.borrow() else {
            Self::show_status(status, "Select a game.");
            return;
        };
        let Some(game) = games.borrow().get(index).cloned() else {
            Self::show_status(status, "Select a game.");
            return;
        };
        match action {
            GameAction::Scrape => Self::open_scrape_dialog(
                window, status, running, config, conn, &game, all_games, games, grid, detail,
                selected,
            ),
            GameAction::Rename => {
                let status = status.clone();
                let conn = conn.clone();
                let games = games.clone();
                let all_games = all_games.clone();
                let grid = grid.clone();
                let stack = stack.clone();
                let detail = detail.clone();
                let selected = selected.clone();
                let search = search.clone();
                let grid_filter = grid_filter.clone();
                let current = current.clone();
                let config = config.clone();
                let game_id = game.id.clone();
                crate::dialogs::open_rename(window, &game.title, move |title| {
                    Self::apply_rename(
                        &status,
                        &conn,
                        &config,
                        &games,
                        &all_games,
                        &grid,
                        &stack,
                        &detail,
                        &selected,
                        &search,
                        &grid_filter,
                        &current,
                        &game_id,
                        &title,
                    );
                });
            }
            GameAction::Delete => {
                let status = status.clone();
                let conn = conn.clone();
                let games = games.clone();
                let all_games = all_games.clone();
                let grid = grid.clone();
                let stack = stack.clone();
                let detail = detail.clone();
                let selected = selected.clone();
                let search = search.clone();
                let grid_filter = grid_filter.clone();
                let current = current.clone();
                let config = config.clone();
                let heading = game.title.clone();
                crate::dialogs::open_delete(window, &heading, move |options| {
                    Self::apply_delete(
                        &status,
                        &conn,
                        &config,
                        &games,
                        &all_games,
                        &grid,
                        &stack,
                        &detail,
                        &selected,
                        &search,
                        &grid_filter,
                        &current,
                        &game,
                        options,
                    );
                });
            }
        }
    }

    fn apply_rename(
        status: &gtk4::Label,
        conn: &Rc<RefCell<rusqlite::Connection>>,
        config: &Rc<RefCell<crate::config::Config>>,
        games: &Rc<RefCell<Vec<Game>>>,
        all_games: &Rc<RefCell<Vec<Game>>>,
        grid: &gtk4::FlowBox,
        stack: &gtk4::Stack,
        detail: &gtk4::Box,
        selected: &Rc<RefCell<Option<usize>>>,
        search: &gtk4::SearchEntry,
        grid_filter: &Rc<RefCell<GridFilter>>,
        current: &Rc<RefCell<Option<String>>>,
        game_id: &str,
        title: &str,
    ) {
        if database::set_game_title(&conn.borrow(), &game_id.to_string(), title).is_err() {
            Self::show_status(status, "Could not rename the game.");
            return;
        }
        Self::write_title(all_games, game_id, title);
        Self::write_title(games, game_id, title);
        let visible = visible_games(&all_games.borrow(), &search.text(), *grid_filter.borrow());
        let still_shown = visible.iter().any(|game| game.id == game_id);
        if still_shown {
            if let Some(index) = games.borrow().iter().position(|game| game.id == game_id) {
                Self::set_tile_title(grid, index, title);
                if let Some(game) = games.borrow().get(index).cloned() {
                    Self::update_game_details(detail, &game, &config.borrow(), conn);
                }
            }
        } else {
            let library_empty = all_games.borrow().is_empty();
            Self::replace_visible_games(
                grid,
                stack,
                &config.borrow(),
                visible,
                library_empty,
                games,
                selected,
                detail,
                conn,
                &current.borrow(),
            );
        }
    }

    fn apply_delete(
        status: &gtk4::Label,
        conn: &Rc<RefCell<rusqlite::Connection>>,
        config: &Rc<RefCell<crate::config::Config>>,
        games: &Rc<RefCell<Vec<Game>>>,
        all_games: &Rc<RefCell<Vec<Game>>>,
        grid: &gtk4::FlowBox,
        stack: &gtk4::Stack,
        detail: &gtk4::Box,
        selected: &Rc<RefCell<Option<usize>>>,
        search: &gtk4::SearchEntry,
        grid_filter: &Rc<RefCell<GridFilter>>,
        current: &Rc<RefCell<Option<String>>>,
        game: &Game,
        options: crate::types::DeleteOptions,
    ) {
        if let Err(err) = database::delete_game(&conn.borrow(), &game.id) {
            Self::show_status(
                status,
                &format!("Could not remove the game from the library: {err}"),
            );
            return;
        }
        let mut notes = Vec::new();
        if options.rom_file {
            if let Err(err) = scraper::delete_rom_file(&game.rom) {
                notes.push(format!("ROM file: {err}"));
            }
        }
        if options.scraped_assets {
            match scraper::media_root() {
                Ok(root) => {
                    if let Err(err) = scraper::delete_cached_assets(&root, game) {
                        notes.push(format!("artwork: {err}"));
                    }
                }
                Err(err) => notes.push(format!("artwork: {err}")),
            }
        }
        let game_id = game.id.clone();
        all_games.borrow_mut().retain(|item| item.id != game_id);
        let visible = visible_games(&all_games.borrow(), &search.text(), *grid_filter.borrow());
        let library_empty = all_games.borrow().is_empty();
        Self::replace_visible_games(
            grid,
            stack,
            &config.borrow(),
            visible,
            library_empty,
            games,
            selected,
            detail,
            conn,
            &current.borrow(),
        );
        let message = if notes.is_empty() {
            format!("Removed {} from the library.", game.title)
        } else {
            format!(
                "Removed {} from the library. {}",
                game.title,
                notes.join(" ")
            )
        };
        Self::show_status(status, &message);
    }

    fn write_title(games: &Rc<RefCell<Vec<Game>>>, id: &str, title: &str) {
        if let Some(game) = games.borrow_mut().iter_mut().find(|game| game.id == id) {
            game.title = title.to_string();
        }
    }

    fn set_tile_title(grid: &gtk4::FlowBox, index: usize, title: &str) {
        let Some(flow_child) = grid.child_at_index(index as i32) else {
            return;
        };
        let Some(tile) = Self::tile_box(&flow_child) else {
            return;
        };
        let Some(label) = Self::find_widget_name(tile.upcast_ref(), "game-title")
            .and_then(|widget| widget.downcast::<gtk4::Label>().ok())
        else {
            return;
        };
        label.set_text(title);
    }

    fn select_game_index(
        grid: &gtk4::FlowBox,
        games: &Rc<RefCell<Vec<Game>>>,
        selected: &Rc<RefCell<Option<usize>>>,
        focus: &Rc<RefCell<FocusPane>>,
        detail: &gtk4::Box,
        config: &Rc<RefCell<crate::config::Config>>,
        conn: &Rc<RefCell<rusqlite::Connection>>,
        index: i32,
    ) {
        let Some(child) = grid.child_at_index(index) else {
            return;
        };
        *focus.borrow_mut() = FocusPane::Games;
        grid.select_child(&child);
        *selected.borrow_mut() = Some(index as usize);
        if let Some(game) = games.borrow().get(index as usize) {
            Self::update_game_details(detail, game, &config.borrow(), conn);
        }
    }

    fn popup_selected_menu(
        menu: &gtk4::Popover,
        grid: &gtk4::FlowBox,
        games: &Rc<RefCell<Vec<Game>>>,
        selected: &Rc<RefCell<Option<usize>>>,
        focus: &Rc<RefCell<FocusPane>>,
    ) {
        if *focus.borrow() != FocusPane::Games {
            return;
        }
        let Some(index) = *selected.borrow() else {
            return;
        };
        if games.borrow().get(index).is_none() {
            return;
        }
        Self::popup_game_menu(menu, grid, index as i32);
    }

    fn popup_game_menu(menu: &gtk4::Popover, grid: &gtk4::FlowBox, index: i32) {
        let Some(child) = grid.child_at_index(index) else {
            return;
        };
        let child_widget = child.clone().upcast::<gtk4::Widget>();
        let same_parent = menu.parent().is_some_and(|parent| parent == child_widget);
        if menu.is_visible() {
            menu.popdown();
        }
        if !same_parent {
            if menu.parent().is_some() {
                menu.unparent();
            }
            menu.set_parent(&child);
        }
        let rect = child.allocation();
        menu.set_pointing_to(Some(&gdk::Rectangle::new(
            0,
            0,
            rect.width().max(1),
            rect.height().max(1),
        )));
        menu.popup();
        if let Some(list) = menu.child().and_downcast::<gtk4::ListBox>() {
            if let Some(row) = list.row_at_index(0) {
                list.select_row(Some(&row));
                row.grab_focus();
            }
        }
    }

    fn move_menu(menu: &gtk4::Popover, dir: NavDir) {
        let Some(list) = menu.child().and_downcast::<gtk4::ListBox>() else {
            return;
        };
        let current = list.selected_row().map(|row| row.index()).unwrap_or(0);
        let next = match dir {
            NavDir::Up => current - 1,
            NavDir::Down => current + 1,
            NavDir::Left | NavDir::Right => return,
        };
        if next < 0 {
            return;
        }
        if let Some(row) = list.row_at_index(next) {
            list.select_row(Some(&row));
            row.grab_focus();
        }
    }

    fn activate_menu(menu: &gtk4::Popover) {
        let Some(list) = menu.child().and_downcast::<gtk4::ListBox>() else {
            return;
        };
        let row = list.selected_row().or_else(|| list.row_at_index(0));
        if let Some(row) = row {
            row.activate();
        }
    }

    fn detach_popovers(widget: &gtk4::Widget) {
        let mut found = Vec::new();
        Self::collect_popovers(widget, &mut found);
        for popover in found {
            popover.popdown();
            if popover.parent().is_some() {
                popover.unparent();
            }
        }
    }

    fn collect_popovers(widget: &gtk4::Widget, out: &mut Vec<gtk4::Popover>) {
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Ok(popover) = current.clone().downcast::<gtk4::Popover>() {
                out.push(popover);
            } else {
                Self::collect_popovers(&current, out);
            }
            child = current.next_sibling();
        }
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

    fn refresh_grid_tile(
        grid: &gtk4::FlowBox,
        index: usize,
        game: &Game,
        config: &crate::config::Config,
    ) {
        let Some(flow_child) = grid.child_at_index(index as i32) else {
            return;
        };
        let Some(tile) = Self::tile_box(&flow_child) else {
            return;
        };
        let Some(frame) = Self::tile_art_frame(&tile) else {
            return;
        };
        Self::set_tile_art(&frame, game, config);
    }

    fn tile_box(flow_child: &gtk4::FlowBoxChild) -> Option<gtk4::Box> {
        flow_child
            .child()?
            .downcast::<gtk4::Overlay>()
            .ok()?
            .child()?
            .downcast::<gtk4::Box>()
            .ok()
    }

    fn tile_art_frame(tile: &gtk4::Box) -> Option<gtk4::Frame> {
        tile.first_child()?.downcast::<gtk4::Frame>().ok()
    }

    fn show_status(status: &gtk4::Label, text: &str) {
        status.set_text(text);
        status.set_visible(!text.is_empty());
    }

    #[allow(clippy::too_many_arguments)]
    fn redraw_current_games(
        grid: &gtk4::FlowBox,
        stack: &gtk4::Stack,
        config: &Rc<RefCell<Config>>,
        games: &Rc<RefCell<Vec<Game>>>,
        all_games: &Rc<RefCell<Vec<Game>>>,
        selected: &Rc<RefCell<Option<usize>>>,
        detail: &gtk4::Box,
        conn: &Rc<RefCell<Connection>>,
        current_console: &Rc<RefCell<Option<String>>>,
    ) {
        let visible = games.borrow().clone();
        let library_empty = all_games.borrow().is_empty();
        Self::replace_visible_games(
            grid,
            stack,
            &config.borrow(),
            visible,
            library_empty,
            games,
            selected,
            detail,
            conn,
            &current_console.borrow(),
        );
    }

    fn move_grid(
        grid: &gtk4::FlowBox,
        games: &Rc<RefCell<Vec<Game>>>,
        selected: &Rc<RefCell<Option<usize>>>,
        focus_pane: &Rc<RefCell<FocusPane>>,
        dir: NavDir,
        update_details: &impl Fn(usize),
    ) {
        *focus_pane.borrow_mut() = FocusPane::Games;
        let len = games.borrow().len() as i32;
        let current = grid.selected_children().first().map(|child| child.index());
        let Some(next) = gamepad::grid_step(current, dir, len, Self::flow_columns(grid)) else {
            return;
        };
        if let Some(child) = grid.child_at_index(next) {
            grid.select_child(&child);
            child.grab_focus();
            *selected.borrow_mut() = Some(next as usize);
            update_details(next as usize);
        }
    }

    /// Live column count from the FlowBox layout. GTK keeps `cur_children_per_line`
    /// private, so this reads the first line it already placed: those children
    /// share an allocation y. Hiding the details pane or resizing the window
    /// changes that y-run, and the next up/down uses the new length.
    fn flow_columns(grid: &gtk4::FlowBox) -> i32 {
        let fallback = i32::try_from(grid.max_children_per_line())
            .unwrap_or(1)
            .max(1);
        let mut tiles = Vec::new();
        let mut index = 0;
        while let Some(child) = grid.child_at_index(index) {
            let rect = child.allocation();
            tiles.push(gamepad::FlowTile {
                y: rect.y(),
                height: rect.height(),
            });
            index += 1;
            if tiles.len() > 1
                && tiles
                    .last()
                    .is_some_and(|tile| tile.y != tiles[0].y || tile.height <= 0)
            {
                break;
            }
        }
        gamepad::line_columns(&tiles, fallback)
    }

    fn list_len(list: &gtk4::ListBox) -> i32 {
        let mut count = 0;
        while list.row_at_index(count).is_some() {
            count += 1;
        }
        count
    }

    /// `FocusPane` is `Copy`. The `Ref` ends before `body` runs, so `body`
    /// may `borrow_mut` this cell. `row-selected` does that from `move_systems`.
    fn with_focus_pane(focus: &RefCell<FocusPane>, body: impl FnOnce(FocusPane)) {
        let pane = *focus.borrow();
        body(pane);
    }

    fn move_systems(list: &gtk4::ListBox, dir: NavDir) {
        let current = list.selected_row().map(|row| row.index());
        let Some(next) = gamepad::list_step(current, dir, Self::list_len(list)) else {
            return;
        };
        if let Some(row) = list.row_at_index(next) {
            list.select_row(Some(&row));
            row.grab_focus();
        }
    }

    fn enter_games(
        grid: &gtk4::FlowBox,
        games: &Rc<RefCell<Vec<Game>>>,
        selected: &Rc<RefCell<Option<usize>>>,
        focus_pane: &Rc<RefCell<FocusPane>>,
        detail: &gtk4::Box,
        config: &Rc<RefCell<Config>>,
        conn: &Rc<RefCell<Connection>>,
    ) {
        if grid.child_at_index(0).is_none() {
            return;
        }
        *focus_pane.borrow_mut() = FocusPane::Games;
        let idx = selected
            .borrow()
            .filter(|idx| grid.child_at_index(*idx as i32).is_some())
            .unwrap_or(0);
        if let Some(child) = grid.child_at_index(idx as i32) {
            grid.select_child(&child);
            child.grab_focus();
            *selected.borrow_mut() = Some(idx);
            if let Some(game) = games.borrow().get(idx) {
                Self::update_game_details(detail, game, &config.borrow(), conn);
            }
        }
    }

    fn write_favorite(games: &Rc<RefCell<Vec<Game>>>, id: &str, favorite: bool) {
        if let Some(game) = games.borrow_mut().iter_mut().find(|game| game.id == id) {
            game.favorite = favorite;
        }
    }

    fn set_tile_favorite(grid: &gtk4::FlowBox, index: usize, favorite: bool) {
        let Some(flow_child) = grid.child_at_index(index as i32) else {
            return;
        };
        let Some(overlay) = flow_child.child().and_downcast::<gtk4::Overlay>() else {
            return;
        };
        let mut widget = overlay.first_child();
        while let Some(current) = widget {
            if current.widget_name() == "favorite-badge" {
                current.set_visible(favorite);
                return;
            }
            widget = current.next_sibling();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn toggle_favorite(
        conn: &Rc<RefCell<Connection>>,
        games: &Rc<RefCell<Vec<Game>>>,
        all_games: &Rc<RefCell<Vec<Game>>>,
        selected: &Rc<RefCell<Option<usize>>>,
        grid_filter: &Rc<RefCell<GridFilter>>,
        grid: &gtk4::FlowBox,
        stack: &gtk4::Stack,
        config: &Rc<RefCell<Config>>,
        detail: &gtk4::Box,
        current_console: &Rc<RefCell<Option<String>>>,
        search: &gtk4::SearchEntry,
    ) {
        let Some(idx) = *selected.borrow() else {
            return;
        };
        let Some(game) = games.borrow().get(idx).cloned() else {
            return;
        };
        let next = !game.favorite;
        if database::set_favorite(&conn.borrow(), &game.id, next).is_err() {
            return;
        }
        Self::write_favorite(all_games, &game.id, next);
        Self::write_favorite(games, &game.id, next);
        if *grid_filter.borrow() == GridFilter::Favorites && !next {
            let visible = visible_games(&all_games.borrow(), &search.text(), GridFilter::Favorites);
            let library_empty = all_games.borrow().is_empty();
            Self::replace_visible_games(
                grid,
                stack,
                &config.borrow(),
                visible,
                library_empty,
                games,
                selected,
                detail,
                conn,
                &current_console.borrow(),
            );
            return;
        }
        Self::set_tile_favorite(grid, idx, next);
        Self::set_detail_favorite(detail, next);
    }

    fn favorite_toggle_button(favorite: bool) -> gtk4::Button {
        let button = gtk4::Button::new();
        button.set_widget_name("favorite-toggle");
        button.add_css_class("favorite-toggle");
        button.set_tooltip_text(Some("Toggle favorite"));
        button.set_action_name(Some("win.toggle-favorite"));
        button.set_hexpand(false);
        button.set_valign(gtk4::Align::Center);
        Self::apply_favorite_toggle(&button, favorite);
        button
    }

    fn apply_favorite_toggle(button: &gtk4::Button, favorite: bool) {
        button.set_label(if favorite { "♥" } else { "♡" });
        if favorite {
            button.add_css_class("is-favorite");
        } else {
            button.remove_css_class("is-favorite");
        }
    }

    fn set_detail_favorite(detail: &gtk4::Box, favorite: bool) {
        let Some(widget) = Self::find_widget_name(detail.upcast_ref(), "favorite-toggle") else {
            return;
        };
        let Ok(button) = widget.downcast::<gtk4::Button>() else {
            return;
        };
        Self::apply_favorite_toggle(&button, favorite);
    }

    fn find_widget_name(root: &gtk4::Widget, name: &str) -> Option<gtk4::Widget> {
        if root.widget_name() == name {
            return Some(root.clone());
        }
        let mut child = root.first_child();
        while let Some(current) = child {
            if let Some(found) = Self::find_widget_name(&current, name) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }

    fn setup_favorite_action(&self) {
        let action = gtk4::gio::SimpleAction::new("toggle-favorite", None);
        let conn = self.conn.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let selected_game = self.selected_game.clone();
        let grid_filter = self.grid_filter.clone();
        let game_grid = self.game_grid.clone();
        let center_stack = self.center_stack.clone();
        let config = self.config.clone();
        let detail_content = self.detail_content.clone();
        let current_console = self.current_console.clone();
        let search_entry = self.search_entry.clone();
        action.connect_activate(move |_, _| {
            Self::toggle_favorite(
                &conn,
                &games,
                &all_games,
                &selected_game,
                &grid_filter,
                &game_grid,
                &center_stack,
                &config,
                &detail_content,
                &current_console,
                &search_entry,
            );
        });
        self.window.add_action(&action);
    }

    fn launch_selected(
        window: &adw::ApplicationWindow,
        config: &Rc<RefCell<Config>>,
        conn: &Rc<RefCell<Connection>>,
        detail: &gtk4::Box,
        games: &Rc<RefCell<Vec<Game>>>,
        selected: &Rc<RefCell<Option<usize>>>,
    ) {
        let Some(idx) = *selected.borrow() else {
            return;
        };
        let Some(game) = games.borrow().get(idx).cloned() else {
            return;
        };
        Self::launch_game(window, config, conn, detail, &game);
    }

    fn launch_game(
        window: &adw::ApplicationWindow,
        config: &Rc<RefCell<Config>>,
        conn: &Rc<RefCell<Connection>>,
        detail: &gtk4::Box,
        game: &Game,
    ) {
        let Some(profile) = Self::resolve_profile(&config.borrow(), game) else {
            Self::alert_missing_profile(window);
            return;
        };
        let Ok(mut child) = launcher::launch_game_tracked(&profile, &game.rom) else {
            return;
        };
        let game_id = game.id.clone();
        let game_console = game.console.clone();
        let conn_for_update = conn.clone();
        let detail_for_update = detail.clone();
        let config_for_update = config.clone();
        let start_time = Instant::now();
        let (sender, receiver) = std::sync::mpsc::channel();

        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            if let Ok((game_id, game_console, elapsed)) = receiver.try_recv() {
                let _ =
                    database::increment_play_stats(&conn_for_update.borrow(), &game_id, elapsed);
                let _ = database::update_last_played(&conn_for_update.borrow(), &game_id);
                if let Ok(updated_games) =
                    database::load_games(&conn_for_update.borrow(), Some(&game_console))
                {
                    if let Some(updated_game) = updated_games.iter().find(|g| g.id == game_id) {
                        Self::update_game_details(
                            &detail_for_update,
                            updated_game,
                            &config_for_update.borrow(),
                            &conn_for_update,
                        );
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

    fn pad_blocked(
        window: &adw::ApplicationWindow,
        search: &gtk4::SearchEntry,
        combos: &[&gtk4::ComboBoxText],
    ) -> bool {
        if !window.is_active() {
            return true;
        }
        let Some(focused) = gtk4::prelude::RootExt::focus(window) else {
            return false;
        };
        let widget = focused.upcast_ref::<gtk4::Widget>();
        if widget == search.upcast_ref::<gtk4::Widget>() || search.is_ancestor(&focused) {
            return true;
        }
        combos.iter().any(|combo| {
            widget == combo.upcast_ref::<gtk4::Widget>() || combo.is_ancestor(&focused)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn step_nav(
        console_list: &gtk4::ListBox,
        game_grid: &gtk4::FlowBox,
        games: &Rc<RefCell<Vec<Game>>>,
        selected_game: &Rc<RefCell<Option<usize>>>,
        focus_pane: &Rc<RefCell<FocusPane>>,
        detail_content: &gtk4::Box,
        config: &Rc<RefCell<Config>>,
        conn: &Rc<RefCell<Connection>>,
        dir: NavDir,
    ) {
        let update_details = |idx: usize| {
            if let Some(game) = games.borrow().get(idx) {
                Self::update_game_details(detail_content, game, &config.borrow(), conn);
            }
        };
        // Copy the pane out before any GTK call. `move_systems` selects a
        // row, which emits `row-selected` on this same stack and writes
        // `focus_pane` again. A live `borrow()` here aborts the process.
        Self::with_focus_pane(focus_pane, |pane| match pane {
            FocusPane::Systems => Self::move_systems(console_list, dir),
            FocusPane::Games => Self::move_grid(
                game_grid,
                games,
                selected_game,
                focus_pane,
                dir,
                &update_details,
            ),
        });
    }

    fn setup_direction_repeat(
        &self,
        key_x: Rc<RefCell<AxisHold>>,
        key_y: Rc<RefCell<AxisHold>>,
        repeat_x: Rc<RefCell<DirectionRepeat<NavDir>>>,
        repeat_y: Rc<RefCell<DirectionRepeat<NavDir>>>,
        started: Instant,
    ) {
        let mut gilrs = match gilrs::Gilrs::new() {
            Ok(gilrs) => Some(gilrs),
            Err(err) => {
                eprintln!("Gamepad unavailable: {err}");
                None
            }
        };
        let mut pad = gamepad::PadHeld::default();
        let window = self.window.clone();
        let game_grid = self.game_grid.clone();
        let console_list = self.console_list.clone();
        let games = self.games.clone();
        let all_games = self.all_games.clone();
        let selected_game = self.selected_game.clone();
        let focus_pane = self.focus_pane.clone();
        let config = self.config.clone();
        let conn = self.conn.clone();
        let detail_content = self.detail_content.clone();
        let center_stack = self.center_stack.clone();
        let grid_filter = self.grid_filter.clone();
        let current_console = self.current_console.clone();
        let search_entry = self.search_entry.clone();
        let grid_art_combo = self.grid_art_combo.clone();
        let grid_filter_combo = self.grid_filter_combo.clone();
        let game_menu = self.game_menu.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            let blocked = Self::pad_blocked(
                &window,
                &search_entry,
                &[&grid_art_combo, &grid_filter_combo],
            );
            if let Some(gilrs) = gilrs.as_mut() {
                while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
                    let action = pad.apply(&event);
                    let Some(action) = action else {
                        continue;
                    };
                    match Self::pad_target(&window, &game_menu) {
                        PadTarget::Menu => Self::menu_pad_action(&game_menu, action),
                        PadTarget::Dialog(dialog) => Self::dialog_pad_action(&dialog, action),
                        PadTarget::Grid => {
                            if blocked {
                                continue;
                            }
                            Self::with_focus_pane(&focus_pane, |pane| match action {
                                PadAction::Confirm => match pane {
                                    FocusPane::Systems => Self::enter_games(
                                        &game_grid,
                                        &games,
                                        &selected_game,
                                        &focus_pane,
                                        &detail_content,
                                        &config,
                                        &conn,
                                    ),
                                    FocusPane::Games => Self::launch_selected(
                                        &window,
                                        &config,
                                        &conn,
                                        &detail_content,
                                        &games,
                                        &selected_game,
                                    ),
                                },
                                PadAction::Back => {
                                    if pane == FocusPane::Games {
                                        *focus_pane.borrow_mut() = FocusPane::Systems;
                                        if let Some(row) = console_list.selected_row() {
                                            row.grab_focus();
                                        } else {
                                            console_list.grab_focus();
                                        }
                                    }
                                }
                                PadAction::Favorite => {
                                    if pane == FocusPane::Games {
                                        Self::toggle_favorite(
                                            &conn,
                                            &games,
                                            &all_games,
                                            &selected_game,
                                            &grid_filter,
                                            &game_grid,
                                            &center_stack,
                                            &config,
                                            &detail_content,
                                            &current_console,
                                            &search_entry,
                                        );
                                    }
                                }
                                PadAction::Menu => {
                                    if pane == FocusPane::Games {
                                        Self::popup_selected_menu(
                                            &game_menu,
                                            &game_grid,
                                            &games,
                                            &selected_game,
                                            &focus_pane,
                                        );
                                    }
                                }
                            });
                        }
                    }
                }
            }
            let target = Self::pad_target(&window, &game_menu);
            let use_grid = matches!(target, PadTarget::Grid) && !blocked;
            if !use_grid {
                *key_x.borrow_mut() = AxisHold::default();
                *key_y.borrow_mut() = AxisHold::default();
            }
            let settings = config.borrow().input.sanitized();
            let now = Self::monotonic_ms(started);
            let held_x = if use_grid {
                Self::axis_nav(*key_x.borrow(), NavDir::Left, NavDir::Right)
                    .or_else(|| pad.horizontal())
            } else if matches!(target, PadTarget::Grid) {
                None
            } else {
                pad.horizontal()
            };
            let held_y = if use_grid {
                Self::axis_nav(*key_y.borrow(), NavDir::Up, NavDir::Down).or_else(|| pad.vertical())
            } else if matches!(target, PadTarget::Grid) {
                None
            } else {
                pad.vertical()
            };
            let steps_x = repeat_x.borrow_mut().poll(&settings, held_x, now);
            let steps_y = repeat_y.borrow_mut().poll(&settings, held_y, now);
            match target {
                PadTarget::Menu => {
                    if let Some(dir) = held_y {
                        for _ in 0..steps_y {
                            Self::move_menu(&game_menu, dir);
                        }
                    }
                }
                PadTarget::Dialog(dialog) => {
                    if let Some(dir) = held_x {
                        for _ in 0..steps_x {
                            dialog.child_focus(Self::nav_direction(dir));
                        }
                    }
                    if let Some(dir) = held_y {
                        for _ in 0..steps_y {
                            dialog.child_focus(Self::nav_direction(dir));
                        }
                    }
                }
                PadTarget::Grid if use_grid => {
                    if let Some(dir) = held_x {
                        for _ in 0..steps_x {
                            Self::step_nav(
                                &console_list,
                                &game_grid,
                                &games,
                                &selected_game,
                                &focus_pane,
                                &detail_content,
                                &config,
                                &conn,
                                dir,
                            );
                        }
                    }
                    if let Some(dir) = held_y {
                        for _ in 0..steps_y {
                            Self::step_nav(
                                &console_list,
                                &game_grid,
                                &games,
                                &selected_game,
                                &focus_pane,
                                &detail_content,
                                &config,
                                &conn,
                                dir,
                            );
                        }
                    }
                }
                PadTarget::Grid => {}
            }
            glib::ControlFlow::Continue
        });
    }

    fn pad_target(main: &adw::ApplicationWindow, menu: &gtk4::Popover) -> PadTarget {
        if menu.is_visible() {
            return PadTarget::Menu;
        }
        if let Some(active) = main.application().and_then(|app| app.active_window()) {
            if active.upcast_ref::<gtk4::Widget>() != main.upcast_ref::<gtk4::Widget>() {
                return PadTarget::Dialog(active);
            }
        }
        PadTarget::Grid
    }

    fn menu_pad_action(menu: &gtk4::Popover, action: PadAction) {
        match action {
            PadAction::Menu | PadAction::Back => menu.popdown(),
            PadAction::Confirm => Self::activate_menu(menu),
            PadAction::Favorite => {}
        }
    }

    fn dialog_pad_action(dialog: &gtk4::Window, action: PadAction) {
        match action {
            PadAction::Back => dialog.close(),
            PadAction::Confirm => {
                if let Some(widget) = gtk4::prelude::RootExt::focus(dialog) {
                    let _ = widget.activate();
                }
            }
            PadAction::Menu | PadAction::Favorite => {}
        }
    }

    fn nav_direction(dir: NavDir) -> gtk4::DirectionType {
        match dir {
            NavDir::Left => gtk4::DirectionType::Left,
            NavDir::Right => gtk4::DirectionType::Right,
            NavDir::Up => gtk4::DirectionType::Up,
            NavDir::Down => gtk4::DirectionType::Down,
        }
    }

    pub fn show(&self) {
        self.window.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamepad::NavDir;

    #[test]
    fn systems_move_can_rewrite_focus_while_the_tick_runs() {
        let focus = RefCell::new(FocusPane::Systems);
        App::with_focus_pane(&focus, |pane| {
            assert_eq!(pane, FocusPane::Systems);
            // `row-selected` does this synchronously from `ListBox::select_row`.
            *focus.borrow_mut() = FocusPane::Systems;
        });
        assert_eq!(*focus.borrow(), FocusPane::Systems);
    }

    #[test]
    fn stick_move_selects_the_next_system_without_reborrowing() {
        gtk4::init().expect("gtk init");
        let list = gtk4::ListBox::new();
        list.append(&gtk4::Label::new(Some("nes")));
        list.append(&gtk4::Label::new(Some("snes")));
        let focus = Rc::new(RefCell::new(FocusPane::Systems));
        let focus_in_handler = focus.clone();
        list.connect_row_selected(move |_, _| {
            *focus_in_handler.borrow_mut() = FocusPane::Systems;
        });
        list.select_row(list.row_at_index(0).as_ref());

        App::with_focus_pane(&focus, |pane| {
            assert_eq!(pane, FocusPane::Systems);
            App::move_systems(&list, NavDir::Down);
        });

        assert_eq!(list.selected_row().map(|row| row.index()), Some(1));

        // GTK init is process-wide and thread-affine. A second test that calls
        // `gtk4::init` aborts, so the other GTK checks share this init.
        flow_columns_follows_the_allocated_line();
        favorite_badge_toggles_on_the_card_overlay();
        detail_pane_puts_play_above_stats_and_rom_at_the_bottom();
        detail_column_stays_fixed_when_play_expands();
        favorite_toggle_updates_badge_and_detail_heart();
        game_menu_activates_rename();
        rebuilding_the_grid_unparents_the_game_menu();
        controller_reaches_context_dialog_controls();
    }

    fn game_menu_activates_rename() {
        let seen = Rc::new(RefCell::new(None));
        let popover = App::game_menu_popover({
            let seen = seen.clone();
            Rc::new(move |action| *seen.borrow_mut() = Some(action))
        });
        let list = popover.child().and_downcast::<gtk4::ListBox>().unwrap();
        let labels: Vec<String> = (0..3)
            .map(|index| {
                list.row_at_index(index)
                    .unwrap()
                    .child()
                    .and_downcast::<gtk4::Label>()
                    .unwrap()
                    .text()
                    .to_string()
            })
            .collect();
        assert_eq!(labels, vec!["Scrape…", "Rename…", "Delete…"]);
        list.row_at_index(1).unwrap().activate();
        assert_eq!(*seen.borrow(), Some(GameAction::Rename));
    }

    fn rebuilding_the_grid_unparents_the_game_menu() {
        use crate::config::Config;
        use crate::types::{Console, GridArt, MediaToggles};

        let config = Config {
            consoles: vec![Console {
                id: "snes".into(),
                name: "Super Nintendo".into(),
                rom_dirs: Vec::new(),
                extensions: Vec::new(),
                profile: None,
                grid_art: GridArt::BoxArt,
                media: MediaToggles::default(),
            }],
            ..Config::default()
        };
        let grid = gtk4::FlowBox::new();
        let stack = gtk4::Stack::new();
        stack.add_named(
            &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
            Some("grid"),
        );
        stack.add_named(&gtk4::Label::new(Some("empty")), Some("empty"));
        let games = vec![sample_game("Chrono Trigger", false)];
        App::update_game_grid(&grid, &stack, &games, &config, false);

        let menu = gtk4::Popover::new();
        menu.set_child(Some(&gtk4::Label::new(Some("Scrape…"))));
        let tile = grid.child_at_index(0).unwrap();
        menu.set_parent(&tile);
        assert!(menu.parent().is_some());

        App::update_game_grid(&grid, &stack, &games, &config, false);
        assert!(menu.parent().is_none());
        assert!(menu.child().is_some());
        menu.set_parent(&grid.child_at_index(0).unwrap());
        assert!(menu.parent().is_some());
        menu.unparent();
    }

    fn flow_columns_follows_the_allocated_line() {
        let grid = gtk4::FlowBox::new();
        grid.set_max_children_per_line(6);
        grid.set_homogeneous(true);
        grid.set_column_spacing(12);
        grid.set_row_spacing(12);
        for _ in 0..12 {
            let tile = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            tile.set_size_request(100, 80);
            grid.insert(&tile, -1);
        }
        // Same max of 6 as the games grid. A narrower allocation wraps at 4.
        grid.allocate(480, 400, -1, None);
        assert_eq!(App::flow_columns(&grid), 4);
        grid.allocate(720, 400, -1, None);
        assert_eq!(App::flow_columns(&grid), 6);
    }

    fn favorite_badge_toggles_on_the_card_overlay() {
        use crate::config::Config;
        use crate::types::{Console, GridArt, MediaToggles};

        let config = Config {
            consoles: vec![Console {
                id: "snes".into(),
                name: "Super Nintendo".into(),
                rom_dirs: Vec::new(),
                extensions: Vec::new(),
                profile: None,
                grid_art: GridArt::BoxArt,
                media: MediaToggles::default(),
            }],
            ..Config::default()
        };
        let games = vec![
            sample_game("Chrono Trigger", false),
            sample_game("Super Metroid", true),
        ];
        let grid = gtk4::FlowBox::new();
        let stack = gtk4::Stack::new();
        stack.add_named(
            &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
            Some("grid"),
        );
        stack.add_named(
            &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
            Some("empty"),
        );
        App::update_game_grid(&grid, &stack, &games, &config, false);

        assert!(!favorite_badge(&grid, 0).is_visible());
        assert!(favorite_badge(&grid, 1).is_visible());
        assert_eq!(
            favorite_badge(&grid, 1)
                .downcast::<gtk4::Label>()
                .unwrap()
                .text()
                .as_str(),
            "♥"
        );

        let flow_child = grid.child_at_index(1).unwrap();
        let tile = App::tile_box(&flow_child).unwrap();
        let frame = App::tile_art_frame(&tile).unwrap();
        let badge = favorite_badge(&grid, 1);
        let overlay = badge.parent().unwrap().downcast::<gtk4::Overlay>().unwrap();
        assert!(overlay.child().unwrap().downcast::<gtk4::Box>().is_ok());
        assert!(frame.parent().unwrap().downcast::<gtk4::Box>().is_ok());

        App::set_tile_favorite(&grid, 1, false);
        assert!(!favorite_badge(&grid, 1).is_visible());
        App::set_tile_favorite(&grid, 1, true);
        assert!(favorite_badge(&grid, 1).is_visible());

        App::refresh_grid_tile(&grid, 1, &games[1], &config);
        let frame = App::tile_art_frame(&App::tile_box(&flow_child).unwrap()).unwrap();
        assert_eq!(
            frame.child().and_downcast::<gtk4::Label>().unwrap().text(),
            "No Art"
        );

        let window = gtk4::Window::new();
        window.set_default_size(480, 400);
        window.set_child(Some(&grid));
        window.present();
        for _ in 0..8 {
            while glib::MainContext::default().iteration(false) {}
        }
        let frame_bounds = frame.compute_bounds(&overlay).expect("art bounds");
        let badge_bounds = favorite_badge(&grid, 1)
            .compute_bounds(&overlay)
            .expect("heart bounds");
        let art_bottom = frame_bounds.y() + frame_bounds.height();
        assert!(
            badge_bounds.y() >= art_bottom - 1.0,
            "heart top {} is on the art, which ends at {}",
            badge_bounds.y(),
            art_bottom
        );
        assert!(badge_bounds.x() > frame_bounds.x() + frame_bounds.width() / 2.0);
        window.close();
    }

    fn sample_game(title: &str, favorite: bool) -> Game {
        Game {
            id: title.to_string(),
            console: "snes".into(),
            rom: std::path::PathBuf::from(title),
            title: title.to_string(),
            crc32: None,
            profile: None,
            media: Vec::new(),
            last_played: None,
            play_count: 0,
            play_time: 0,
            favorite,
        }
    }

    fn favorite_badge(grid: &gtk4::FlowBox, index: i32) -> gtk4::Widget {
        let overlay = grid
            .child_at_index(index)
            .expect("tile")
            .child()
            .and_downcast::<gtk4::Overlay>()
            .expect("card overlay");
        let mut widget = overlay.first_child();
        while let Some(current) = widget {
            if current.widget_name() == "favorite-badge" {
                return current;
            }
            widget = current.next_sibling();
        }
        panic!("favorite-badge missing");
    }

    fn detail_pane_puts_play_above_stats_and_rom_at_the_bottom() {
        use crate::config::Config;
        use crate::types::{Console, GridArt, MediaToggles};
        use chrono::TimeZone;

        let config = Config {
            consoles: vec![Console {
                id: "atari2600".into(),
                name: "Atari 2600".into(),
                rom_dirs: Vec::new(),
                extensions: Vec::new(),
                profile: None,
                grid_art: GridArt::BoxArt,
                media: MediaToggles::default(),
            }],
            ..Config::default()
        };
        let conn = Rc::new(RefCell::new(
            rusqlite::Connection::open_in_memory().unwrap(),
        ));
        let mut game = sample_game("Airlock", false);
        game.console = "atari2600".into();
        game.rom = std::path::PathBuf::from("/tmp/Airlock.zip");
        game.crc32 = Some(0x8678_f408);
        game.play_count = 3;
        game.play_time = 26;
        game.last_played = Some(
            chrono::Utc
                .with_ymd_and_hms(2026, 9, 25, 17, 48, 0)
                .unwrap(),
        );

        let detail = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        App::update_game_details(&detail, &game, &config, &conn);

        let row = detail.first_child().unwrap();
        let play = row.first_child().and_downcast::<gtk4::Button>().unwrap();
        assert_eq!(play.label().as_deref(), Some("Play"));
        let heart = play.next_sibling().and_downcast::<gtk4::Button>().unwrap();
        assert_eq!(heart.widget_name().as_str(), "favorite-toggle");
        assert_eq!(heart.label().as_deref(), Some("♡"));
        assert!(!heart.has_css_class("is-favorite"));
        assert_eq!(heart.action_name().as_deref(), Some("win.toggle-favorite"));

        assert_eq!(
            detail_texts(&detail),
            vec![
                "play-row",
                "Airlock",
                "Console: Atari 2600",
                "Last played: 2026-09-25 17:48",
                "Play count: 3",
                "Play time: 26s",
                "separator",
                "ROM: /tmp/Airlock.zip",
                "CRC32: 8678f408",
            ]
        );
        let rom = detail
            .last_child()
            .unwrap()
            .prev_sibling()
            .and_downcast::<gtk4::Label>()
            .unwrap();
        assert!(rom.wraps());
        assert_eq!(rom.wrap_mode(), gtk4::pango::WrapMode::WordChar);

        App::set_detail_favorite(&detail, true);
        assert_eq!(heart.label().as_deref(), Some("♥"));
        assert!(heart.has_css_class("is-favorite"));

        game.favorite = true;
        App::update_game_details(&detail, &game, &config, &conn);
        let heart = App::find_widget_name(detail.upcast_ref(), "favorite-toggle")
            .and_downcast::<gtk4::Button>()
            .unwrap();
        assert_eq!(heart.label().as_deref(), Some("♥"));
        assert!(heart.has_css_class("is-favorite"));

        App::update_console_details(&detail, "atari2600", &conn.borrow(), &config);
        assert!(App::find_widget_name(detail.upcast_ref(), "favorite-toggle").is_none());
        assert!(detail_texts(&detail).iter().all(|text| text != "play-row"));
    }

    fn detail_column_stays_fixed_when_play_expands() {
        let pane = App::detail_column();
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let play = gtk4::Button::with_label("Play");
        play.set_hexpand(true);
        row.append(&play);
        row.append(&gtk4::Button::with_label("♡"));
        content.append(&row);
        let rom = gtk4::Label::new(Some(
            "ROM: /home/dwelch/Games/roms/Emulation/roms/atari2600/Airlock (USA).zip",
        ));
        rom.set_wrap(true);
        rom.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        content.append(&rom);
        pane.set_child(Some(&content));

        assert!(!pane.compute_expand(gtk4::Orientation::Horizontal));
        let (min, nat, _, _) = pane.measure(gtk4::Orientation::Horizontal, -1);
        assert_eq!(min, DETAIL_PANE_WIDTH);
        assert_eq!(nat, DETAIL_PANE_WIDTH);
    }

    fn favorite_toggle_updates_badge_and_detail_heart() {
        use crate::config::Config;
        use crate::types::{Console, GridArt, GridFilter, MediaToggles};

        let config = Config {
            consoles: vec![Console {
                id: "snes".into(),
                name: "Super Nintendo".into(),
                rom_dirs: Vec::new(),
                extensions: Vec::new(),
                profile: None,
                grid_art: GridArt::BoxArt,
                media: MediaToggles::default(),
            }],
            ..Config::default()
        };
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = Rc::new(RefCell::new(database::open_db(file.path()).unwrap()));
        let game = sample_game("Chrono Trigger", false);
        database::upsert_game(&conn.borrow(), &game).unwrap();

        let games = Rc::new(RefCell::new(vec![game.clone()]));
        let all_games = Rc::new(RefCell::new(vec![game.clone()]));
        let selected = Rc::new(RefCell::new(Some(0usize)));
        let filter = Rc::new(RefCell::new(GridFilter::All));
        let grid = gtk4::FlowBox::new();
        let stack = gtk4::Stack::new();
        stack.add_named(
            &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
            Some("grid"),
        );
        stack.add_named(
            &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
            Some("empty"),
        );
        App::update_game_grid(&grid, &stack, &games.borrow(), &config, false);
        let detail = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        let config_rc = Rc::new(RefCell::new(config));
        App::update_game_details(&detail, &game, &config_rc.borrow(), &conn);
        let current = Rc::new(RefCell::new(Some("snes".into())));
        let search = gtk4::SearchEntry::new();

        let toggle = || {
            App::toggle_favorite(
                &conn, &games, &all_games, &selected, &filter, &grid, &stack, &config_rc, &detail,
                &current, &search,
            );
        };

        toggle();
        assert!(games.borrow()[0].favorite);
        assert!(database::load_games(&conn.borrow(), None).unwrap()[0].favorite);
        assert!(favorite_badge(&grid, 0).is_visible());
        let heart = App::find_widget_name(detail.upcast_ref(), "favorite-toggle")
            .and_downcast::<gtk4::Button>()
            .unwrap();
        assert_eq!(heart.label().as_deref(), Some("♥"));
        assert!(heart.has_css_class("is-favorite"));

        toggle();
        assert!(!games.borrow()[0].favorite);
        assert!(!favorite_badge(&grid, 0).is_visible());
        assert_eq!(heart.label().as_deref(), Some("♡"));
        assert!(!heart.has_css_class("is-favorite"));

        toggle();
        *filter.borrow_mut() = GridFilter::Favorites;
        toggle();
        assert!(App::find_widget_name(detail.upcast_ref(), "favorite-toggle").is_none());
        assert!(selected.borrow().is_none());
        assert!(!database::load_games(&conn.borrow(), None).unwrap()[0].favorite);
    }

    fn detail_texts(detail: &gtk4::Box) -> Vec<String> {
        let mut out = Vec::new();
        let mut child = detail.first_child();
        while let Some(current) = child {
            if current.downcast_ref::<gtk4::Separator>().is_some() {
                out.push("separator".into());
            } else if let Some(label) = current.downcast_ref::<gtk4::Label>() {
                out.push(label.text().to_string());
            } else if current
                .first_child()
                .and_downcast::<gtk4::Button>()
                .is_some()
            {
                out.push("play-row".into());
            }
            child = current.next_sibling();
        }
        out
    }

    fn pump_until(mut ready: impl FnMut() -> bool) {
        let start = std::time::Instant::now();
        let ctx = glib::MainContext::default();
        while start.elapsed() < std::time::Duration::from_secs(2) {
            while ctx.pending() {
                ctx.iteration(false);
            }
            if ready() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        panic!("timed out waiting for a dialog");
    }

    fn toplevel_titled(title: &str) -> gtk4::Window {
        gtk4::Window::list_toplevels()
            .into_iter()
            .find_map(|widget| {
                let window = widget.downcast::<gtk4::Window>().ok()?;
                (window.title().as_deref() == Some(title)).then_some(window)
            })
            .unwrap_or_else(|| panic!("missing dialog {title}"))
    }

    fn focused_check(dialog: &gtk4::Window) -> gtk4::CheckButton {
        gtk4::prelude::RootExt::focus(dialog)
            .and_then(|widget| widget.downcast::<gtk4::CheckButton>().ok())
            .expect("focus is not a check button")
    }

    fn controller_reaches_context_dialog_controls() {
        let app = adw::Application::builder()
            .application_id("org.omarchy.Retromarchy.DialogFocusTest")
            .build();
        let failure: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let failure_in = failure.clone();
        app.connect_activate(move |app| {
            let app = app.clone();
            let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let parent = adw::ApplicationWindow::new(&app);
                parent.set_default_size(640, 480);
                parent.present();
                let menu = gtk4::Popover::new();
                pump_until(|| parent.is_mapped());
                assert!(
                    matches!(App::pad_target(&parent, &menu), PadTarget::Grid),
                    "controller left the grid with no dialog open"
                );

                crate::dialogs::open_delete(&parent, "Stay Frosty 2", |_| {});
                pump_until(|| {
                    let dialog = toplevel_titled("Delete");
                    dialog.is_mapped() && dialog.allocation().width() > 50
                });
                let PadTarget::Dialog(dialog) = App::pad_target(&parent, &menu) else {
                    panic!("controller stayed on the grid while Delete was open");
                };
                assert!(dialog.child_focus(gtk4::DirectionType::Up));
                let assets = focused_check(&dialog);
                assert_eq!(assets.label().as_deref(), Some("Delete scraped assets"));
                assert!(!assets.is_active());
                assert!(assets.activate());
                assert!(assets.is_active());
                assert!(dialog.child_focus(gtk4::DirectionType::Up));
                let rom = focused_check(&dialog);
                assert_eq!(rom.label().as_deref(), Some("Delete ROM file from disk"));
                assert!(!rom.is_active());
                assert!(rom.activate());
                assert!(rom.is_active());
                assert!(dialog.child_focus(gtk4::DirectionType::Down));
                assert!(dialog.child_focus(gtk4::DirectionType::Down));
                let buttons =
                    gtk4::prelude::RootExt::focus(&dialog).expect("focus left the dialog");
                assert!(
                    buttons.downcast_ref::<gtk4::Button>().is_some(),
                    "down from the checkboxes did not reach Cancel or Delete"
                );
                dialog.close();

                crate::dialogs::open_rename(&parent, "Stay Frosty 2", |_| {});
                pump_until(|| toplevel_titled("Rename").is_mapped());
                let PadTarget::Dialog(rename) = App::pad_target(&parent, &menu) else {
                    panic!("controller stayed on the grid while Rename was open");
                };
                assert!(rename.child_focus(gtk4::DirectionType::Down));
                let cancel = gtk4::prelude::RootExt::focus(&rename).expect("rename lost focus");
                let cancel = cancel
                    .downcast::<gtk4::Button>()
                    .expect("Down did not reach a button");
                assert_eq!(cancel.label().as_deref(), Some("Cancel"));
                assert!(rename.child_focus(gtk4::DirectionType::Right));
                let save = gtk4::prelude::RootExt::focus(&rename)
                    .and_then(|widget| widget.downcast::<gtk4::Button>().ok())
                    .expect("Right did not reach Save");
                assert_eq!(save.label().as_deref(), Some("Save"));
                assert!(rename.child_focus(gtk4::DirectionType::Up));
                let entry = gtk4::prelude::RootExt::focus(&rename).expect("Up left Rename");
                assert!(
                    entry.ancestor(gtk4::Entry::static_type()).is_some()
                        || entry.downcast_ref::<gtk4::Entry>().is_some(),
                    "Up from Save did not return to the title entry"
                );
                rename.close();

                crate::dialogs::open_game_scrape(
                    &parent,
                    "Stay Frosty 2",
                    "nes",
                    Rc::new(RefCell::new(Config::default())),
                    Rc::new(|_| {}),
                );
                pump_until(|| toplevel_titled("Scrape").is_mapped());
                let PadTarget::Dialog(scrape) = App::pad_target(&parent, &menu) else {
                    panic!("controller stayed on the grid while Scrape was open");
                };
                assert!(scrape.child_focus(gtk4::DirectionType::Right));
                let search = gtk4::prelude::RootExt::focus(&scrape)
                    .and_then(|widget| widget.downcast::<gtk4::Button>().ok())
                    .expect("Right did not reach Search");
                assert_eq!(search.label().as_deref(), Some("Search"));
                scrape.close();
                app.quit();
            }));
            if let Err(payload) = ran {
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| {
                        payload
                            .downcast_ref::<&str>()
                            .map(|text| (*text).to_string())
                    })
                    .unwrap_or_else(|| "dialog focus test panicked".to_string());
                *failure_in.borrow_mut() = Some(message);
                app.quit();
            }
        });
        app.run_with_args::<&str>(&[]);
        let message = failure.borrow().clone();
        if let Some(message) = message {
            panic!("{message}");
        }
    }
}
