use crate::browse::{
    columns_for, cover_path, file_for, format_play_time, key_from_name, resolve_profile, Browse,
    Key, LibraryKind, Pane,
};
use crate::launcher;
use crate::types::MediaKind;
use gpui_kit::{
    div, img, px, App, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, StyledImage, Window, WindowBounds,
    WindowDecorations, WindowOptions,
};
use gpui_omarchy::{
    badge, button, empty_state, focus_scope, keycap, separator, ActiveTheme, ButtonVariant, Status,
};

pub struct Shell {
    browse: Browse,
    focus_handle: FocusHandle,
    armed: bool,
}

impl Shell {
    pub fn new(browse: Browse, cx: &mut Context<Self>) -> Self {
        Self {
            browse,
            focus_handle: cx.focus_handle(),
            armed: false,
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modified =
            keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform;
        let Some(key) = key_from_name(keystroke.key.as_ref(), modified) else {
            return;
        };
        if key == Key::Launch {
            self.launch_selected();
        } else {
            self.browse.apply(key);
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn launch_selected(&mut self) {
        let Some(game) = self.browse.selected_game().cloned() else {
            self.browse.status = "Select a game, then press Enter.".into();
            return;
        };
        let Some(profile) = resolve_profile(&self.browse.library, &game) else {
            self.browse.status = "This system has no emulator profile.".into();
            return;
        };
        match launcher::launch_game_tracked(&profile, &game.rom) {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                self.browse.status = format!("Launched {}.", game.title);
            }
            Err(err) => self.browse.status = err.to_string(),
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width.as_f32();
        self.browse
            .set_columns(columns_for(width, self.browse.details_open));
        if !self.armed {
            self.armed = true;
            self.focus_handle.focus(window, cx);
        }

        let theme_name = cx.omarchy().name.to_string();
        let background = cx.omarchy().background;
        let foreground = cx.omarchy().foreground;
        let font = cx.omarchy().font.clone();

        focus_scope("retromarchy")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .font_family(font)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.on_key(event, cx);
            }))
            .child(header(&self.browse, &theme_name, cx))
            .children(note_bar(&self.browse.library.note, cx))
            .child(body(&self.browse, cx))
            .child(status_line(&self.browse.status, cx))
    }
}

fn header(browse: &Browse, theme_name: &str, cx: &App) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut row = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .h(px(48.))
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .font_weight(gpui_kit::FontWeight::BOLD)
                .child("Retromarchy"),
        )
        .child(keycap(theme_name, cx));
    if browse.library.kind == LibraryKind::Demo {
        row = row.child(badge("Demo library", Status::Warning, cx));
    }
    row.child(div().flex_1()).child(
        div()
            .flex()
            .gap(px(6.))
            .items_center()
            .text_color(theme.secondary)
            .text_size(px(12.))
            .child(keycap("arrows", cx))
            .child("move")
            .child(keycap("d", cx))
            .child("details")
            .child(keycap("esc", cx))
            .child("clear"),
    )
}

fn note_bar(note: &str, cx: &App) -> Option<impl IntoElement> {
    if note.is_empty() {
        return None;
    }
    let theme = cx.omarchy();
    Some(
        div()
            .px(px(16.))
            .py(px(6.))
            .bg(theme.surface)
            .text_size(px(12.))
            .text_color(theme.secondary)
            .border_b_1()
            .border_color(theme.border)
            .child(note.to_string()),
    )
}

fn body(browse: &Browse, cx: &Context<Shell>) -> impl IntoElement {
    let mut row = div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_row()
        .child(sidebar(browse, cx))
        .child(grid(browse, cx));
    if browse.details_open {
        row = row.child(details(browse, cx));
    }
    row
}

