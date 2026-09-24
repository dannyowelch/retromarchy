mod config;
mod database;
mod launcher;
mod scanner;
mod types;
mod ui;

use anyhow::Result;
use gtk4::prelude::*;
use libadwaita as adw;

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
    let conn = database::init_db()?;

    let ui = ui::App::new(app, config, conn)?;
    ui.show();

    Ok(())
}
