// Minimal GTK4 keyboard event test
use gtk4::prelude::*;

fn main() {
    let app = gtk4::Application::builder()
        .application_id("test.keyboard")
        .build();

    app.connect_activate(|app| {
        let window = gtk4::ApplicationWindow::new(app);
        window.set_title(Some("KeyboardTest"));
        window.set_default_size(400, 300);

        let label = gtk4::Label::new(Some("Press any key..."));
        window.set_child(Some(&label));

        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, code, _| {
            eprintln!("KEY PRESSED: {:?} (code: {})", key, code);
            gtk4::glib::Propagation::Stop
        });
        window.add_controller(key_controller);

        window.present();
    });

    app.run();
}
