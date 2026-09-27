use gpui_kit::AppContext;
use retromarchy::browse::{self, Browse};

fn main() {
    let library = browse::open_library();
    let cover_width = Browse::saved_cover_width();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            let browse = Browse::with_cover_width(library, cover_width);
            cx.open_window(retromarchy::shell::window_options(), move |_window, cx| {
                cx.new(|cx| retromarchy::shell::Shell::new(browse, cx))
            })
            .expect("open window");
            cx.activate(true);
        });
}
