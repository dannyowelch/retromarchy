use crate::browse::{
    columns_for, cover_path, file_for, format_play_time, key_from_name, resolve_profile, row_of,
    Browse, Key, LibraryKind, Pane, TileFrame, DETAILS_WIDTH, GRID_PAD, SIDEBAR_WIDTH, TILE_GAP,
};
use crate::launcher;
use crate::types::{Game, GridArt, MediaKind};
use gpui_kit::{
    div, img, point, px, App, ClickEvent, Context, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, ObjectFit, ParentElement, Render, ScrollHandle, StatefulInteractiveElement,
    Styled, StyledImage, Window, WindowBounds, WindowDecorations, WindowOptions,
};
use gpui_omarchy::{
    badge, button, empty_state, focus_scope, keycap, separator, ActiveTheme, ButtonVariant, Status,
};

pub struct Shell {
    browse: Browse,
    focus_handle: FocusHandle,
    armed: bool,
    grid_scroll: ScrollHandle,
    sidebar_scroll: ScrollHandle,
    revealed_console: Option<usize>,
    revealed_game: Option<usize>,
    revealed_columns: usize,
}

impl Shell {
    pub fn new(browse: Browse, cx: &mut Context<Self>) -> Self {
        Self {
            browse,
            focus_handle: cx.focus_handle(),
            armed: false,
            grid_scroll: ScrollHandle::new(),
            sidebar_scroll: ScrollHandle::new(),
            revealed_console: None,
            revealed_game: None,
            revealed_columns: 0,
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

    /// Scroll the selected row into view after the scrollports have a real size.
    /// Remembering the last reveal keeps a wheel gesture from snapping back.
    fn reveal_selection(&mut self) {
        if self.grid_scroll.bounds().size.height <= px(0.) {
            return;
        }
        let columns = self.browse.columns.max(1);
        let console = self.browse.console;
        let game = self.browse.game;
        if self.revealed_console == Some(console)
            && self.revealed_game == game
            && self.revealed_columns == columns
        {
            return;
        }
        if self.revealed_console != Some(console) {
            self.grid_scroll.set_offset(point(px(0.), px(0.)));
            if self.sidebar_scroll.bounds().size.height > px(0.) {
                self.sidebar_scroll.scroll_to_item(console + 1);
            }
        }
        if let Some(index) = game {
            self.grid_scroll.scroll_to_item(row_of(index, columns));
        }
        self.revealed_console = Some(console);
        self.revealed_game = game;
        self.revealed_columns = columns;
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width.as_f32();
        let frame = self.browse.tile_frame();
        self.browse
            .set_columns(columns_for(width, self.browse.details_open, frame.width));
        self.reveal_selection();
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
            .child(body(
                &self.browse,
                &self.grid_scroll,
                &self.sidebar_scroll,
                cx,
            ))
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

fn body(
    browse: &Browse,
    grid_scroll: &ScrollHandle,
    sidebar_scroll: &ScrollHandle,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let mut row = div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_row()
        .child(sidebar(browse, sidebar_scroll, cx))
        .child(grid(browse, grid_scroll, cx));
    if browse.details_open {
        row = row.child(details(browse, cx));
    }
    row
}

fn sidebar(browse: &Browse, scroll: &ScrollHandle, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let focused = browse.pane == Pane::Sidebar;
    div()
        .id("sidebar")
        .w(px(SIDEBAR_WIDTH))
        .h_full()
        .min_h_0()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .bg(theme.inset)
        .border_r_1()
        .border_color(if focused { theme.accent } else { theme.border })
        .p(px(8.))
        .gap(px(4.))
        .child(
            div()
                .flex_shrink_0()
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
                        .id(("console", index))
                        .flex_shrink_0()
                        .px(px(8.))
                        .py(px(8.))
                        .bg(if selected {
                            theme.selected_fill()
                        } else {
                            theme.background
                        })
                        .border_1()
                        .border_color(if selected { theme.accent } else { theme.border })
                        .hover(|style| style.bg(theme.hover_fill()))
                        .on_click(cx.listener(
                            move |this: &mut Shell, _: &ClickEvent, window, cx| {
                                this.revealed_console = None;
                                this.browse.select_console(index);
                                this.focus_handle.focus(window, cx);
                                cx.notify();
                            },
                        ))
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

fn grid(browse: &Browse, scroll: &ScrollHandle, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let focused = browse.pane == Pane::Grid;
    let shelf = browse.shelf();
    let mut pane = div()
        .id("grid")
        .flex_1()
        .h_full()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .gap(px(TILE_GAP))
        .overflow_hidden()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .p(px(GRID_PAD))
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
    let columns = browse.columns.max(1);
    let art = shelf.console.grid_art;
    let demo = browse.library.kind == LibraryKind::Demo;
    let count = shelf.games.len();
    for start in (0..count).step_by(columns) {
        let end = (start + columns).min(count);
        let mut row = div().flex().flex_row().flex_shrink_0().gap(px(TILE_GAP));
        for index in start..end {
            let game = &shelf.games[index];
            row = row.child(tile(index, game, art, browse.game == Some(index), demo, cx));
        }
        pane = pane.child(row);
    }
    pane
}

fn tile(
    index: usize,
    game: &Game,
    art: GridArt,
    selected: bool,
    demo: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let frame = TileFrame::for_art(art);
    let title = game.title.clone();
    let cover = cover_path(game, art);
    // An explicit ratio stops GPUI from resizing the tile to the file's own ratio.
    let image = div()
        .w(px(frame.width))
        .h(px(frame.height))
        .flex_shrink_0()
        .overflow_hidden()
        .bg(theme.surface);
    let image = if let Some(path) = cover {
        image.child(
            img(path)
                .w(px(frame.width))
                .h(px(frame.height))
                .aspect_ratio(frame.ratio())
                .object_fit(ObjectFit::Contain),
        )
    } else {
        image
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(28.))
            .font_weight(gpui_kit::FontWeight::BOLD)
            .text_color(theme.accent)
            .child(initials(&title))
    };
    let mut caption = div()
        .p(px(8.))
        .w(px(frame.width))
        .text_size(px(12.))
        .line_clamp(2)
        .child(title);
    if demo {
        caption = caption.child(div().text_color(theme.secondary).child("Placeholder"));
    }
    div()
        .id(("game", index))
        .w(px(frame.width))
        .flex_shrink_0()
        .bg(theme.inset)
        .border_1()
        .border_color(if selected { theme.accent } else { theme.border })
        .hover(|style| style.border_color(theme.accent))
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                this.revealed_game = None;
                this.browse.select_game(index);
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(image)
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

const DETAILS_PAD: f32 = 8.0;
const DETAILS_GAP: f32 = 6.0;

fn details(browse: &Browse, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut pane = div()
        .id("details")
        .w(px(DETAILS_WIDTH))
        .h_full()
        .flex_shrink_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .justify_start()
        .gap(px(DETAILS_GAP))
        .p(px(DETAILS_PAD))
        .bg(theme.surface)
        .border_l_1()
        .border_color(theme.border);
    let Some(shelf) = browse.shelf() else {
        return pane;
    };
    if let Some(game) = browse.selected_game() {
        pane = pane.child(
            button("play", "Play", ButtonVariant::Primary, cx)
                .flex_shrink_0()
                .on_click(cx.listener(|this: &mut Shell, _: &ClickEvent, window, cx| {
                    this.launch_selected();
                    this.focus_handle.focus(window, cx);
                    cx.notify();
                })),
        );
        if let Some(path) = file_for(game, MediaKind::BoxArt) {
            pane = pane.child(art_block(path));
        }
        if let Some(path) = file_for(game, MediaKind::Screenshot) {
            pane = pane.child(art_block(path));
        }
        pane = pane
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

/// Details art at the pane's content width and the file's aspect.
/// A percent width makes GPUI lock the height to the file's pixel height,
/// so Contain letterboxes the picture inside a tall empty slot.
fn art_block(path: std::path::PathBuf) -> impl IntoElement {
    let width = DETAILS_WIDTH - DETAILS_PAD * 2.0 - 1.0;
    div().w_full().flex_shrink_0().child(
        img(path)
            .w(px(width))
            .flex_shrink_0()
            .object_fit(ObjectFit::Contain),
    )
}

fn heading(text: &str) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .font_weight(gpui_kit::FontWeight::BOLD)
        .text_size(px(16.))
        .child(text.to_string())
}

fn meta(text: String, cx: &App) -> impl IntoElement {
    div()
        .flex_shrink_0()
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
