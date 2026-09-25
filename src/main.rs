mod catalog;
mod config;
mod cores;
mod database;
mod gamepad;
mod dialogs;
mod importer;
mod launcher;
mod scanner;
mod scraper;
mod softname;
mod types;
mod ui;

use anyhow::Result;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> Result<()> {
    let app = adw::Application::builder()
        .application_id("org.omarchy.Retromarchy")
        .build();

    app.connect_activate(|app| {
        if let Err(e) = run_app(app) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    });

    app.run();
    Ok(())
}

fn run_app(app: &adw::Application) -> Result<()> {
    let config = config::load_config()?;

    if std::env::var("RETROMARCHY_DEBUG").is_ok() {
        let config_path = config::config_path()?;
        eprintln!("Loaded config from: {}", config_path.display());
        eprintln!("  Consoles: {}", config.consoles.len());
        eprintln!("  Profiles: {}", config.profiles.len());
        eprintln!("  Theme: {}", config.theme);
        eprintln!("  Details visible: {}", config.details_visible);

        let db_path = database::db_path()?;
        eprintln!("Database: {}", db_path.display());
    }

    let conn = Rc::new(RefCell::new(database::init_db()?));

    let ui = ui::App::new(app, config, conn)?;
    ui.show();

    Ok(())
}