fn sidebar(browse: &Browse, cx: &App) -> impl IntoElement {
    let theme = cx.omarchy();
    let focused = browse.pane == Pane::Sidebar;
    div()
        .id("sidebar")
        .w(px(220.))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .bg(theme.inset)
        .border_r_1()
        .border_color(if focused { theme.accent } else { theme.border })
        .p(px(8.))
        .gap(px(4.))
        .child(
            div()
                .px(px(8.))
                .py(px(6.))
                .text_size(px(12.))
                .text_color(theme.secondary)
                .child("Systems"),
        )
        .children(
            browse
                .library
                .shelves
                .iter()
                .enumerate()
                .map(|(index, shelf)| {
                    let selected = index == browse.console;
                    div()
                        .px(px(8.))
                        .py(px(8.))
                        .bg(if selected {
                            theme.selected_fill()
                        } else {
                            theme.background
                        })
                        .border_1()
                        .border_color(if selected { theme.accent } else { theme.border })
                        .child(shelf.console.name.clone())
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.secondary)
                                .child(format!("{} games", shelf.games.len())),
                        )
                }),
        )
}

fn grid(browse: &Browse, cx: &App) -> impl IntoElement {
    let theme = cx.omarchy();
    let focused = browse.pane == Pane::Grid;
    let shelf = browse.shelf();
    let mut pane = div()
        .id("grid")
        .flex_1()
        .h_full()
        .min_w_0()
        .overflow_y_scroll()
        .p(px(16.))
        .border_1()
        .border_color(if focused {
            theme.accent
        } else {
            theme.background
        });
    let Some(shelf) = shelf else {
        return pane.child(empty_state(
            "No systems",
            "The library has no consoles.",
            cx,
        ));
    };
    if shelf.games.is_empty() {
        return pane.child(empty_state(
            "No games found",
            "This console has no scanned games.",
            cx,
        ));
    }
    pane = pane.child(div().flex().flex_wrap().gap(px(12.)).children(
        shelf.games.iter().enumerate().map(|(index, game)| {
            tile(
                &game.title,
                cover_path(game, shelf.console.grid_art),
                browse.game == Some(index),
                browse.library.kind == LibraryKind::Demo,
                cx,
            )
        }),
    ));
    pane
}

fn tile(
    title: &str,
    cover: Option<std::path::PathBuf>,
    selected: bool,
    demo: bool,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let art = div().w(px(148.)).h(px(148.)).overflow_hidden();
    let art = if let Some(path) = cover {
        art.child(img(path).size_full().object_fit(gpui_kit::ObjectFit::Cover))
    } else {
        art.bg(theme.surface)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(28.))
            .font_weight(gpui_kit::FontWeight::BOLD)
            .text_color(theme.accent)
            .child(initials(title))
    };
    let mut caption = div()
        .p(px(8.))
        .w(px(148.))
        .text_size(px(12.))
        .child(title.to_string());
    if demo {
        caption = caption.child(div().text_color(theme.secondary).child("Placeholder"));
    }
    div()
        .w(px(148.))
        .bg(theme.inset)
        .border_1()
        .border_color(if selected { theme.accent } else { theme.border })
        .child(art)
        .child(caption)
}

fn initials(title: &str) -> String {
    let mut letters = String::new();
    for word in title.split_whitespace() {
        if let Some(ch) = word.chars().next() {
            letters.push(ch);
        }
        if letters.chars().count() == 3 {
            break;
        }
    }
    if letters.is_empty() {
        "?".into()
    } else {
        letters
    }
}

