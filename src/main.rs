use gpui_kit::AppContext;
use retromarchy::browse::{self, Browse};

fn main() {
    let library = browse::open_library();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            let browse = Browse::new(library);
            cx.open_window(retromarchy::shell::window_options(), move |_window, cx| {
                cx.new(|cx| retromarchy::shell::Shell::new(browse, cx))
            })
            .expect("open window");
            cx.activate(true);
        });
}