fn details(browse: &Browse, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut pane = div()
        .id("details")
        .w(px(280.))
        .h_full()
        .flex_shrink_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(16.))
        .bg(theme.surface)
        .border_l_1()
        .border_color(theme.border);
    let Some(shelf) = browse.shelf() else {
        return pane;
    };
    if let Some(game) = browse.selected_game() {
        pane = pane
            .child(
                button("play", "Play", ButtonVariant::Primary, cx).on_click(cx.listener(
                    |this: &mut Shell, _, _, cx| {
                        this.launch_selected();
                        cx.notify();
                    },
                )),
            )
            .child(art_block(file_for(game, MediaKind::BoxArt), cx))
            .child(art_block(file_for(game, MediaKind::Screenshot), cx))
            .child(heading(&game.title))
            .child(meta(format!("Console: {}", shelf.console.name), cx));
        if let Some(played) = game.last_played {
            pane = pane.child(meta(
                format!("Last played: {}", played.format("%Y-%m-%d %H:%M")),
                cx,
            ));
        }
        if game.play_count > 0 {
            pane = pane.child(meta(format!("Play count: {}", game.play_count), cx));
        }
        if game.play_time > 0 {
            pane = pane.child(meta(
                format!("Play time: {}", format_play_time(game.play_time)),
                cx,
            ));
        }
        pane = pane
            .child(separator(cx))
            .child(meta(format!("ROM: {}", game.rom.display()), cx));
        if let Some(crc) = game.crc32 {
            pane = pane.child(meta(format!("CRC32: {crc:08x}"), cx));
        }
        if browse.library.kind == LibraryKind::Demo {
            pane = pane.child(meta("Placeholder title. This row is not a ROM.".into(), cx));
        }
        return pane;
    }

    pane = pane.child(heading(&shelf.console.name));
    if let Some(manufacturer) = &shelf.manufacturer {
        pane = pane.child(meta(format!("Manufacturer: {manufacturer}"), cx));
    }
    if let Some(year) = shelf.year {
        pane = pane.child(meta(format!("Year: {year}"), cx));
    }
    if let Some(description) = &shelf.description {
        pane = pane.child(meta(description.clone(), cx));
    }
    pane.child(separator(cx))
        .child(heading("Library statistics"))
        .child(meta(
            format!("Total games: {}", shelf.stats.total_games),
            cx,
        ))
        .children(shelf.stats.last_played_date.map(|date| {
            meta(
                format!("Last played: {}", date.format("%Y-%m-%d %H:%M")),
                cx,
            )
        }))
        .children(
            shelf
                .stats
                .last_played_game
                .as_ref()
                .map(|title| meta(format!("{title}"), cx)),
        )
        .children(
            (shelf.stats.total_play_count > 0)
                .then(|| meta(format!("Total plays: {}", shelf.stats.total_play_count), cx)),
        )
        .children((shelf.stats.total_play_time > 0).then(|| {
            meta(
                format!(
                    "Total play time: {}",
                    format_play_time(shelf.stats.total_play_time)
                ),
                cx,
            )
        }))
        .children(shelf.stats.most_played_game.as_ref().map(|title| {
            meta(
                format!(
                    "Most played: {title} ({} plays)",
                    shelf.stats.most_played_count
                ),
                cx,
            )
        }))
}

fn art_block(path: Option<std::path::PathBuf>, _cx: &App) -> impl IntoElement {
    let block = div().w_full().h(px(140.)).overflow_hidden();
    if let Some(path) = path {
        block.child(
            img(path)
                .w_full()
                .h(px(140.))
                .object_fit(gpui_kit::ObjectFit::Contain),
        )
    } else {
        block
    }
}

fn heading(text: &str) -> impl IntoElement {
    div()
        .font_weight(gpui_kit::FontWeight::BOLD)
        .text_size(px(16.))
        .child(text.to_string())
}

fn meta(text: String, cx: &App) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .text_color(cx.omarchy().secondary)
        .child(text)
}

fn status_line(status: &str, cx: &App) -> impl IntoElement {
    let theme = cx.omarchy();
    let text = if status.is_empty() {
        "Import, scrape, and emulator setup stay on the GTK app."
    } else {
        status
    };
    div()
        .h(px(32.))
        .flex()
        .items_center()
        .px(px(16.))
        .bg(theme.inset)
        .border_t_1()
        .border_color(theme.border)
        .text_size(px(12.))
        .text_color(theme.secondary)
        .child(text.to_string())
}

pub fn window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
            origin: gpui_kit::point(px(40.), px(40.)),
            size: gpui_kit::size(px(1280.), px(800.)),
        })),
        app_id: Some("org.omarchy.Retromarchy".into()),
        // Hyprland tiles server-decorated windows. Client frames fight the layout.
        window_decorations: Some(WindowDecorations::Server),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Retromarchy".into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}
