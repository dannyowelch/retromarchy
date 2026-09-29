use crate::appearance::{self, launchbox_theme, theme_key};
use crate::browse::{
    clamp_cover_width, columns_for, file_for, format_play_time, game_count_label, grid_art_key,
    image_aspect, is_launch_key, key_from_parts, rescan_key, resolve_launch, reveal_row_scroll,
    row_of, row_reveal_insets, scrape_chord, system_sort_key, title_search_key, Browse, Confirm,
    Focus, Key, LibraryKind, MenuItem, ScrapeChord, TileFrame, COVER_WIDTH_MAX, COVER_WIDTH_MIN,
    COVER_WIDTH_STEP, DETAILS_PAD, DETAILS_WIDTH, GRID_PAD, SIDEBAR_WIDTH, TILE_GAP,
};
use crate::config::{self, InputSettings, SystemSort};
use crate::cores;
use crate::database;
use crate::emulators::{
    self, emulator_key, Emulators, Field as EmulatorField, Step as EmulatorStep,
};
use crate::game_menu::{
    game_menu_key, Aim, DeleteSlot, Overlay, OverlayCommand, RenameSlot, ScrapeSlot,
};
use crate::gamepad::{InputGate, NavDir, PadAction, PadHeld, PadHook};
use crate::input_repeat::HoldRepeat;
use crate::launcher;
use crate::options::{
    self, options_key, Command as OptionsCommand, Options, Section as OptionsSection,
};
use crate::picker::{self, Jump};
use crate::scanner;
use crate::scraper::{self, NameSearch, ScrapeBatch, ScrapeRuns, ScrapeUpdate};
use crate::scraper_settings::{
    scraper_key, Block as ScraperBlock, Part as ScraperPart, Slot as ScraperSlot,
};
use crate::split;
use crate::systems::{self, systems_key, Field as SystemField, Step as SystemStep, Systems};
use crate::types::{
    DeleteOptions, EmulatorKind, Game, GameMetadata, GridArt, GridFilter, Media, MediaKind,
};
use gpui_kit::base::slider::{SliderEvent, SliderState};
use gpui_kit::base::CheckboxState;
use gpui_kit::{
    anchored, deferred, div, img, point, px, rgb, uniform_list, AnyElement, App, AppContext,
    ClickEvent, Context, Edges, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    KeyUpEvent, Keystroke, MouseButton, MouseDownEvent, ObjectFit, ParentElement,
    PathPromptOptions, Render, ScrollHandle, StatefulInteractiveElement, Styled, StyledImage,
    Subscription, Task, UniformListScrollHandle, Window, WindowBounds, WindowDecorations,
    WindowOptions,
};
use gpui_omarchy::{
    badge, button, checkbox, empty_state, focus_scope, keycap, separator, slider,
    vertical_separator, with_tooltip, ActiveTheme, ButtonVariant, Status,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct Shell {
    browse: Browse,
    focus_handle: FocusHandle,
    armed: bool,
    grid_scroll: UniformListScrollHandle,
    sidebar_scroll: ScrollHandle,
    revealed_console: Option<usize>,
    revealed_game: Option<usize>,
    revealed_columns: usize,
    cover_slider: Entity<SliderState>,
    _cover_slider_sub: Subscription,
    gilrs: Option<gilrs::Gilrs>,
    pad: PadHeld,
    /// Pad and key actions stay off while a game is running, while this window
    /// is inactive, and until the pad is released after either of those ends.
    gate: InputGate,
    /// `RETROMARCHY_PAD_HOOK` lines, on machines with no gamepad device.
    pad_hook: Option<PadHook>,
    activation: Option<Subscription>,
    /// Arrow keys and the pad share this clock. Rates come from config `[input]`
    /// and update when Options saves.
    hold: HoldRepeat,
    input: InputSettings,
    /// Config `theme`: `system` follows Omarchy, `launchbox` is the built-in palette.
    appearance: String,
    nav_started: Instant,
    _nav_poll: Task<()>,
    search: Option<std::sync::mpsc::Receiver<NameSearch>>,
    apply: Option<std::sync::mpsc::Receiver<ScrapeUpdate>>,
    scrape_runs: ScrapeRuns,
    scrape_scroll: ScrollHandle,
    emulators: Option<Emulators>,
    systems: Option<Systems>,
    options: Option<Options>,
    emulator_scroll: ScrollHandle,
    systems_scroll: ScrollHandle,
    picker_scroll: ScrollHandle,
    picker_mark: Option<(SystemField, usize)>,
    options_scroll: ScrollHandle,
    /// Closing the dialog drops whatever control had focus. The next frame
    /// puts the keyboard back on the shell.
    refocus: bool,
    play_tx: std::sync::mpsc::Sender<PlayNote>,
    play_rx: std::sync::mpsc::Receiver<PlayNote>,
    rescan_rx: Option<std::sync::mpsc::Receiver<RescanNote>>,
}

enum PlayNote {
    Saved {
        game_id: String,
        play_count: u32,
        play_time: u32,
        last_played: chrono::DateTime<chrono::Utc>,
    },
    Failed(String),
}

enum PadIn {
    Device(gilrs::EventType),
    Hook(gilrs::Button, bool),
}

enum RescanNote {
    Done {
        console_id: String,
        name: String,
        games: Vec<Game>,
        stats: database::LibraryStats,
    },
    Failed(String),
}

const FAVORITE_RED: u32 = 0xe01b24;

impl Shell {
    pub fn new(browse: Browse, cx: &mut Context<Self>) -> Self {
        let cover_slider = cx.new(|_| {
            // max before min: the builder clamps against the previous max, which starts at 100.
            SliderState::new()
                .max(COVER_WIDTH_MAX)
                .min(COVER_WIDTH_MIN)
                .step(COVER_WIDTH_STEP)
                .default_value(browse.cover_width)
        });
        let cover_slider_sub =
            cx.subscribe(&cover_slider, |this, _slider, event: &SliderEvent, cx| {
                let value = match event {
                    SliderEvent::Change(value) | SliderEvent::Release(value) => value.end(),
                };
                this.apply_cover_width(value, cx);
            });
        let gilrs = match gilrs::Gilrs::new() {
            Ok(gilrs) => Some(gilrs),
            Err(err) => {
                eprintln!("Gamepad unavailable: {err}");
                None
            }
        };
        let appearance = load_appearance();
        if appearance::is_launchbox(&appearance) {
            launchbox_theme().apply(cx);
        }
        let (play_tx, play_rx) = std::sync::mpsc::channel();
        let nav_started = Instant::now();
        let nav_poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            if this
                .update(cx, |this, cx| {
                    this.poll_nav(cx);
                })
                .is_err()
            {
                break;
            }
        });
        Self {
            browse,
            focus_handle: cx.focus_handle(),
            armed: false,
            grid_scroll: UniformListScrollHandle::new(),
            sidebar_scroll: ScrollHandle::new(),
            revealed_console: None,
            revealed_game: None,
            revealed_columns: 0,
            cover_slider,
            _cover_slider_sub: cover_slider_sub,
            gilrs,
            pad: PadHeld::default(),
            gate: InputGate::default(),
            pad_hook: PadHook::open(),
            activation: None,
            hold: HoldRepeat::default(),
            input: load_input(),
            appearance,
            nav_started,
            _nav_poll: nav_poll,
            search: None,
            apply: None,
            scrape_runs: ScrapeRuns::default(),
            scrape_scroll: ScrollHandle::new(),
            emulators: None,
            systems: None,
            options: None,
            emulator_scroll: ScrollHandle::new(),
            systems_scroll: ScrollHandle::new(),
            picker_scroll: ScrollHandle::new(),
            picker_mark: None,
            options_scroll: ScrollHandle::new(),
            refocus: false,
            play_tx,
            play_rx,
            rescan_rx: None,
        }
    }

    /// Face buttons are edges. Held directions step through [`HoldRepeat`].
    /// South enters the grid or launches. East returns to the system list.
    /// North toggles a favorite. Select opens the game menu, and closes it.
    /// None of that is delivered while [`InputGate`] is closed.
    fn poll_nav(&mut self, cx: &mut Context<Self>) {
        self.pump_thumbs(cx);
        let events = self.drain_pad_events();
        for event in &events {
            self.observe_pad(event);
        }
        if !self.gate.live() {
            self.sink_pad(&events);
            let changed = self.poll_jobs();
            self.close_pad_poll();
            if changed {
                cx.notify();
            }
            return;
        }
        let mut changed = self.poll_jobs();
        if !self.gate.live() {
            self.sink_pad(&events);
            self.close_pad_poll();
            if changed {
                cx.notify();
            }
            return;
        }
        if self.options.is_some() {
            changed |= self.poll_options_pad(&events, cx);
            if changed {
                cx.notify();
            }
            return;
        }
        if self.systems.is_some() {
            changed |= self.poll_systems_pad(&events, cx);
            if changed {
                cx.notify();
            }
            return;
        }
        if self.emulators.is_some() {
            changed |= self.poll_emulator_pad(&events, cx);
            if changed {
                cx.notify();
            }
            return;
        }
        for event in &events {
            if !self.gate.live() {
                let _ = self.apply_pad(event);
                continue;
            }
            match self.apply_pad(event) {
                Some(PadAction::Favorite) if !self.browse.overlay_open() => {
                    changed |= self.toggle_favorite();
                }
                Some(PadAction::Confirm) if self.browse.overlay_open() => {
                    self.confirm_overlay();
                    changed = true;
                }
                Some(PadAction::Confirm) => changed |= self.confirm_pad(),
                Some(PadAction::Back) if self.browse.overlay_open() => {
                    self.dismiss_overlay();
                    changed = true;
                }
                Some(PadAction::Back) => changed |= self.browse.back(),
                Some(PadAction::Menu) if self.browse.menu_open() => {
                    self.dismiss_overlay();
                    changed = true;
                }
                Some(PadAction::Menu) if !self.browse.overlay_open() => {
                    self.browse.open_game_menu();
                    changed = true;
                }
                Some(PadAction::Favorite | PadAction::Menu) | None => {}
            }
        }
        if !self.gate.live() {
            self.close_pad_poll();
            if changed {
                cx.notify();
            }
            return;
        }
        let now = monotonic_ms(self.nav_started);
        let (step_x, step_y) =
            self.hold
                .poll(&self.input, self.pad.horizontal(), self.pad.vertical(), now);
        if self.browse.overlay_open() {
            if let Some(dir) = step_x {
                self.browse.move_overlay(dir);
                changed = true;
            }
            if let Some(dir) = step_y {
                self.browse.move_overlay(dir);
                changed = true;
            }
        } else {
            if let Some(dir) = step_x {
                self.browse.apply(Key::Arrow(dir));
                changed = true;
            }
            if let Some(dir) = step_y {
                self.browse.apply(Key::Arrow(dir));
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn confirm_pad(&mut self) -> bool {
        match self.browse.confirm() {
            Some(Confirm::Launch) => {
                self.launch_selected();
                true
            }
            Some(Confirm::Entered) => true,
            Some(Confirm::Menu(item)) => {
                self.open_menu_item(item);
                true
            }
            None => false,
        }
    }

    fn open_menu_item(&mut self, item: MenuItem) {
        self.activate_screen(screen_for_menu(item));
    }

    fn sink_pad(&mut self, events: &[PadIn]) {
        for event in events {
            let _ = self.apply_pad(event);
        }
    }

    fn observe_pad(&mut self, event: &PadIn) {
        match event {
            PadIn::Device(event) => self.gate.observe_gilrs(event),
            PadIn::Hook(button, down) => self.gate.observe_button(*button as u16, *down),
        }
    }

    fn apply_pad(&mut self, event: &PadIn) -> Option<PadAction> {
        match event {
            PadIn::Device(event) => self.pad.apply(event),
            PadIn::Hook(button, down) => self.pad.apply_button(*button, *down),
        }
    }

    fn axis_held(&self) -> bool {
        self.pad.horizontal().is_some() || self.pad.vertical().is_some()
    }

    fn close_pad_poll(&mut self) {
        self.gate.end_poll(self.axis_held());
        if !self.gate.live() {
            self.hold = HoldRepeat::default();
        }
    }

    fn drain_pad_events(&mut self) -> Vec<PadIn> {
        let mut events = Vec::new();
        if let Some(gilrs) = self.gilrs.as_mut() {
            while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
                events.push(PadIn::Device(event));
            }
        }
        if let Some(hook) = self.pad_hook.as_mut() {
            for (button, down) in hook.poll() {
                events.push(PadIn::Hook(button, down));
            }
        }
        events
    }

    fn toggle_favorite(&mut self) -> bool {
        let conn = if self.browse.library.kind == LibraryKind::Disk {
            match database::init_db() {
                Ok(conn) => Some(conn),
                Err(err) => {
                    self.browse.status = format!("Could not save favorite ({err}).");
                    return false;
                }
            }
        } else {
            None
        };
        self.browse.toggle_favorite(conn.as_ref())
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.gate.live() {
            cx.stop_propagation();
            return;
        }
        if self.options.is_some() {
            self.on_options_key(&event.keystroke, false, cx);
            return;
        }
        if self.systems.is_some() {
            self.on_systems_key(&event.keystroke, false, cx);
            return;
        }
        if self.emulators.is_some() {
            self.on_emulator_key(&event.keystroke, false, cx);
            return;
        }
        if self.browse.overlay_open() {
            self.on_overlay_key(&event.keystroke, false, cx);
            return;
        }
        if !event.is_held
            && emulator_key(
                event.keystroke.key.as_str(),
                event.keystroke.key_char.as_deref(),
                event.keystroke.modifiers.control,
            )
        {
            self.open_emulators();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if !event.is_held
            && systems_key(
                event.keystroke.key.as_str(),
                event.keystroke.key_char.as_deref(),
                event.keystroke.modifiers.control,
            )
        {
            self.open_systems();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if !event.is_held
            && options_key(
                event.keystroke.key.as_str(),
                event.keystroke.key_char.as_deref(),
                event.keystroke.modifiers.control,
            )
        {
            self.open_options(OptionsSection::Input);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if !event.is_held
            && scraper_key(
                event.keystroke.key.as_str(),
                event.keystroke.key_char.as_deref(),
                event.keystroke.modifiers.control,
            )
        {
            self.open_options(OptionsSection::Scraper);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.browse.search_open() {
            if self.edit_title_search(event, cx) {
                return;
            }
        } else if !event.is_held
            && title_search_key(
                event.keystroke.key.as_str(),
                event.keystroke.key_char.as_deref(),
                event.keystroke.modifiers.shift,
                event.keystroke.modifiers.control
                    || event.keystroke.modifiers.alt
                    || event.keystroke.modifiers.platform,
            )
        {
            self.browse.open_search();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if !event.is_held
            && !self.browse.search_open()
            && rescan_key(
                event.keystroke.key.as_str(),
                event.keystroke.key_char.as_deref(),
                event.keystroke.modifiers.shift,
                event.keystroke.modifiers.control
                    || event.keystroke.modifiers.alt
                    || event.keystroke.modifiers.platform,
            )
        {
            self.rescan_current();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if !event.is_held
            && theme_key(
                event.keystroke.key.as_str(),
                event.keystroke.key_char.as_deref(),
                event.keystroke.modifiers.shift,
                event.keystroke.modifiers.control
                    || event.keystroke.modifiers.alt
                    || event.keystroke.modifiers.platform,
            )
        {
            self.toggle_appearance(cx);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if !event.is_held
            && !self.browse.search_open()
            && system_sort_key(
                event.keystroke.key.as_str(),
                event.keystroke.key_char.as_deref(),
                event.keystroke.modifiers.shift,
                event.keystroke.modifiers.control
                    || event.keystroke.modifiers.alt
                    || event.keystroke.modifiers.platform,
            )
        {
            self.choose_system_sort(self.browse.library.system_sort.next());
            cx.notify();
            cx.stop_propagation();
            return;
        }
        match self.on_arrow(&event.keystroke, false, cx) {
            ArrowKey::Absent => {}
            ArrowKey::Handled | ArrowKey::Propagate => return,
        }
        if game_menu_key(
            event.keystroke.key.as_str(),
            event.keystroke.modifiers.shift,
            event.keystroke.modifiers.control
                || event.keystroke.modifiers.alt
                || event.keystroke.modifiers.platform,
        ) && !event.is_held
        {
            self.browse.open_game_menu();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if !event.is_held {
            let keystroke = &event.keystroke;
            let modified = keystroke.modifiers.control
                || keystroke.modifiers.alt
                || keystroke.modifiers.platform;
            if let Some(chord) = scrape_chord(
                keystroke.key.as_str(),
                keystroke.key_char.as_deref(),
                keystroke.modifiers.shift,
                modified,
            ) {
                match chord {
                    ScrapeChord::Selected => self.scrape_selected(),
                    ScrapeChord::Missing => self.scrape_missing(),
                }
                cx.notify();
                cx.stop_propagation();
                return;
            }
        }
        let keystroke = &event.keystroke;
        let modified =
            keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform;
        if !modified {
            if let Some(art) = grid_art_key(keystroke.key.as_str())
                .or_else(|| keystroke.key_char.as_deref().and_then(grid_art_key))
            {
                if !event.is_held && self.browse.set_grid_art(art) {
                    self.persist_grid_art();
                    cx.notify();
                }
                cx.stop_propagation();
                return;
            }
        }
        let Some(key) = key_from_parts(
            keystroke.key.as_ref(),
            keystroke.key_char.as_deref(),
            modified,
        ) else {
            return;
        };
        let before = self.browse.cover_width;
        if key == Key::Launch {
            if !event.is_held {
                if let Focus::Menu(item) = self.browse.focus {
                    self.open_menu_item(item);
                } else {
                    self.launch_selected();
                }
            }
        } else if key == Key::ToggleFavorite {
            self.toggle_favorite();
        } else {
            self.browse.apply(key);
        }
        if (self.browse.cover_width - before).abs() >= 0.5 {
            let width = self.browse.cover_width;
            self.cover_slider.update(cx, |state, cx| {
                state.set_value(width, window, cx);
            });
            self.persist_cover_width();
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, cx: &mut Context<Self>) {
        if !self.gate.live() {
            cx.stop_propagation();
            return;
        }
        if self.options.is_some() {
            self.on_options_key(&event.keystroke, true, cx);
            return;
        }
        if self.systems.is_some() {
            self.on_systems_key(&event.keystroke, true, cx);
            return;
        }
        if self.emulators.is_some() {
            self.on_emulator_key(&event.keystroke, true, cx);
            return;
        }
        if self.browse.overlay_open() {
            self.on_overlay_key(&event.keystroke, true, cx);
            return;
        }
        self.on_arrow(&event.keystroke, true, cx);
    }

    fn poll_emulator_pad(&mut self, events: &[PadIn], cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for event in events {
            match self.apply_pad(event) {
                Some(PadAction::Confirm) => {
                    self.confirm_emulators(cx);
                    changed = true;
                }
                Some(PadAction::Back) => {
                    self.finish_emulators();
                    changed = true;
                }
                Some(PadAction::Favorite | PadAction::Menu) | None => {}
            }
        }
        let now = monotonic_ms(self.nav_started);
        let (step_x, step_y) =
            self.hold
                .poll(&self.input, self.pad.horizontal(), self.pad.vertical(), now);
        for dir in [step_x, step_y].into_iter().flatten() {
            let step = self
                .emulators
                .as_mut()
                .map(|dialog| dialog.move_dir(dir))
                .unwrap_or(EmulatorStep::Stay);
            self.apply_emulator_step(step);
            changed = true;
        }
        changed
    }

    fn on_emulator_key(&mut self, keystroke: &Keystroke, release: bool, cx: &mut Context<Self>) {
        if let Some(dir) = arrow_dir(keystroke) {
            let modified = keystroke.modifiers.control
                || keystroke.modifiers.alt
                || keystroke.modifiers.platform;
            if !(modified && !release) {
                let now = monotonic_ms(self.nav_started);
                if release {
                    self.hold.release(&self.input, dir, now);
                } else if self.hold.press(&self.input, dir, now) > 0 {
                    let step = self
                        .emulators
                        .as_mut()
                        .map(|dialog| dialog.move_dir(dir))
                        .unwrap_or(EmulatorStep::Stay);
                    self.apply_emulator_step(step);
                    cx.notify();
                }
            }
            cx.stop_propagation();
            return;
        }
        if release {
            // The focus scope binds Tab to its own focus order and consumes the
            // keydown. The keyup still arrives, and that is what moves this dialog.
            if keystroke.key == "tab" {
                if let Some(dialog) = &mut self.emulators {
                    dialog.tab(keystroke.modifiers.shift);
                }
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        let key = keystroke.key.as_str();
        let modified =
            keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform;
        if key == "escape" {
            self.finish_emulators();
            cx.notify();
        } else if key == "enter" {
            self.confirm_emulators(cx);
            cx.notify();
        } else if key == "backspace" {
            if let Some(dialog) = &mut self.emulators {
                dialog.backspace();
            }
            cx.notify();
        } else if key == "delete" {
            if let Some(dialog) = &mut self.emulators {
                dialog.delete_forward();
            }
            cx.notify();
        } else if key == "space" && !modified {
            let typing = self
                .emulators
                .as_ref()
                .is_some_and(|dialog| dialog.accepts_text());
            if typing {
                if let Some(dialog) = &mut self.emulators {
                    dialog.type_text(" ");
                }
            } else {
                self.confirm_emulators(cx);
            }
            cx.notify();
        } else if !modified {
            let rescan = self
                .emulators
                .as_ref()
                .is_some_and(|dialog| dialog.rescan_key(key, keystroke.key_char.as_deref()));
            if rescan {
                self.rescan_cores();
                cx.notify();
            } else if let Some(text) = typed_text(keystroke) {
                if let Some(dialog) = &mut self.emulators {
                    dialog.type_text(text);
                }
                cx.notify();
            }
        }
        cx.stop_propagation();
    }

    fn on_options_key(&mut self, keystroke: &Keystroke, release: bool, cx: &mut Context<Self>) {
        if let Some(dir) = arrow_dir(keystroke) {
            let modified = keystroke.modifiers.control
                || keystroke.modifiers.alt
                || keystroke.modifiers.platform;
            if !(modified && !release) {
                let now = monotonic_ms(self.nav_started);
                if release {
                    self.hold.release(&self.input, dir, now);
                } else if self.hold.press(&self.input, dir, now) > 0 {
                    let command = self
                        .options
                        .as_mut()
                        .map(|dialog| dialog.move_dir(dir))
                        .unwrap_or(OptionsCommand::None);
                    self.apply_options_command(command, cx);
                    cx.notify();
                }
            }
            cx.stop_propagation();
            return;
        }
        if release {
            if keystroke.key == "tab" {
                if let Some(dialog) = &mut self.options {
                    dialog.tab(keystroke.modifiers.shift);
                }
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        let key = keystroke.key.as_str();
        let modified =
            keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform;
        if key == "escape" {
            self.finish_options();
            cx.notify();
        } else if key == "enter" {
            self.confirm_options(cx);
            cx.notify();
        } else if key == "backspace" {
            if let Some(dialog) = &mut self.options {
                dialog.backspace();
            }
            cx.notify();
        } else if key == "delete" {
            if let Some(dialog) = &mut self.options {
                dialog.delete_forward();
            }
            cx.notify();
        } else if key == "space" && !modified {
            let typing = self
                .options
                .as_ref()
                .is_some_and(|dialog| dialog.accepts_text());
            if typing {
                if let Some(dialog) = &mut self.options {
                    dialog.type_text(" ");
                }
            } else {
                self.confirm_options(cx);
            }
            cx.notify();
        } else if !modified {
            if let Some(text) = typed_text(keystroke) {
                if let Some(dialog) = &mut self.options {
                    dialog.type_text(text);
                }
                cx.notify();
            }
        }
        cx.stop_propagation();
    }

    fn poll_options_pad(&mut self, events: &[PadIn], cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for event in events {
            match self.apply_pad(event) {
                Some(PadAction::Confirm) => {
                    self.confirm_options(cx);
                    changed = true;
                }
                Some(PadAction::Back) => {
                    self.finish_options();
                    changed = true;
                }
                Some(PadAction::Favorite | PadAction::Menu) | None => {}
            }
        }
        let now = monotonic_ms(self.nav_started);
        let (step_x, step_y) =
            self.hold
                .poll(&self.input, self.pad.horizontal(), self.pad.vertical(), now);
        for dir in [step_x, step_y].into_iter().flatten() {
            let command = self
                .options
                .as_mut()
                .map(|dialog| dialog.move_dir(dir))
                .unwrap_or(OptionsCommand::None);
            self.apply_options_command(command, cx);
            changed = true;
        }
        changed
    }

    fn on_systems_key(&mut self, keystroke: &Keystroke, release: bool, cx: &mut Context<Self>) {
        if let Some(dir) = arrow_dir(keystroke) {
            let modified = keystroke.modifiers.control
                || keystroke.modifiers.alt
                || keystroke.modifiers.platform;
            if !(modified && !release) {
                let now = monotonic_ms(self.nav_started);
                if release {
                    self.hold.release(&self.input, dir, now);
                } else if self.hold.press(&self.input, dir, now) > 0 {
                    let step = self
                        .systems
                        .as_mut()
                        .map(|dialog| dialog.move_dir(dir))
                        .unwrap_or(SystemStep::Stay);
                    self.apply_system_step(step, cx);
                    cx.notify();
                }
            }
            cx.stop_propagation();
            return;
        }
        if release {
            if keystroke.key == "tab" {
                if let Some(dialog) = &mut self.systems {
                    dialog.tab(keystroke.modifiers.shift);
                }
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        let key = keystroke.key.as_str();
        let modified =
            keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform;
        if key == "escape" {
            self.back_systems();
            cx.notify();
        } else if key == "enter" {
            self.confirm_systems(cx);
            cx.notify();
        } else if let Some(jump) = list_jump(key) {
            if !modified {
                let moved = self
                    .systems
                    .as_mut()
                    .is_some_and(|dialog| dialog.jump(jump));
                if moved {
                    cx.notify();
                }
            }
        } else if key == "backspace" {
            if let Some(dialog) = &mut self.systems {
                dialog.backspace();
            }
            cx.notify();
        } else if key == "delete" {
            if let Some(dialog) = &mut self.systems {
                dialog.delete_forward();
            }
            cx.notify();
        } else if key == "space" && !modified {
            let typing = self
                .systems
                .as_ref()
                .is_some_and(|dialog| dialog.accepts_text());
            if typing {
                if let Some(dialog) = &mut self.systems {
                    dialog.type_text(" ");
                }
            } else {
                self.confirm_systems(cx);
            }
            cx.notify();
        } else if !modified {
            if let Some(text) = typed_text(keystroke) {
                if let Some(dialog) = &mut self.systems {
                    dialog.type_text(text);
                }
                cx.notify();
            }
        }
        cx.stop_propagation();
    }

    fn poll_systems_pad(&mut self, events: &[PadIn], cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for event in events {
            match self.apply_pad(event) {
                Some(PadAction::Confirm) => {
                    self.confirm_systems(cx);
                    changed = true;
                }
                Some(PadAction::Back) => {
                    self.back_systems();
                    changed = true;
                }
                Some(PadAction::Favorite | PadAction::Menu) | None => {}
            }
        }
        let now = monotonic_ms(self.nav_started);
        let (step_x, step_y) =
            self.hold
                .poll(&self.input, self.pad.horizontal(), self.pad.vertical(), now);
        for dir in [step_x, step_y].into_iter().flatten() {
            let step = self
                .systems
                .as_mut()
                .map(|dialog| dialog.move_dir(dir))
                .unwrap_or(SystemStep::Stay);
            self.apply_system_step(step, cx);
            changed = true;
        }
        changed
    }

    fn screen_busy(&self) -> bool {
        self.emulators.is_some() || self.systems.is_some() || self.options.is_some()
    }

    fn current_screen(&self) -> Option<Screen> {
        if self.emulators.is_some() {
            Some(Screen::Emulators)
        } else if self.systems.is_some() {
            Some(Screen::Systems)
        } else {
            self.options
                .as_ref()
                .map(|dialog| Screen::Options(dialog.section()))
        }
    }

    /// Header buttons share one path. The screen already on display stays put.
    /// Another settings screen is saved the way Esc saves it, then the target opens.
    fn activate_screen(&mut self, target: Screen) {
        match screen_menu(self.current_screen(), target) {
            ScreenMenu::Stay => {}
            ScreenMenu::Section(section) => {
                if let Some(dialog) = &mut self.options {
                    dialog.select(section);
                }
            }
            ScreenMenu::Open(next) => {
                if !self.save_and_close_settings() {
                    return;
                }
                match next {
                    Screen::Emulators => self.open_emulators(),
                    Screen::Systems => self.open_systems(),
                    Screen::Options(section) => self.open_options(section),
                }
            }
        }
    }

    /// Esc's save path: commit the open screen and write it. An unconfirmed
    /// system picker or delete prompt is dropped first, the same way Esc drops
    /// it, and the screen still saves instead of discarding the draft.
    fn save_and_close_settings(&mut self) -> bool {
        if self.emulators.is_some() {
            self.finish_emulators();
            return self.emulators.is_none();
        }
        if self.systems.is_some() {
            if let Some(dialog) = &mut self.systems {
                while dialog.dismiss() {}
            }
            self.finish_systems();
            return self.systems.is_none();
        }
        if self.options.is_some() {
            self.finish_options();
            return self.options.is_none();
        }
        true
    }

    fn open_options(&mut self, section: OptionsSection) {
        if self.screen_busy() {
            return;
        }
        let config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                self.browse.status = format!("Could not read config ({err}).");
                return;
            }
        };
        self.browse.close_overlay();
        self.search = None;
        self.input = config.input.sanitized();
        self.options = Some(Options::open(&config, section));
    }

    fn finish_options(&mut self) {
        if self.options.is_none() {
            return;
        }
        if self.write_options_scraper() {
            self.options = None;
            self.refocus = true;
        }
    }

    fn confirm_options(&mut self, cx: &mut Context<Self>) {
        let command = self
            .options
            .as_mut()
            .map(|dialog| dialog.confirm())
            .unwrap_or(OptionsCommand::None);
        self.apply_options_command(command, cx);
    }

    fn apply_options_command(&mut self, command: OptionsCommand, cx: &mut Context<Self>) {
        match command {
            OptionsCommand::None => {}
            OptionsCommand::Close => self.finish_options(),
            OptionsCommand::Input => self.commit_input(),
            OptionsCommand::Theme => self.commit_theme_from_options(cx),
            OptionsCommand::Cover => {
                if let Some(width) = self.options.as_ref().map(|dialog| dialog.cover_width()) {
                    self.browse.cover_width = width;
                    self.persist_cover_width();
                }
            }
            OptionsCommand::Sort => {
                if let Some(sort) = self.options.as_ref().map(|dialog| dialog.system_sort()) {
                    self.choose_system_sort(sort);
                }
            }
            OptionsCommand::SaveScraper => {
                self.write_options_scraper();
            }
        }
    }

    fn commit_input(&mut self) {
        let Some(input) = self.options.as_ref().map(|dialog| dialog.input()) else {
            return;
        };
        self.input = input;
        if let Err(err) = config::save_input_settings(input) {
            self.browse.status = format!("Could not save input ({err}).");
        }
    }

    fn commit_theme_from_options(&mut self, cx: &mut Context<Self>) {
        let Some(theme) = self
            .options
            .as_ref()
            .map(|dialog| dialog.theme().to_string())
        else {
            return;
        };
        if theme == self.appearance {
            return;
        }
        let previous = self.appearance.clone();
        self.appearance = theme;
        self.apply_appearance(cx);
        if let Err(err) = config::save_theme_name(&self.appearance) {
            self.appearance = previous;
            self.apply_appearance(cx);
            self.browse.status = format!("Could not save theme ({err}).");
        }
    }

    fn write_options_scraper(&mut self) -> bool {
        let Some(draft) = self.options.as_ref().map(|dialog| dialog.scraper().draft()) else {
            return false;
        };
        if let Err(err) = config::save_scraper_settings(draft) {
            let message = format!("Could not save scraper settings ({err}).");
            self.browse.status = message.clone();
            if let Some(dialog) = &mut self.options {
                dialog.scraper_mut().set_error(message);
            }
            return false;
        }
        true
    }

    fn choose_system_sort(&mut self, sort: SystemSort) {
        let previous = self.browse.library.system_sort;
        if !self.browse.set_system_sort(sort) {
            return;
        }
        self.revealed_console = Some(self.browse.console);
        self.scroll_sidebar_to_console(self.browse.console);
        if self.browse.library.kind != LibraryKind::Disk {
            return;
        }
        if let Err(err) = config::save_system_sort(sort) {
            self.browse.set_system_sort(previous);
            self.revealed_console = Some(self.browse.console);
            self.scroll_sidebar_to_console(self.browse.console);
            self.browse.status = format!("Could not save system sort ({err}).");
        }
    }

    fn toggle_appearance(&mut self, cx: &mut Context<Self>) {
        let next = appearance::next_theme(&self.appearance);
        let previous = self.appearance.clone();
        self.appearance = next.to_string();
        self.apply_appearance(cx);
        if let Err(err) = config::save_theme_name(&self.appearance) {
            self.appearance = previous;
            self.apply_appearance(cx);
            self.browse.status = format!("Could not save theme ({err}).");
        }
    }

    fn apply_appearance(&self, cx: &mut Context<Self>) {
        if appearance::is_launchbox(&self.appearance) {
            launchbox_theme().apply(cx);
        } else {
            gpui_omarchy::Theme::follow_system(cx);
        }
    }

    fn close_emulators(&mut self) {
        self.emulators = None;
        self.refocus = true;
    }

    fn finish_emulators(&mut self) {
        if let Some(dialog) = &mut self.emulators {
            dialog.commit();
        }
        if self.write_emulators() {
            self.close_emulators();
        }
    }

    fn open_emulators(&mut self) {
        if self.screen_busy() {
            return;
        }
        let mut config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                self.browse.status = format!("Could not read config ({err}).");
                return;
            }
        };
        let env = cores::DetectEnv::process();
        let found = cores::find_installs(&env);
        if cores::ensure_installs(&mut config.emulators, &found) {
            if let Err(err) = config::save_config(&config) {
                self.browse.status = format!("Could not save config ({err}).");
                return;
            }
            self.browse.library.emulators = config.emulators.clone();
        }
        let catalogs = cores::catalogs_for(&config, &env);
        let missing = config
            .emulators
            .iter()
            .map(|emulator| {
                emulator.kind == crate::types::EmulatorKind::RetroArch
                    && !cores::command_present(&emulator.path, &env, &found)
            })
            .collect();
        self.browse.close_overlay();
        self.search = None;
        self.emulators = Some(Emulators::open(&config, catalogs, missing));
    }

    fn confirm_emulators(&mut self, _cx: &mut Context<Self>) {
        let step = self
            .emulators
            .as_mut()
            .map(|dialog| dialog.confirm())
            .unwrap_or(EmulatorStep::Stay);
        self.apply_emulator_step(step);
    }

    fn apply_emulator_step(&mut self, step: EmulatorStep) {
        match step {
            EmulatorStep::Stay => {}
            EmulatorStep::Write => {
                self.write_emulators();
            }
            EmulatorStep::Rescan => self.rescan_cores(),
        }
    }

    fn write_emulators(&mut self) -> bool {
        let emulators = {
            let Some(dialog) = &self.emulators else {
                return false;
            };
            dialog.emulators().to_vec()
        };
        let mut config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                if let Some(dialog) = &mut self.emulators {
                    dialog.set_error(format!("Could not read config ({err})."));
                }
                return false;
            }
        };
        emulators::apply_emulators(&mut config, &emulators);
        if let Err(err) = config::save_config(&config) {
            if let Some(dialog) = &mut self.emulators {
                dialog.set_error(format!("Could not save config ({err})."));
            }
            return false;
        }
        self.copy_library(&config);
        true
    }

    fn rescan_cores(&mut self) {
        if let Some(dialog) = &mut self.emulators {
            dialog.commit();
        }
        let mut config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                self.browse.status = format!("Could not read config ({err}).");
                return;
            }
        };
        if let Some(dialog) = &self.emulators {
            emulators::apply_emulators(&mut config, dialog.emulators());
        }
        let env = cores::DetectEnv::process();
        let target = self
            .emulators
            .as_ref()
            .and_then(|dialog| dialog.retroarch_target());
        let Some(target) = target else {
            return;
        };
        let catalog = cores::catalog_for_emulator(&target, &env);
        if !target.id.is_empty() {
            cores::repoint_id(&mut config.consoles, &target.id, &catalog);
        }
        if let Err(err) = config::save_config(&config) {
            self.browse.status = format!("Could not save config ({err}).");
            return;
        }
        self.copy_library(&config);
        let status = match &catalog.note {
            Some(note) => note.clone(),
            None => format!(
                "Rescanned {} cores from {}.",
                catalog.cores.len(),
                catalog.directory.display()
            ),
        };
        if let Some(dialog) = &mut self.emulators {
            dialog.set_catalog(catalog);
        }
        self.browse.status = status;
    }

    fn open_systems(&mut self) {
        if self.screen_busy() {
            return;
        }
        let config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                self.browse.status = format!("Could not read config ({err}).");
                return;
            }
        };
        self.browse.close_overlay();
        self.search = None;
        let catalogs = cores::catalogs_for(&config, &cores::DetectEnv::process());
        self.systems = Some(Systems::open(&config, catalogs));
    }

    fn back_systems(&mut self) {
        let closed_picker = self.systems.as_mut().is_some_and(|dialog| dialog.dismiss());
        if !closed_picker {
            self.finish_systems();
        }
    }

    fn finish_systems(&mut self) {
        if self.write_systems() {
            self.systems = None;
            self.refocus = true;
        }
    }

    fn pick_system(&mut self, index: usize, cx: &mut Context<Self>) {
        let step = self
            .systems
            .as_mut()
            .map(|dialog| dialog.choose(index))
            .unwrap_or(SystemStep::Stay);
        self.apply_system_step(step, cx);
    }

    fn confirm_systems(&mut self, cx: &mut Context<Self>) {
        let step = self
            .systems
            .as_mut()
            .map(|dialog| dialog.confirm())
            .unwrap_or(SystemStep::Stay);
        self.apply_system_step(step, cx);
    }

    fn apply_system_step(&mut self, step: SystemStep, cx: &mut Context<Self>) {
        match step {
            SystemStep::Stay => {}
            SystemStep::Write => {
                self.write_systems();
            }
            SystemStep::Rescan => {
                if self.write_systems() {
                    if let Some(id) = self
                        .systems
                        .as_ref()
                        .and_then(|dialog| dialog.selected_id())
                    {
                        self.rescan_console_id(&id);
                    }
                }
            }
            SystemStep::Browse => self.pick_folder(cx),
            SystemStep::ScrapeMissing => {
                if let Some(id) = self
                    .systems
                    .as_ref()
                    .and_then(|dialog| dialog.selected_id())
                {
                    let announce = !self.scrape_runs.tracks(&id);
                    self.scrape_missing_id(&id, announce);
                }
            }
        }
    }

    fn write_systems(&mut self) -> bool {
        let rows = {
            let Some(dialog) = &self.systems else {
                return false;
            };
            dialog.rows().to_vec()
        };
        let mut config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                self.browse.status = format!("Could not read config ({err}).");
                return false;
            }
        };
        systems::apply_systems(&mut config, &rows);
        if let Err(err) = config::save_config(&config) {
            self.browse.status = format!("Could not save config ({err}).");
            return false;
        }
        if let Some(id) = self
            .systems
            .as_mut()
            .and_then(|dialog| dialog.take_dropped())
        {
            match database::init_db() {
                Ok(conn) => {
                    if let Err(err) = database::delete_console_library(&conn, &id) {
                        self.browse.status =
                            format!("Removed {id} from config. Library rows stayed ({err}).");
                    }
                }
                Err(err) => {
                    self.browse.status =
                        format!("Removed {id} from config. Library rows stayed ({err}).");
                }
            }
        }
        self.copy_library(&config);
        true
    }

    fn copy_library(&mut self, config: &config::Config) {
        self.browse.library.emulators = config.emulators.clone();
        if self.browse.library.kind == LibraryKind::Disk {
            self.browse.sync_consoles(&config.consoles);
            return;
        }
        for shelf in &mut self.browse.library.shelves {
            if let Some(console) = config
                .consoles
                .iter()
                .find(|item| item.id == shelf.console.id)
            {
                shelf.console.name = console.name.clone();
                shelf.console.extensions = console.extensions.clone();
                shelf.console.rom_dirs = console.rom_dirs.clone();
                shelf.console.emulator = console.emulator.clone();
                shelf.console.core = console.core.clone();
                shelf.console.extra_args = console.extra_args.clone();
            }
        }
    }

    /// Browse opens a folder through the XDG desktop portal
    /// via [`App::prompt_for_paths`]. A typed path still works when the portal is unavailable.
    fn pick_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select".into()),
        });
        cx.spawn(async move |this, cx| {
            let picked = match rx.await {
                Ok(Ok(Some(paths))) => paths
                    .into_iter()
                    .next()
                    .map(Picked::Path)
                    .unwrap_or(Picked::Cancel),
                Ok(Ok(None)) | Err(_) => Picked::Cancel,
                Ok(Err(err)) => {
                    Picked::Failed(format!("Folder picker unavailable ({err}). Type the path."))
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.finish_pick(picked, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_pick(&mut self, picked: Picked, cx: &mut Context<Self>) {
        match picked {
            Picked::Cancel => {}
            Picked::Path(path) => {
                let step = self
                    .systems
                    .as_mut()
                    .map(|dialog| dialog.add_rom_dir(path))
                    .unwrap_or(SystemStep::Stay);
                self.apply_system_step(step, cx);
            }
            Picked::Failed(message) => self.browse.status = message,
        }
    }

    /// Arrows share the hold clock with the pad. Escape closes. Enter confirms.
    /// A title or search box takes typed characters; every other key stays here.
    fn on_overlay_key(&mut self, keystroke: &Keystroke, release: bool, cx: &mut Context<Self>) {
        if let Some(dir) = arrow_dir(keystroke) {
            let modified = keystroke.modifiers.control
                || keystroke.modifiers.alt
                || keystroke.modifiers.platform;
            if !(modified && !release) {
                let now = monotonic_ms(self.nav_started);
                if release {
                    self.hold.release(&self.input, dir, now);
                } else if self.hold.press(&self.input, dir, now) > 0 {
                    self.browse.move_overlay(dir);
                    cx.notify();
                }
            }
            cx.stop_propagation();
            return;
        }
        if release {
            cx.stop_propagation();
            return;
        }
        let key = keystroke.key.as_str();
        if key == "escape" {
            self.dismiss_overlay();
            cx.notify();
        } else if key == "enter" {
            self.confirm_overlay();
            cx.notify();
        } else if key == "backspace" {
            self.browse.backspace();
            cx.notify();
        } else if key == "delete" {
            self.browse.delete_forward();
            cx.notify();
        } else if self.browse.menu_open() && matches!(key, "j" | "k") {
            let dir = if key == "j" { NavDir::Down } else { NavDir::Up };
            self.browse.move_overlay(dir);
            cx.notify();
        } else if self.browse.accepts_text() {
            if let Some(text) = typed_text(keystroke) {
                self.browse.insert_text(text);
                cx.notify();
            }
        } else if key == "space" {
            self.confirm_overlay();
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn confirm_overlay(&mut self) {
        let busy = self.apply.is_some();
        match self.browse.confirm_overlay(busy) {
            OverlayCommand::None => {}
            OverlayCommand::CommitRename => self.commit_rename(),
            OverlayCommand::CommitDelete => self.commit_delete(),
            OverlayCommand::Search { query, console_id } => self.start_search(query, console_id),
            OverlayCommand::Apply { game_id, candidate } => self.start_apply(game_id, candidate),
        }
    }

    fn dismiss_overlay(&mut self) {
        let scrape = matches!(self.browse.overlay, Overlay::Scrape(_));
        self.browse.close_overlay();
        if scrape {
            self.search = None;
        }
    }

    fn commit_rename(&mut self) {
        if self.browse.library.kind == LibraryKind::Demo {
            self.browse.commit_rename(None);
            return;
        }
        match database::init_db() {
            Ok(conn) => {
                self.browse.commit_rename(Some(&conn));
            }
            Err(err) => {
                self.browse.close_overlay();
                self.browse.status = format!("Could not rename the game ({err}).");
            }
        }
    }

    fn commit_delete(&mut self) {
        if self.browse.library.kind == LibraryKind::Demo {
            self.browse.commit_delete(None);
            return;
        }
        let removed = match database::init_db() {
            Ok(conn) => self.browse.commit_delete(Some(&conn)),
            Err(err) => {
                self.browse.close_overlay();
                self.browse.status = format!("Could not remove the game from the library: {err}");
                return;
            }
        };
        let Some(removed) = removed else {
            return;
        };
        let notes = delete_notes(&removed.game, removed.options);
        if !notes.is_empty() {
            self.browse.status = format!(
                "Removed {} from the library. {}",
                removed.game.display_title(),
                notes.join(" ")
            );
        }
    }

    fn start_search(&mut self, query: String, console_id: String) {
        let config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                self.browse
                    .fail_search(format!("Could not read scraper settings ({err})."));
                return;
            }
        };
        self.search = Some(scraper::spawn_name_search(
            query,
            console_id,
            config.scraper,
            scraper::fixture_dir_from_env(),
        ));
    }

    fn scrape_selected(&mut self) {
        if let OverlayCommand::Search { query, console_id } = self.browse.scrape_selected() {
            self.start_search(query, console_id);
        }
    }

    fn scrape_missing(&mut self) {
        let Some(id) = self.browse.shelf().map(|shelf| shelf.console.id.clone()) else {
            self.browse.status = "Select a system before scraping missing artwork.".into();
            return;
        };
        self.scrape_missing_id(&id, true);
    }

    fn scrape_missing_id(&mut self, console_id: &str, announce_busy: bool) {
        if self.scrape_runs.tracks(console_id) {
            if announce_busy {
                self.browse.status = "A scrape is already running.".into();
            }
            return;
        }
        let Some(games) = self.browse.missing_scrape_games_for(console_id) else {
            return;
        };
        self.enqueue_missing(console_id, games, announce_busy);
    }

    fn enqueue_missing(&mut self, console_id: &str, games: Vec<Game>, announce_busy: bool) {
        match self.scrape_runs.request_missing(console_id, games) {
            scraper::Admit::Busy => {
                if announce_busy {
                    self.browse.status = "A scrape is already running.".into();
                }
            }
            scraper::Admit::Queued => {}
            scraper::Admit::Start(batch) => self.launch_missing(batch),
        }
    }

    fn enqueue_added(&mut self, console_id: &str, games: Vec<Game>) {
        if let scraper::Admit::Start(batch) = self.scrape_runs.request_added(console_id, games) {
            self.launch_missing(batch);
        }
    }

    fn launch_missing(&mut self, batch: ScrapeBatch) {
        let config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                self.scrape_runs.clear();
                self.apply = None;
                self.browse.status = format!("Could not read scraper settings ({err}).");
                return;
            }
        };
        let games = self.games_now(batch);
        self.browse.status = "Scraping artwork…".into();
        self.apply = Some(scraper::spawn_scrape(
            games,
            config.scraper,
            scraper::fixture_dir_from_env(),
        ));
    }

    fn games_now(&self, batch: ScrapeBatch) -> Vec<Game> {
        batch
            .games
            .into_iter()
            .map(|game| self.browse.game_by_id(&game.id).cloned().unwrap_or(game))
            .collect()
    }

    fn promote_scrape(&mut self) {
        if let Some(batch) = self.scrape_runs.finish() {
            self.launch_missing(batch);
        }
    }

    fn start_apply(&mut self, game_id: String, candidate: crate::scraper::ScrapeCandidate) {
        self.search = None;
        let Some(game) = self.browse.game_by_id(&game_id).cloned() else {
            self.browse.status = "Select a game.".into();
            return;
        };
        if self.browse.library.kind == LibraryKind::Demo {
            self.browse.status = "Demo library. Scrape needs a library on disk.".into();
            return;
        }
        let config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                self.browse.status = format!("Could not read scraper settings ({err}).");
                return;
            }
        };
        if self.apply.is_some() || !self.scrape_runs.request_one(&game.console) {
            self.browse.status = "A scrape is already running.".into();
            return;
        }
        self.browse.status = "Scraping artwork…".into();
        self.apply = Some(scraper::spawn_apply_candidate(
            game,
            candidate,
            config.scraper,
            scraper::fixture_dir_from_env(),
        ));
    }

    fn poll_jobs(&mut self) -> bool {
        let mut changed = false;
        // One update per tick. A scrape can queue many saves; draining them
        // here would decode images on the UI thread.
        let search = self.search.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(outcome) => Some(Ok(outcome)),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(())),
        });
        if let Some(search) = search {
            self.search = None;
            match search {
                Ok(outcome) => self.browse.finish_search(outcome),
                Err(()) => self.browse.fail_search("Search stopped.".into()),
            }
            changed = true;
        }
        let update = self.apply.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(update) => Some(Ok(update)),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(())),
        });
        if let Some(update) = update {
            match update {
                Ok(ScrapeUpdate::Status(text)) => self.browse.status = text,
                Ok(ScrapeUpdate::Saved { game_id, media }) => self.store_media(&game_id, &media),
                Ok(ScrapeUpdate::Metadata { game_id, metadata }) => {
                    self.store_metadata(&game_id, metadata);
                }
                Ok(ScrapeUpdate::Done(text)) => {
                    self.apply = None;
                    self.browse.status = text;
                    self.promote_scrape();
                }
                Err(()) => {
                    self.apply = None;
                    self.browse.status = "Scrape stopped.".into();
                    self.promote_scrape();
                }
            }
            changed = true;
        }
        while let Ok(note) = self.play_rx.try_recv() {
            self.gate.end_playing();
            match note {
                PlayNote::Saved {
                    game_id,
                    play_count,
                    play_time,
                    last_played,
                } => {
                    if let Some(title) =
                        self.browse
                            .apply_play_stats(&game_id, play_count, play_time, last_played)
                    {
                        self.browse.status = format!("Played {title}.");
                    }
                }
                PlayNote::Failed(message) => self.browse.status = message,
            }
            changed = true;
        }
        if let Some(note) = self.rescan_rx.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(note) => Some(note),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(RescanNote::Failed("Rescan stopped.".into()))
            }
        }) {
            self.rescan_rx = None;
            match note {
                RescanNote::Done {
                    console_id,
                    name,
                    games,
                    stats,
                } => {
                    let previous = self.browse.game_ids(&console_id);
                    let added = scanner::added_games(&previous, &games);
                    for game in &games {
                        for media in &game.media {
                            crate::covers::invalidate_stale(&media.path);
                        }
                    }
                    let count = games.len();
                    if self.browse.replace_console_games(&console_id, games, stats) {
                        self.browse.status = format!("Scanned {count} games in {name}.");
                        if !added.is_empty() {
                            self.enqueue_added(&console_id, added);
                        }
                    }
                }
                RescanNote::Failed(message) => {
                    self.browse.status = format!("Rescan failed: {message}");
                }
            }
            changed = true;
        }
        changed
    }

    fn store_media(&mut self, game_id: &str, media: &Media) {
        if self.browse.library.kind != LibraryKind::Disk {
            return;
        }
        let Ok(conn) = database::init_db() else {
            self.browse.status = "Could not save artwork.".into();
            return;
        };
        let id = game_id.to_string();
        if database::set_game_media(&conn, &id, media).is_err() {
            self.browse.status = "Could not save artwork.".into();
            return;
        }
        if let Some(old) = self
            .browse
            .game_by_id(game_id)
            .and_then(|game| file_for(game, media.kind))
        {
            if old != media.path {
                crate::covers::invalidate(&old);
            }
        }
        crate::covers::invalidate(&media.path);
        self.browse.remember_media(game_id, media);
    }

    fn store_metadata(&mut self, game_id: &str, metadata: GameMetadata) {
        if self.browse.library.kind != LibraryKind::Disk {
            return;
        }
        let id = game_id.to_string();
        let saved =
            database::init_db().and_then(|conn| database::set_game_metadata(&conn, &id, &metadata));
        if saved.is_err() {
            self.browse.status = "Could not save metadata.".into();
            return;
        }
        self.browse.remember_metadata(game_id, metadata);
    }

    fn on_arrow(
        &mut self,
        keystroke: &Keystroke,
        release: bool,
        cx: &mut Context<Self>,
    ) -> ArrowKey {
        let Some(dir) = arrow_dir(keystroke) else {
            return ArrowKey::Absent;
        };
        let modified =
            keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform;
        if modified && !release {
            return ArrowKey::Propagate;
        }
        let now = monotonic_ms(self.nav_started);
        if release {
            self.hold.release(&self.input, dir, now);
        } else {
            let steps = self.hold.press(&self.input, dir, now);
            if steps > 0 {
                self.browse.apply(Key::Arrow(dir));
                cx.notify();
            }
        }
        cx.stop_propagation();
        ArrowKey::Handled
    }

    fn apply_cover_width(&mut self, width: f32, cx: &mut Context<Self>) {
        let width = clamp_cover_width(width);
        if (self.browse.cover_width - width).abs() < 0.5 {
            return;
        }
        self.browse.cover_width = width;
        self.persist_cover_width();
        cx.notify();
    }

    fn persist_grid_art(&mut self) {
        if self.browse.library.kind != LibraryKind::Disk {
            return;
        }
        let Some(shelf) = self.browse.shelf() else {
            return;
        };
        let id = shelf.console.id.clone();
        let art = shelf.console.grid_art;
        match config::save_console_grid_art(&id, art) {
            Ok(()) => {
                if self.browse.status.starts_with("Could not save grid art") {
                    self.browse.status.clear();
                }
            }
            Err(err) => {
                self.browse.status = format!("Could not save grid art ({err}).");
            }
        }
    }

    fn persist_cover_width(&mut self) {
        match config::save_cover_width(self.browse.cover_width) {
            Ok(()) => {
                if self.browse.status.starts_with("Could not save cover width") {
                    self.browse.status.clear();
                }
            }
            Err(err) => {
                self.browse.status = format!("Could not save cover width ({err}).");
            }
        }
    }

    /// Characters go into the title filter. Arrows, Tab, and Enter still move
    /// and launch, so the grid stays keyboard-first while the query changes.
    fn edit_title_search(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let keystroke = &event.keystroke;
        let modified =
            keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform;
        if modified {
            return false;
        }
        let key = keystroke.key.as_str();
        if is_launch_key(key) || matches!(key, "up" | "down" | "left" | "right" | "tab") {
            return false;
        }
        if key == "escape" {
            self.browse.close_search();
            cx.notify();
            cx.stop_propagation();
            return true;
        }
        if key == "backspace" {
            self.browse.pop_search();
            cx.notify();
            cx.stop_propagation();
            return true;
        }
        if title_search_key(
            key,
            keystroke.key_char.as_deref(),
            keystroke.modifiers.shift,
            false,
        ) {
            if !event.is_held {
                self.browse.close_search();
                cx.notify();
            }
            cx.stop_propagation();
            return true;
        }
        if let Some(text) = typed_text(keystroke) {
            self.browse.push_search(text);
            cx.notify();
            cx.stop_propagation();
            return true;
        }
        false
    }

    fn rescan_current(&mut self) {
        let Some(id) = self.browse.shelf().map(|shelf| shelf.console.id.clone()) else {
            self.browse.status = "Select a system before rescanning.".into();
            return;
        };
        self.rescan_console_id(&id);
    }

    fn rescan_console_id(&mut self, id: &str) {
        if self.rescan_rx.is_some() {
            self.browse.status = "A rescan is already running.".into();
            return;
        }
        if self.browse.library.kind == LibraryKind::Demo {
            self.browse.status = "Demo library. Rescan needs a library on disk.".into();
            return;
        }
        let Some(console) = self
            .browse
            .library
            .shelves
            .iter()
            .find(|shelf| shelf.console.id == id)
            .map(|shelf| shelf.console.clone())
        else {
            self.browse.status = "Select a system before rescanning.".into();
            return;
        };
        let name = console.name.clone();
        self.browse.status = format!("Scanning {name}…");
        let (tx, rx) = std::sync::mpsc::channel();
        self.rescan_rx = Some(rx);
        std::thread::spawn(move || {
            let note = match rescan_console(console) {
                Ok((console_id, name, games, stats)) => RescanNote::Done {
                    console_id,
                    name,
                    games,
                    stats,
                },
                Err(err) => RescanNote::Failed(err),
            };
            let _ = tx.send(note);
        });
    }

    fn launch_selected(&mut self) {
        if !self.gate.live() {
            return;
        }
        let Some(game) = self.browse.selected_game().cloned() else {
            self.browse.status = "Select a game, then press Enter.".into();
            return;
        };
        let Some(launch) = resolve_launch(&self.browse.library, &game) else {
            self.browse.status =
                "This system has no emulator. Use Manage Emulators to add one and assign it."
                    .into();
            return;
        };
        match launcher::launch_game_tracked(&launch, &game.rom) {
            Ok(running) => {
                self.gate.begin_playing();
                let tx = self.play_tx.clone();
                let game_id = game.id.clone();
                let console_id = game.console.clone();
                let disk = self.browse.library.kind == LibraryKind::Disk;
                let base_count = game.play_count;
                let base_time = game.play_time;
                std::thread::spawn(move || {
                    let started = Instant::now();
                    running.wait();
                    let elapsed = u32::try_from(started.elapsed().as_secs()).unwrap_or(u32::MAX);
                    let note = if disk {
                        match save_play_session(&game_id, &console_id, elapsed) {
                            Ok(saved) => saved,
                            Err(err) => PlayNote::Failed(err),
                        }
                    } else {
                        PlayNote::Saved {
                            game_id,
                            play_count: base_count.saturating_add(1),
                            play_time: base_time.saturating_add(elapsed),
                            last_played: chrono::Utc::now(),
                        }
                    };
                    let _ = tx.send(note);
                });
                self.browse.status = format!("Launched {}.", game.display_title());
            }
            Err(err) => self.browse.status = err.to_string(),
        }
    }

    fn grid_base(&self) -> ScrollHandle {
        self.grid_scroll.0.borrow().base_handle.clone()
    }

    /// Scroll the selected row into view after the scrollports have a real size.
    /// Remembering the last reveal keeps a wheel gesture from snapping back.
    fn reveal_selection(&mut self) {
        let scroll = self.grid_base();
        if scroll.bounds().size.height <= px(0.) {
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
            scroll.set_offset(point(px(0.), px(0.)));
            self.scroll_sidebar_to_console(console);
        }
        let revealed = match game {
            Some(index) => self.reveal_grid_row(row_of(index, columns)),
            None => true,
        };
        if revealed {
            self.revealed_console = Some(console);
            self.revealed_game = game;
            self.revealed_columns = columns;
        }
    }

    fn scroll_sidebar_to_console(&self, index: usize) {
        if self.sidebar_scroll.bounds().size.height > px(0.) {
            self.sidebar_scroll.scroll_to_item(index);
        }
    }

    /// Move the selected row into view from the previous frame's measured row
    /// height. Returns false until the list has been laid out. Each row owns
    /// its trailing gap, so the reveal box is the row itself.
    fn reveal_grid_row(&self, row: usize) -> bool {
        let columns = self.browse.columns.max(1);
        let games = self.browse.visible_len();
        let rows = if games == 0 {
            0
        } else {
            games.div_ceil(columns)
        };
        let measured = self.grid_scroll.0.borrow().last_item_size;
        let Some(measured) = measured else {
            return false;
        };
        if rows == 0 || row >= rows || measured.contents.height <= px(0.) {
            return false;
        }
        let scroll = self.grid_base();
        let viewport_height = measured.item.height.as_f32();
        if viewport_height <= 0.0 {
            return false;
        }
        let row_height = measured.contents.height.as_f32() / rows as f32;
        let row_top = row as f32 * row_height;
        let row_bottom = row_top + row_height;
        let max_scroll = scroll.max_offset().y.as_f32();
        let (before, after) = row_reveal_insets(
            row,
            rows,
            row_top,
            row_bottom,
            viewport_height,
            max_scroll,
            0.0,
        );
        let next = reveal_row_scroll(
            -(scroll.offset().y.as_f32()),
            viewport_height,
            max_scroll,
            row_top,
            row_bottom,
            before,
            after,
        );
        let y = if next <= 0.0 { px(0.) } else { px(-next) };
        scroll.set_offset(point(px(0.), y));
        true
    }

    /// Save and leave whichever settings screen is open. Same writes as Esc
    /// and as switching screens from the menu. On the library this does nothing.
    fn go_library(&mut self) {
        self.save_and_close_settings();
    }

    /// Start thumbs for cards that are still missing. In-flight work stays capped.
    fn pump_thumbs(&mut self, cx: &mut Context<Self>) {
        while let Some(work) = crate::covers::pop_work() {
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { crate::covers::produce(work) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if crate::covers::complete(result) {
                        cx.notify();
                    }
                    this.pump_thumbs(cx);
                });
            })
            .detach();
        }
    }

    /// One grid row per index. The list asks for the visible range only.
    fn game_rows(
        &mut self,
        range: std::ops::Range<usize>,
        scale: f32,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        crate::covers::begin_visible();
        let count_tiles = crate::scroll_profile::count_pass();
        let columns = self.browse.columns.max(1);
        let cover = self.browse.cover_width;
        let selected = self.browse.game;
        let menu_cursor = self.browse.menu_cursor();
        let demo = self.browse.library.kind == LibraryKind::Demo;
        let art = self
            .browse
            .shelf()
            .map(|shelf| shelf.console.grid_art)
            .unwrap_or(GridArt::BoxArt);
        let console_name = self
            .browse
            .shelf()
            .map(|shelf| shelf.console.name.clone())
            .unwrap_or_default();
        let games: Vec<&Game> = self.browse.visible_games().collect();
        let mut rows = Vec::with_capacity(range.len());
        for row in range {
            let start = row * columns;
            let end = (start + columns).min(games.len());
            let mut line = div()
                .w_full()
                .flex()
                .flex_row()
                .flex_shrink_0()
                .gap(px(TILE_GAP))
                .pb(px(TILE_GAP));
            for index in start..end {
                if count_tiles {
                    crate::scroll_profile::count_tile();
                }
                let menu = (selected == Some(index)).then_some(menu_cursor).flatten();
                line = line.child(tile(
                    index,
                    games[index],
                    &console_name,
                    art,
                    cover,
                    scale,
                    selected == Some(index),
                    menu,
                    demo,
                    cx,
                ));
            }
            rows.push(line.into_any_element());
        }
        crate::covers::end_visible();
        rows
    }

    fn reveal_emulators(&mut self) {
        let Some(dialog) = &self.emulators else {
            return;
        };
        if self.emulator_scroll.bounds().size.height > px(0.) {
            self.emulator_scroll.scroll_to_item(dialog.selected_index());
        }
    }

    fn reveal_systems(&mut self, window: &mut Window) {
        if let Some(dialog) = &self.systems {
            if self.systems_scroll.bounds().size.height > px(0.) {
                self.systems_scroll.scroll_to_item(dialog.selected_index());
            }
        }
        if self.reveal_system_picker() {
            window.request_animation_frame();
        }
    }

    fn reveal_system_picker(&mut self) -> bool {
        let mark = self.systems.as_ref().and_then(|dialog| dialog.open_mark());
        if mark == self.picker_mark {
            return false;
        }
        let Some((_, cursor)) = mark else {
            self.picker_scroll.set_offset(point(px(0.), px(0.)));
            self.picker_mark = None;
            return false;
        };
        let scroll = &self.picker_scroll;
        if scroll.bounds().size.height > px(0.) && scroll.children_count() > cursor {
            scroll.scroll_to_item(cursor);
            self.picker_mark = mark;
            false
        } else {
            true
        }
    }

    fn reveal_options(&mut self) {
        let Some(dialog) = &self.options else {
            return;
        };
        if self.options_scroll.bounds().size.height > px(0.) {
            self.options_scroll.scroll_to_item(dialog.selected_index());
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.pump_thumbs(cx);
        crate::scroll_profile::begin_frame();
        let width = window.viewport_size().width.as_f32();
        let frame = self.browse.tile_frame();
        self.browse
            .set_columns(columns_for(width, self.browse.details_open, frame.width));
        self.reveal_selection();
        self.reveal_emulators();
        self.reveal_systems(window);
        self.reveal_options();
        if let Some(options) = &self.options {
            let width = options.cover_width();
            if (self.browse.cover_width - width).abs() >= 0.5 {
                self.browse.cover_width = width;
                self.cover_slider.update(cx, |state, cx| {
                    state.set_value(width, window, cx);
                });
            }
        }
        if let Overlay::Scrape(prompt) = &self.browse.overlay {
            if prompt.slot == ScrapeSlot::Results {
                self.scrape_scroll.scroll_to_item(prompt.cursor);
            }
        }
        if !self.armed {
            self.armed = true;
            self.focus_handle.focus(window, cx);
        }
        if self.activation.is_none() {
            self.gate.set_focused(window.is_window_active());
            self.activation = Some(cx.observe_window_activation(window, |this, window, _cx| {
                this.gate.set_focused(window.is_window_active());
            }));
        }
        if self.refocus {
            self.refocus = false;
            self.focus_handle.focus(window, cx);
        }

        let background = cx.omarchy().background;
        let foreground = cx.omarchy().foreground;
        let font = cx.omarchy().font.clone();
        let library = self.current_screen().is_none();
        let note = if library {
            note_bar(&self.browse.library.note, cx)
        } else {
            None
        };
        let content = if let Some(screen) =
            emulator_screen(self.emulators.as_ref(), &self.emulator_scroll, cx)
        {
            screen
        } else if let Some(screen) = systems_screen(
            self.systems.as_ref(),
            &self.systems_scroll,
            &self.picker_scroll,
            self.systems
                .as_ref()
                .and_then(|dialog| dialog.selected_id())
                .is_some_and(|id| self.scrape_runs.tracks(&id)),
            cx,
        ) {
            screen
        } else if let Some(screen) = options_screen(self.options.as_ref(), &self.options_scroll, cx)
        {
            screen
        } else {
            body(&self.browse, &self.grid_scroll, &self.sidebar_scroll, cx).into_any_element()
        };

        let ui = focus_scope("retromarchy")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .font_family(font)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .capture_key_up(cx.listener(|this, event: &KeyUpEvent, _window, cx| {
                this.on_key_up(event, cx);
            }))
            .child(header(&self.browse, cx))
            .children(note)
            .child(content)
            .child(status_line(&self.browse, &self.cover_slider, window, cx))
            .children(game_dialog(&self.browse, &self.scrape_scroll, cx));
        let grid_scroll = self.grid_base();
        crate::scroll_profile::after_frame(&grid_scroll, window, cx);
        ui
    }
}

fn library_title(id: &'static str, cx: &Context<Shell>) -> impl IntoElement {
    div()
        .id(id)
        .flex_none()
        .font_weight(gpui_kit::FontWeight::BOLD)
        .cursor_pointer()
        .on_click(cx.listener(|this: &mut Shell, _: &ClickEvent, window, cx| {
            this.go_library();
            this.focus_handle.focus(window, cx);
            cx.notify();
        }))
        .child("Retromarchy")
}

fn header(browse: &Browse, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut row = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .h(px(48.))
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .child(library_title("library-title", cx));
    if browse.library.kind == LibraryKind::Demo {
        row = row.child(badge("Demo library", Status::Warning, cx));
    }
    row.child(
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(2.))
            .child(menu_button(
                "manage-emulators",
                "Manage Emulators",
                MenuItem::Emulators,
                browse,
                cx,
            ))
            .child(menu_button(
                "manage-systems",
                "Manage Systems",
                MenuItem::Systems,
                browse,
                cx,
            ))
            .child(menu_button(
                "options",
                "Options",
                MenuItem::Options,
                browse,
                cx,
            )),
    )
    .child(if browse.search_open() {
        search_field(browse.query(), cx).into_any_element()
    } else {
        div().flex_1().into_any_element()
    })
    .child(filter_control(browse, cx))
    .children((!browse.search_open()).then(|| {
        div()
            .flex()
            .flex_shrink_1()
            .min_w(px(0.))
            .overflow_hidden()
            .gap(px(0.))
            .items_center()
            .text_color(theme.secondary)
            .text_size(px(12.))
            .child(keycap("arrows", cx))
            .child("move")
            .child(keycap("d", cx))
            .child("details")
            .child(keycap("f", cx))
            .child("fav")
            .child(keycap("/", cx))
            .child("filter")
            .child(keycap("r", cx))
            .child("scan")
            .child(keycap("s", cx))
            .child("scrape")
            .child(keycap("S", cx))
            .child("miss")
            .child(keycap("menu", cx))
            .child("game")
            .child(keycap("esc", cx))
            .child("clear")
    }))
}

fn search_field(query: &str, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let empty = query.is_empty();
    div()
        .id("title-search")
        .flex()
        .flex_1()
        .min_w(px(160.))
        .items_center()
        .h(px(28.))
        .px(px(8.))
        .gap(px(6.))
        .bg(theme.background)
        .border_1()
        .border_color(theme.accent)
        .overflow_hidden()
        .text_size(px(13.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(if empty {
                    theme.secondary
                } else {
                    theme.foreground
                })
                .child(if empty {
                    "Filter titles".to_string()
                } else {
                    query.to_string()
                }),
        )
        .child(div().w(px(1.)).h(px(14.)).bg(theme.accent))
}

fn menu_button(
    id: &'static str,
    label: &'static str,
    item: MenuItem,
    browse: &Browse,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let focused = matches!(browse.focus, Focus::Menu(current) if current == item);
    let mut control = button(id, label, ButtonVariant::Secondary, cx)
        .px(px(4.))
        .flex_shrink_0()
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                this.activate_screen(screen_for_menu(item));
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        );
    if focused {
        control = control
            .border_color(theme.accent)
            .bg(theme.selected_fill())
            .text_color(theme.accent);
    }
    control
}

fn filter_control(browse: &Browse, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .id("grid-filter")
        .flex()
        .flex_shrink_0()
        .border_1()
        .border_color(theme.border)
        .child(filter_segment(browse, GridFilter::All, false, cx))
        .child(filter_segment(browse, GridFilter::Favorites, true, cx))
}

fn filter_segment(
    browse: &Browse,
    filter: GridFilter,
    divider: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let on = browse.filter == filter;
    let mut segment = div()
        .id(match filter {
            GridFilter::All => "filter-all",
            GridFilter::Favorites => "filter-favorites",
        })
        .px(px(6.))
        .py(px(4.))
        .text_size(px(12.))
        .cursor_pointer()
        .bg(if on {
            theme.selected_fill()
        } else {
            theme.background
        })
        .text_color(if on { theme.accent } else { theme.foreground })
        .hover(|style| style.bg(theme.hover_fill()))
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                this.browse.set_filter(filter);
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(filter.label());
    if divider {
        segment = segment.border_l_1().border_color(theme.border);
    }
    segment
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
    grid_scroll: &UniformListScrollHandle,
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
    let focused = browse.focus == Focus::Sidebar;
    div()
        .id("sidebar")
        .w(px(SIDEBAR_WIDTH))
        .h_full()
        .min_h_0()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .bg(theme.inset)
        .border_r_1()
        .border_color(if focused { theme.accent } else { theme.border })
        .child(sidebar_header(browse, cx))
        .child(
            split::list_metrics(
                div()
                    .id("sidebar-list")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(scroll),
            )
            .children(sidebar_consoles(browse, cx)),
        )
}

fn sidebar_header(browse: &Browse, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .px(px(16.))
        .pt(px(10.))
        .pb(px(8.))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w(px(0.))
                .text_size(px(12.))
                .text_color(theme.secondary)
                .child("Systems")
                .child(keycap("o", cx)),
        )
        .child(system_sort_control(browse, cx))
}

fn system_sort_control(browse: &Browse, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .id("system-sort")
        .flex()
        .flex_shrink_0()
        .border_1()
        .border_color(theme.border)
        .child(sort_segment(browse, SystemSort::Name, false, cx))
        .child(sort_segment(browse, SystemSort::Year, true, cx))
}

fn sort_segment(
    browse: &Browse,
    sort: SystemSort,
    divider: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let on = browse.library.system_sort == sort;
    let mut segment = div()
        .id(match sort {
            SystemSort::Name => "sort-name",
            SystemSort::Year => "sort-year",
        })
        .px(px(6.))
        .py(px(4.))
        .text_size(px(12.))
        .cursor_pointer()
        .bg(if on {
            theme.selected_fill()
        } else {
            theme.background
        })
        .text_color(if on { theme.accent } else { theme.foreground })
        .hover(|style| style.bg(theme.hover_fill()))
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                this.choose_system_sort(sort);
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(sort.label());
    if divider {
        segment = segment.border_l_1().border_color(theme.border);
    }
    segment
}

fn sidebar_consoles(browse: &Browse, cx: &Context<Shell>) -> Vec<impl IntoElement> {
    let theme = cx.omarchy();
    browse
        .library
        .shelves
        .iter()
        .enumerate()
        .map(|(index, shelf)| {
            let selected = index == browse.console;
            split::row_metrics(div().id(("console", index)))
                .bg(if selected {
                    theme.selected_fill()
                } else {
                    theme.background
                })
                .border_1()
                .border_color(if selected { theme.accent } else { theme.border })
                .hover(|style| style.bg(theme.hover_fill()))
                .on_click(
                    cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                        this.revealed_console = None;
                        this.browse.select_console(index);
                        this.focus_handle.focus(window, cx);
                        cx.notify();
                    }),
                )
                .child(split::title_metrics(div()).child(shelf.console.name.clone()))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(6.))
                        .min_h(px(split::DETAIL_LINE))
                        .child({
                            let mut year = split::detail_metrics(
                                div().flex_1().min_w(px(0.)).text_color(theme.secondary),
                            );
                            if let Some(launched) = shelf.year {
                                year = year.child(launched.to_string());
                            }
                            year
                        })
                        .child(
                            split::detail_metrics(
                                div()
                                    .flex_shrink_0()
                                    .text_color(theme.secondary)
                                    .whitespace_nowrap(),
                            )
                            .child(game_count_label(shelf.games.len())),
                        ),
                )
        })
        .collect()
}

fn grid(
    browse: &Browse,
    scroll: &UniformListScrollHandle,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let focused = browse.focus == Focus::Grid;
    let shelf = browse.shelf();
    let pane = div()
        .id("grid")
        .flex_1()
        .h_full()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .overflow_hidden()
        .p(px(GRID_PAD))
        .border_1()
        .border_color(if focused {
            theme.accent
        } else {
            theme.background
        });
    let Some(shelf) = shelf else {
        return pane.child(library_empty(cx));
    };
    let visible_count = browse.visible_len();
    if visible_count == 0 {
        if shelf.games.is_empty() {
            return pane.child(library_empty(cx));
        }
        if !browse.query().trim().is_empty() {
            return pane.child(empty_state(
                "No matches",
                "No titles match this filter. Esc clears it.",
                cx,
            ));
        }
        return pane.child(empty_state(
            "No favorites",
            "Show All, select a game, and press f.",
            cx,
        ));
    }
    let columns = browse.columns.max(1);
    let rows = visible_count.div_ceil(columns);
    let shell = cx.entity();
    pane.child(
        uniform_list("game-rows", rows, move |range, window, app| {
            let scale = window.scale_factor();
            shell.update(app, |this, cx| this.game_rows(range, scale, cx))
        })
        .flex_1()
        .w_full()
        .min_h(px(0.))
        .track_scroll(scroll),
    )
}

fn grid_thumb(game: &Game, art: GridArt) -> Option<(crate::covers::ThumbKind, PathBuf)> {
    art.fallback().into_iter().find_map(|kind| {
        let path = file_for(game, kind)?;
        let thumb = crate::covers::ThumbKind::from_media(kind)?;
        Some((thumb, path))
    })
}

fn tile(
    index: usize,
    game: &Game,
    console_name: &str,
    art: GridArt,
    cover_width: f32,
    scale: f32,
    selected: bool,
    menu: Option<usize>,
    demo: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let frame = TileFrame::for_art(art, cover_width);
    let title = game.display_title().to_string();
    let (slot_w, slot_h) = crate::covers::slot_px(frame.width, frame.height, scale);
    let thumb = grid_thumb(game, art);
    let rendered = thumb
        .as_ref()
        .and_then(|(kind, path)| crate::covers::lookup(path, *kind, slot_w, slot_h));
    if rendered.is_none() {
        if let Some((kind, path)) = &thumb {
            crate::covers::note_visible(path.clone(), *kind, slot_w, slot_h);
        }
    }
    // An explicit ratio stops GPUI from resizing the tile to the file's own ratio.
    let image = div()
        .w(px(frame.width))
        .h(px(frame.height))
        .flex_shrink_0()
        .overflow_hidden()
        .bg(theme.surface);
    let image = if let Some(rendered) = rendered {
        image.child(
            img(rendered)
                .w(px(frame.width))
                .h(px(frame.height))
                .aspect_ratio(frame.ratio())
                .object_fit(ObjectFit::Contain),
        )
    } else if thumb.is_some() {
        image
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
        .w(px(frame.width))
        .px(px(8.))
        .pt(px(8.))
        .pb(px(6.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .overflow_hidden()
        .child(
            div()
                .text_size(px(12.))
                .line_clamp(2)
                .min_h(px(32.))
                .child(title),
        );
    if demo {
        caption = caption.child(
            div()
                .text_size(px(12.))
                .text_color(theme.secondary)
                .child("Placeholder"),
        );
    }
    // The heart sits on this line, so a favorite keeps the name clear of it.
    let mut subtitle = div()
        .text_size(px(11.))
        .line_height(px(14.))
        .text_color(theme.secondary)
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis_middle()
        .child(console_name.to_string());
    if game.favorite {
        subtitle = subtitle.pr(px(22.));
    }
    caption = caption.child(subtitle);
    // The heart is a card overlay, in the title band, so the cover does not crop it.
    let mut card = div()
        .id(("game", index))
        .relative()
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
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this: &mut Shell, _: &MouseDownEvent, window, cx| {
                this.revealed_game = None;
                this.browse.select_game(index);
                this.browse.open_game_menu();
                this.focus_handle.focus(window, cx);
                cx.notify();
                cx.stop_propagation();
            }),
        )
        .child(image)
        .child(caption);
    if game.favorite {
        card = card.child(favorite_badge());
    }
    if let Some(cursor) = menu {
        card = card.child(game_menu(cursor, frame.height, cx));
    }
    card
}

fn game_menu(cursor: usize, art_height: f32, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut rows = div()
        .id("game-menu")
        .w(px(200.))
        .flex()
        .flex_col()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .occlude()
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down_out(cx.listener(|this: &mut Shell, _: &MouseDownEvent, _, cx| {
            if this.browse.menu_open() {
                this.dismiss_overlay();
                cx.notify();
            }
        }));
    for (index, action) in crate::types::GameAction::ALL.into_iter().enumerate() {
        let chosen = index == cursor;
        let mut row = div()
            .id(("game-menu-item", index))
            .w_full()
            .px(px(12.))
            .py(px(8.))
            .cursor_pointer()
            .on_click(
                cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                    this.browse.aim(Aim::Menu(index));
                    this.confirm_overlay();
                    this.focus_handle.focus(window, cx);
                    cx.notify();
                    cx.stop_propagation();
                }),
            );
        if chosen {
            row = row.bg(theme.selected_fill()).text_color(theme.accent);
        } else {
            row = row.hover(|style| style.bg(theme.hover_fill()));
        }
        rows = rows.child(row.child(action.label()));
    }
    div()
        .absolute()
        .top(px(art_height))
        .left(px(0.))
        .child(deferred(
            anchored()
                .snap_to_window_with_margin(Edges::all(px(8.)))
                .child(rows),
        ))
}

fn favorite_badge() -> impl IntoElement {
    div()
        .absolute()
        .bottom(px(6.))
        .right(px(6.))
        .font_family("DejaVu Sans")
        .font_weight(gpui_kit::FontWeight::BOLD)
        .text_size(px(18.))
        .text_color(rgb(FAVORITE_RED))
        .child("♥")
}

fn favorite_toggle(favorite: bool, cx: &Context<Shell>) -> impl IntoElement {
    let label = if favorite { "♥" } else { "♡" };
    let mut button = button("favorite", label, ButtonVariant::Secondary, cx)
        .flex_none()
        .font_family("DejaVu Sans")
        .text_size(px(18.))
        .on_click(cx.listener(|this: &mut Shell, _: &ClickEvent, window, cx| {
            this.toggle_favorite();
            this.focus_handle.focus(window, cx);
            cx.notify();
        }));
    if favorite {
        button = button.text_color(rgb(FAVORITE_RED));
    }
    button
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
        .w(px(DETAILS_WIDTH))
        .h_full()
        .flex_shrink_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .justify_start()
        .gap(px(8.))
        .p(px(DETAILS_PAD))
        .bg(theme.surface)
        .border_l_1()
        .border_color(theme.border);
    let Some(shelf) = browse.shelf() else {
        return pane;
    };
    if let Some(game) = browse.selected_game() {
        let favorite = game.favorite;
        pane = pane.child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .child(
                    button("play", "Play", ButtonVariant::Primary, cx)
                        .flex_1()
                        .on_click(cx.listener(|this: &mut Shell, _: &ClickEvent, window, cx| {
                            this.launch_selected();
                            this.focus_handle.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(favorite_toggle(favorite, cx)),
        );
        if let Some(path) = file_for(game, MediaKind::BoxArt) {
            pane = pane.child(detail_art(path, MediaKind::BoxArt));
        }
        if let Some(path) = file_for(game, MediaKind::Screenshot) {
            pane = pane.child(detail_art(path, MediaKind::Screenshot));
        }
        pane = pane
            .child(heading(game.display_title()))
            .child(meta(format!("Console: {}", shelf.console.name), cx));
        if let Some(metadata) = &game.metadata {
            if let Some(publisher) = &metadata.publisher {
                pane = pane.child(meta(format!("Publisher: {publisher}"), cx));
            }
            if let Some(year) = metadata.year {
                pane = pane.child(meta(format!("Year: {year}"), cx));
            }
            if let Some(genre) = &metadata.genre {
                pane = pane.child(meta(format!("Genre: {genre}"), cx));
            }
        }
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

fn detail_art(path: std::path::PathBuf, kind: MediaKind) -> impl IntoElement {
    let ratio = image_aspect(&path).unwrap_or(match kind {
        MediaKind::Screenshot => 4.0 / 3.0,
        _ => 3.0 / 4.0,
    });
    // Border box includes padding and the 1px left border. Height is capped at
    // 180 so a box plus a screenshot do not push play time below the pane.
    let width = (DETAILS_WIDTH - DETAILS_PAD * 2.0 - 1.0).max(1.0);
    let height = (width / ratio.max(0.05)).clamp(1.0, 180.0);
    img(path)
        .w_full()
        .h(px(height))
        .flex_none()
        .aspect_ratio(width / height)
        .object_fit(ObjectFit::Contain)
}

fn heading(text: &str) -> impl IntoElement {
    div()
        .flex_none()
        .font_weight(gpui_kit::FontWeight::BOLD)
        .text_size(px(16.))
        .line_height(px(24.))
        .child(text.to_string())
}

fn meta(text: String, cx: &App) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(12.))
        .text_color(cx.omarchy().secondary)
        .child(text)
}

fn status_line(
    browse: &Browse,
    cover_slider: &Entity<SliderState>,
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let text = if browse.status.is_empty() {
        "Ctrl+E emulators. Ctrl+P systems. Ctrl+O options."
    } else {
        browse.status.as_str()
    };
    div()
        .h(px(40.))
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .bg(theme.inset)
        .border_t_1()
        .border_color(theme.border)
        .text_size(px(12.))
        .text_color(theme.secondary)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(text.to_string()),
        )
        .child(vertical_separator(cx))
        .child(grid_art_control(browse, cx))
        .child(vertical_separator(cx))
        .child(cover_width_control(browse, cover_slider, window, cx))
}

fn grid_art_control(browse: &Browse, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let current = browse
        .shelf()
        .map(|shelf| shelf.console.grid_art)
        .unwrap_or_default();
    let mut segments = div()
        .id("grid-art")
        .flex()
        .flex_shrink_0()
        .border_1()
        .border_color(theme.border);
    for (index, art) in GridArt::ALL.into_iter().enumerate() {
        segments = segments.child(grid_art_segment(art, index, art == current, index > 0, cx));
    }
    with_tooltip(
        div()
            .id("grid-art-control")
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(8.))
            .child("Art")
            .child(segments),
        grid_art_tip(),
    )
}

fn grid_art_tip() -> String {
    let keys = GridArt::ALL
        .iter()
        .enumerate()
        .map(|(index, art)| format!("{} {}", index + 1, art.label()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Grid artwork for this system ({keys})")
}

fn grid_art_segment(
    art: GridArt,
    index: usize,
    on: bool,
    divider: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut segment = div()
        .id(("grid-art", index))
        .px(px(8.))
        .py(px(4.))
        .text_size(px(12.))
        .cursor_pointer()
        .bg(if on {
            theme.selected_fill()
        } else {
            theme.background
        })
        .text_color(if on { theme.accent } else { theme.foreground })
        .hover(|style| style.bg(theme.hover_fill()))
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if this.browse.set_grid_art(art) {
                    this.persist_grid_art();
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(format!("{} {}", index + 1, art.label()));
    if divider {
        segment = segment.border_l_1().border_color(theme.border);
    }
    segment
}

fn cover_width_control(
    browse: &Browse,
    cover_slider: &Entity<SliderState>,
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> impl IntoElement {
    div()
        .id("cover-width")
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(8.))
        .child("Size")
        .child(
            div()
                .w(px(160.))
                .flex_shrink_0()
                .child(slider(cover_slider, false, window, cx)),
        )
        .child(
            div()
                .w(px(48.))
                .flex_shrink_0()
                .child(format!("{:.0}px", browse.cover_width)),
        )
}

fn game_dialog(
    browse: &Browse,
    scrape_scroll: &ScrollHandle,
    cx: &Context<Shell>,
) -> Option<gpui_kit::AnyElement> {
    let page = match &browse.overlay {
        Overlay::Rename(rename) => rename_dialog(rename, cx).into_any_element(),
        Overlay::Delete(prompt) => delete_dialog(prompt, cx).into_any_element(),
        Overlay::Scrape(prompt) => scrape_dialog(prompt, scrape_scroll, cx).into_any_element(),
        Overlay::None | Overlay::Menu(_) => return None,
    };
    Some(modal(page).into_any_element())
}

fn modal(child: impl IntoElement) -> gpui_kit::Div {
    div()
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui_kit::Hsla::from(rgb(0x000000)).opacity(0.45))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
        })
        .child(child)
}

fn dialog_page(
    id: &'static str,
    title: &str,
    width: f32,
    cx: &Context<Shell>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let theme = cx.omarchy();
    div()
        .id(id)
        .w(px(width))
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(16.))
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .occlude()
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
        })
        .child(
            div()
                .flex_none()
                .font_weight(gpui_kit::FontWeight::BOLD)
                .text_size(px(16.))
                .line_height(px(24.))
                .child(title.to_string()),
        )
}

fn hint(text: &str, cx: &Context<Shell>) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(12.))
        .line_height(px(18.))
        .text_color(cx.omarchy().secondary)
        .whitespace_normal()
        .child(text.to_string())
}

fn line_editor(
    id: &'static str,
    edit: &crate::game_menu::LineEdit,
    focused: bool,
    aim: Aim,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let line = edit.caret_line();
    let mut text = div()
        .flex()
        .flex_row()
        .items_center()
        .overflow_hidden()
        .min_h(px(18.));
    if line.selected {
        text = text.child(
            div()
                .bg(theme.selected_fill())
                .text_color(theme.accent)
                .child(line.head),
        );
    } else {
        text = text.child(line.head);
        if focused {
            text = text.child(
                div()
                    .w(px(1.))
                    .h(px(16.))
                    .flex_shrink_0()
                    .bg(theme.foreground),
            );
        }
        text = text.child(line.tail);
    }
    div()
        .id(id)
        .flex_1()
        .min_w(px(0.))
        .px(px(8.))
        .py(px(6.))
        .border_1()
        .border_color(if focused { theme.accent } else { theme.border })
        .bg(theme.surface)
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                this.browse.aim(aim);
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(text)
}

fn aimed_button(
    id: &'static str,
    label: &'static str,
    variant: ButtonVariant,
    aimed: bool,
    aim: Aim,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .border_1()
        .border_color(if aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(button(id, label, variant, cx).on_click(cx.listener(
            move |this: &mut Shell, _: &ClickEvent, window, cx| {
                this.browse.aim(aim);
                this.confirm_overlay();
                this.focus_handle.focus(window, cx);
                cx.notify();
            },
        )))
}

fn rename_dialog(rename: &crate::game_menu::Rename, cx: &Context<Shell>) -> impl IntoElement {
    dialog_page("rename-dialog", "Rename", 420., cx)
        .child(hint(
            "Library title. Leave it blank to show the scraped or file name. This does not rename the ROM file.",
            cx,
        ))
        .child(line_editor(
            "rename-title",
            &rename.edit,
            rename.slot == RenameSlot::Title,
            Aim::Rename(RenameSlot::Title),
            cx,
        ))
        .child(
        div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .child(aimed_button(
                "rename-cancel",
                "Cancel",
                ButtonVariant::Secondary,
                rename.slot == RenameSlot::Cancel,
                Aim::Rename(RenameSlot::Cancel),
                cx,
            ))
            .child(aimed_button(
                "rename-save",
                "Save",
                ButtonVariant::Primary,
                rename.slot == RenameSlot::Save,
                Aim::Rename(RenameSlot::Save),
                cx,
            )),
    )
}

fn delete_dialog(prompt: &crate::game_menu::DeletePrompt, cx: &Context<Shell>) -> impl IntoElement {
    dialog_page("delete-dialog", "Delete", 440., cx)
        .child(
            div()
                .font_weight(gpui_kit::FontWeight::BOLD)
                .text_size(px(18.))
                .whitespace_normal()
                .child(prompt.title.clone()),
        )
        .child(hint(
            "Remove this game from the library. The ROM and scraped artwork stay on disk unless you check a box.",
            cx,
        ))
        .child(delete_check(
            false,
            prompt.options.rom_file,
            prompt.slot == DeleteSlot::Rom,
            cx,
        ))
        .child(delete_check(
            true,
            prompt.options.scraped_assets,
            prompt.slot == DeleteSlot::Assets,
            cx,
        ))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(aimed_button(
                    "delete-cancel",
                    "Cancel",
                    ButtonVariant::Primary,
                    prompt.slot == DeleteSlot::Cancel,
                    Aim::Delete(DeleteSlot::Cancel),
                    cx,
                ))
                .child(aimed_button(
                    "delete-confirm",
                    "Delete",
                    ButtonVariant::Danger,
                    prompt.slot == DeleteSlot::Confirm,
                    Aim::Delete(DeleteSlot::Confirm),
                    cx,
                )),
        )
}

fn delete_check(assets: bool, on: bool, aimed: bool, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let (id, label) = if assets {
        ("delete-assets", "Delete scraped assets")
    } else {
        ("delete-rom", "Delete ROM file from disk")
    };
    let state = if on {
        CheckboxState::Checked
    } else {
        CheckboxState::Unchecked
    };
    let entity = cx.entity();
    div()
        .border_1()
        .border_color(if aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(
            checkbox(id, label, state, cx).on_change(move |state, _, _, cx| {
                entity.update(cx, |this, cx| {
                    this.browse
                        .set_delete_flag(assets, state == CheckboxState::Checked);
                    cx.notify();
                });
            }),
        )
}

fn scrape_dialog(
    prompt: &crate::game_menu::ScrapePrompt,
    scroll: &ScrollHandle,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut page = dialog_page("scrape-dialog", "Scrape", 480., cx)
        .child(hint(
            "Search by name, then pick a match. Box art and screenshot for this game are replaced.",
            cx,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(line_editor(
                    "scrape-query",
                    &prompt.edit,
                    prompt.slot == ScrapeSlot::Query,
                    Aim::ScrapeQuery,
                    cx,
                ))
                .child(aimed_button(
                    "scrape-search",
                    "Search",
                    ButtonVariant::Primary,
                    prompt.slot == ScrapeSlot::Search,
                    Aim::ScrapeSearch,
                    cx,
                )),
        );
    if !prompt.message.is_empty() {
        page = page.child(hint(&prompt.message, cx));
    }
    let mut list = div()
        .id("scrape-results")
        .w_full()
        .max_h(px(240.))
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col();
    for (index, candidate) in prompt.candidates.iter().enumerate() {
        let chosen = prompt.slot == ScrapeSlot::Results && prompt.cursor == index;
        let subtitle = if candidate.system.is_empty() {
            candidate.provider.label().to_string()
        } else {
            format!("{} · {}", candidate.provider.label(), candidate.system)
        };
        let mut row = div()
            .id(("scrape-result", index))
            .w_full()
            .px(px(8.))
            .py(px(6.))
            .cursor_pointer()
            .on_click(
                cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                    this.browse.aim(Aim::ScrapeResult(index));
                    this.confirm_overlay();
                    this.focus_handle.focus(window, cx);
                    cx.notify();
                }),
            );
        if chosen {
            row = row.bg(theme.selected_fill()).text_color(theme.accent);
        } else {
            row = row.hover(|style| style.bg(theme.hover_fill()));
        }
        list = list.child(
            row.child(candidate.title.clone()).child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.secondary)
                    .child(subtitle),
            ),
        );
    }
    page.child(list)
}

fn delete_notes(game: &Game, options: DeleteOptions) -> Vec<String> {
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
    notes
}

fn rescan_console(
    console: crate::types::Console,
) -> Result<(String, String, Vec<Game>, database::LibraryStats), String> {
    let conn = database::init_db().map_err(|err| err.to_string())?;
    let games = scanner::rescan(&console, &conn).map_err(|err| err.to_string())?;
    let stats = crate::browse::stats_of(&games);
    Ok((console.id, console.name, games, stats))
}

fn save_play_session(game_id: &str, console_id: &str, elapsed: u32) -> Result<PlayNote, String> {
    let conn = database::init_db().map_err(|err| format!("Could not save play time ({err})."))?;
    let id = game_id.to_string();
    database::increment_play_stats(&conn, &id, elapsed)
        .map_err(|err| format!("Could not save play time ({err})."))?;
    database::update_last_played(&conn, &id)
        .map_err(|err| format!("Could not save play time ({err})."))?;
    let console = console_id.to_string();
    let game = database::load_games(&conn, Some(&console))
        .map_err(|err| format!("Could not save play time ({err})."))?
        .into_iter()
        .find(|game| game.id == id)
        .ok_or_else(|| "Could not save play time.".to_string())?;
    let last_played = game
        .last_played
        .ok_or_else(|| "Could not save play time.".to_string())?;
    Ok(PlayNote::Saved {
        game_id: id,
        play_count: game.play_count,
        play_time: game.play_time,
        last_played,
    })
}

fn typed_text(keystroke: &Keystroke) -> Option<&str> {
    if keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform {
        return None;
    }
    if keystroke.key == "space" {
        return Some(" ");
    }
    let text = keystroke.key_char.as_deref()?;
    if text.is_empty() || text.chars().any(|ch| ch.is_control()) {
        return None;
    }
    Some(text)
}

fn load_input() -> InputSettings {
    config::load_config()
        .map(|config| config.input)
        .unwrap_or_default()
        .sanitized()
}

fn load_appearance() -> String {
    config::load_config()
        .map(|config| config.theme)
        .unwrap_or_else(|_| appearance::SYSTEM.to_string())
}

fn monotonic_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

enum ArrowKey {
    Absent,
    Handled,
    Propagate,
}

fn list_jump(key: &str) -> Option<Jump> {
    match key {
        "home" => Some(Jump::Home),
        "end" => Some(Jump::End),
        "pageup" => Some(Jump::PageUp),
        "pagedown" => Some(Jump::PageDown),
        _ => None,
    }
}

fn arrow_dir(keystroke: &Keystroke) -> Option<NavDir> {
    match key_from_parts(keystroke.key.as_ref(), keystroke.key_char.as_deref(), false) {
        Some(Key::Arrow(dir)) => Some(dir),
        _ => None,
    }
}

enum Picked {
    Path(PathBuf),
    Cancel,
    Failed(String),
}

fn library_empty(cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(16.))
        .px(px(24.))
        .child(
            div()
                .font_weight(gpui_kit::FontWeight::BOLD)
                .text_size(px(28.))
                .text_center()
                .child("No games found"),
        )
        .child(
            div()
                .max_w(px(460.))
                .text_center()
                .text_color(theme.secondary)
                .whitespace_normal()
                .child(
                    "Add a system in Manage Systems, set its ROM folders, and rescan. Only paths are stored.",
                ),
        )
        .child(
            button(
                "empty-systems",
                "Manage Systems",
                ButtonVariant::Primary,
                cx,
            )
            .on_click(cx.listener(|this: &mut Shell, _: &ClickEvent, window, cx| {
                this.activate_screen(Screen::Systems);
                this.focus_handle.focus(window, cx);
                cx.notify();
            })),
        )
        .child(
            button(
                "empty-emulators",
                "Manage Emulators",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(|this: &mut Shell, _: &ClickEvent, window, cx| {
                this.activate_screen(Screen::Emulators);
                this.focus_handle.focus(window, cx);
                cx.notify();
            })),
        )
}

fn import_error(text: &str, cx: &Context<Shell>) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .text_color(cx.omarchy().danger)
        .child(text.to_string())
}

fn emulator_screen(
    dialog: Option<&Emulators>,
    scroll: &ScrollHandle,
    cx: &Context<Shell>,
) -> Option<gpui_kit::AnyElement> {
    let dialog = dialog?;
    let rows = dialog
        .list_rows()
        .into_iter()
        .map(|row| emulator_list_row(row, cx).into_any_element())
        .collect::<Vec<_>>();
    let panel = emulator_panel(&dialog.panel(), cx);
    Some(
        split::screen(
            "emulator-list",
            "Manage Emulators",
            dialog.list_focused(),
            scroll,
            rows,
            !dialog.list_focused(),
            panel,
            cx,
        )
        .into_any_element(),
    )
}

fn emulator_list_row(row: crate::emulators::ListRow, cx: &Context<Shell>) -> impl IntoElement {
    let index = row.index;
    split_list_row(("emulator", index), row.title, row.detail, row.selected, cx).on_click(
        cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
            if let Some(dialog) = &mut this.emulators {
                dialog.select(index);
            }
            this.focus_handle.focus(window, cx);
            cx.notify();
        }),
    )
}

fn emulator_panel(panel: &crate::emulators::Panel, cx: &Context<Shell>) -> impl IntoElement {
    let title = if panel.adding {
        "New emulator".to_string()
    } else {
        let name = panel.name.text.trim();
        if name.is_empty() {
            "Emulator".to_string()
        } else {
            name.to_string()
        }
    };
    let mut page = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(heading(&title))
        .child(field_label("Name", cx))
        .child(emulator_editor(
            "emulator-name",
            &panel.name,
            panel.name_aimed,
            "Name",
            EmulatorField::Name,
            cx,
        ))
        .child(field_label("Kind", cx))
        .child(kind_block(panel.kind, panel.kind_aimed, cx))
        .child(field_label("Path", cx))
        .child(emulator_editor(
            "emulator-path",
            &panel.path,
            panel.path_aimed,
            "Executable path",
            EmulatorField::Path,
            cx,
        ))
        .child(field_label("Global arguments", cx))
        .child(emulator_editor(
            "emulator-global-args",
            &panel.args,
            panel.args_aimed,
            "Global arguments",
            EmulatorField::Args,
            cx,
        ));
    if panel.missing {
        page = page.child(import_error("Not found.", cx));
    }
    if panel.show_cores {
        page = page
            .child(heading("Cores"))
            .child(field_label("Config", cx))
            .child(emulator_editor(
                "emulator-config",
                &panel.cfg,
                panel.cfg_aimed,
                "retroarch.cfg",
                EmulatorField::Config,
                cx,
            ))
            .child(hint(&format!("Cores {}", panel.cores_dir), cx));
        if let Some(note) = &panel.cores_note {
            page = page.child(hint(note, cx));
        } else if panel.cores.is_empty() {
            page = page.child(hint("No cores in that directory.", cx));
        }
        page = page
            .child(core_list(&panel.cores, cx))
            .child(emulator_press(
                "emulator-rescan".to_string(),
                "Rescan cores".to_string(),
                ButtonVariant::Secondary,
                panel.rescan_aimed,
                EmulatorField::Rescan,
                cx,
            ));
    }
    let mut actions = div().flex().justify_end().gap(px(8.));
    if panel.show_delete {
        actions = actions.child(emulator_press(
            "emulator-delete".to_string(),
            "Delete".to_string(),
            ButtonVariant::Danger,
            panel.delete_aimed,
            EmulatorField::Delete,
            cx,
        ));
    }
    actions = actions.child(emulator_press(
        "emulator-save".to_string(),
        panel.save_label.to_string(),
        ButtonVariant::Primary,
        panel.save_aimed,
        EmulatorField::Save,
        cx,
    ));
    page = page.child(actions);
    if let Some(error) = &panel.error {
        page = page.child(import_error(error, cx));
    }
    page
}

fn core_list(cores: &[crate::emulators::CoreLine], cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    if cores.is_empty() {
        return div().into_any_element();
    }
    let mut list = div().flex().flex_col().gap(px(4.));
    for (index, core) in cores.iter().enumerate() {
        list = list.child(
            div()
                .id(("emulator-core", index))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .px(px(8.))
                .py(px(6.))
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(core.label.clone())
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(theme.secondary)
                        .child(core.file_name.clone()),
                ),
        );
    }
    list.into_any_element()
}

fn systems_screen(
    dialog: Option<&Systems>,
    scroll: &ScrollHandle,
    picker_scroll: &ScrollHandle,
    scraping: bool,
    cx: &Context<Shell>,
) -> Option<gpui_kit::AnyElement> {
    let dialog = dialog?;
    let panel_view = dialog.panel();
    let confirm = panel_view.confirm.clone();
    let rows = dialog
        .list_rows()
        .into_iter()
        .map(|row| system_list_row(row, cx).into_any_element())
        .collect::<Vec<_>>();
    let panel = systems_panel(&panel_view, picker_scroll, scraping, cx);
    let screen = split::screen(
        "systems-list",
        "Manage Systems",
        dialog.list_focused(),
        scroll,
        rows,
        !dialog.list_focused(),
        panel,
        cx,
    );
    // The delete prompt covers this screen only. The shell menu and status bar stay put.
    let mut layer = div()
        .flex_1()
        .min_h_0()
        .w_full()
        .relative()
        .flex()
        .flex_col()
        .child(screen);
    if let Some(confirm) = confirm {
        layer = layer.child(system_delete_dialog(&confirm, cx));
    }
    Some(layer.into_any_element())
}

fn system_list_row(row: crate::systems::ListRow, cx: &Context<Shell>) -> impl IntoElement {
    let index = row.index;
    split_list_row(("system", index), row.title, row.detail, row.selected, cx).on_click(
        cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
            let step = this
                .systems
                .as_mut()
                .map(|dialog| dialog.click_row(index))
                .unwrap_or(SystemStep::Stay);
            this.apply_system_step(step, cx);
            this.focus_handle.focus(window, cx);
            cx.notify();
        }),
    )
}

fn systems_panel(
    panel: &crate::systems::Panel,
    scroll: &ScrollHandle,
    scraping: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    if panel.adding {
        if let Some(menu) = &panel.type_menu {
            return div()
                .w_full()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .gap(px(12.))
                .child(heading("Add system"))
                .child(hint(
                    "Type to jump by id or name. Enter adds it. Types already in the list are hidden.",
                    cx,
                ))
                .child(picker::menu(
                    menu,
                    scroll,
                    420.,
                    cx,
                    |this, index, window, cx| {
                        this.pick_system(index, cx);
                        this.focus_handle.focus(window, cx);
                        cx.notify();
                    },
                ))
                .into_any_element();
        }
        let text = if panel.types_available {
            "Right or Enter to choose a system type."
        } else {
            "Every system type is already in the list."
        };
        return hint(text, cx).into_any_element();
    }
    if panel.empty {
        return hint("No systems in this library.", cx).into_any_element();
    }
    let mut page = div()
        .w_full()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(px(12.))
        .child(heading(&panel.name))
        .child(field_label("Emulator", cx))
        .child(system_choice(
            "system-emulator".to_string(),
            format!("Emulator: {}", panel.emulator),
            panel.emulator_aimed,
            SystemField::Emulator,
            panel.emulator_menu.as_ref(),
            scroll,
            cx,
        ));
    if panel.core_enabled {
        page = page.child(field_label("Core", cx)).child(system_choice(
            "system-core".to_string(),
            format!("Core: {}", panel.core),
            panel.core_aimed,
            SystemField::Core,
            panel.core_menu.as_ref(),
            scroll,
            cx,
        ));
        if panel.core_missing {
            page = page.child(import_error(
                "That core is not in the active RetroArch directory.",
                cx,
            ));
        }
    } else {
        page = page.child(
            div()
                .text_color(cx.omarchy().secondary)
                .child("Core: No core"),
        );
    }
    page = page
        .child(field_label("Extra arguments", cx))
        .child(system_editor(
            "system-args",
            &panel.args,
            panel.args_aimed,
            "Extra arguments",
            SystemField::Args,
            cx,
        ))
        .child(field_label("ROM folders", cx))
        .child(hint(
            "Each folder is scanned for this system's extensions. Files stay where they are.",
            cx,
        ));
    for path in &panel.paths {
        page = page.child(system_path_row(path, cx));
    }
    page = page
        .child(system_editor(
            "system-add-path",
            &panel.path_draft,
            panel.path_draft_aimed,
            "Type a folder path",
            SystemField::AddPath,
            cx,
        ))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(system_action(
                    "system-add-folder".to_string(),
                    "Add path".to_string(),
                    ButtonVariant::Secondary,
                    panel.path_draft_aimed,
                    SystemField::AddPath,
                    true,
                    cx,
                ))
                .child(system_action(
                    "system-browse".to_string(),
                    "Browse…".to_string(),
                    ButtonVariant::Secondary,
                    panel.browse_aimed,
                    SystemField::Browse,
                    true,
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .child(system_action(
                    "system-rescan".to_string(),
                    "Rescan".to_string(),
                    ButtonVariant::Primary,
                    panel.rescan_aimed,
                    SystemField::Rescan,
                    true,
                    cx,
                ))
                .child(system_action(
                    "system-scrape".to_string(),
                    if scraping {
                        "Scraping…".to_string()
                    } else {
                        "Scrape Missing".to_string()
                    },
                    ButtonVariant::Secondary,
                    panel.scrape_aimed,
                    SystemField::ScrapeMissing,
                    !scraping,
                    cx,
                ))
                .child(system_action(
                    "system-delete".to_string(),
                    "Delete system".to_string(),
                    ButtonVariant::Danger,
                    panel.delete_aimed,
                    SystemField::Delete,
                    true,
                    cx,
                )),
        );
    page.into_any_element()
}

fn system_path_row(path: &crate::systems::PathLine, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let index = path.index;
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .px(px(8.))
                .py(px(6.))
                .border_1()
                .border_color(if path.aimed {
                    theme.accent
                } else {
                    theme.border
                })
                .bg(theme.surface)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(path.path.clone()),
        )
        .child(system_action(
            format!("system-path-{index}"),
            "Remove".to_string(),
            ButtonVariant::Secondary,
            path.aimed,
            SystemField::Path(index),
            true,
            cx,
        ))
}

fn system_delete_dialog(prompt: &crate::systems::Confirm, cx: &Context<Shell>) -> impl IntoElement {
    let name = prompt.name.clone();
    modal(
        dialog_page("system-delete-dialog", "Delete system", 460., cx)
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .text_size(px(18.))
                    .whitespace_normal()
                    .child(name.clone()),
            )
            .child(hint(
                "Remove this system from the library. ROM files stay on disk. Games, favorites, play stats, and scraped metadata for this system are dropped.",
                cx,
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(system_confirm_button(
                        "system-delete-cancel",
                        "Cancel",
                        ButtonVariant::Primary,
                        prompt.slot == crate::systems::ConfirmSlot::Cancel,
                        crate::systems::ConfirmSlot::Cancel,
                        cx,
                    ))
                    .child(system_confirm_button(
                        "system-delete-confirm",
                        "Delete",
                        ButtonVariant::Danger,
                        prompt.slot == crate::systems::ConfirmSlot::Delete,
                        crate::systems::ConfirmSlot::Delete,
                        cx,
                    )),
            ),
    )
}

fn system_confirm_button(
    id: &'static str,
    label: &'static str,
    variant: ButtonVariant,
    aimed: bool,
    slot: crate::systems::ConfirmSlot,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .border_1()
        .border_color(if aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(button(id, label, variant, cx).on_click(cx.listener(
            move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(dialog) = &mut this.systems {
                    dialog.aim_confirm(slot);
                }
                this.confirm_systems(cx);
                this.focus_handle.focus(window, cx);
                cx.notify();
            },
        )))
}

fn options_screen(
    dialog: Option<&Options>,
    scroll: &ScrollHandle,
    cx: &Context<Shell>,
) -> Option<gpui_kit::AnyElement> {
    let dialog = dialog?;
    let rows = dialog
        .sections()
        .into_iter()
        .map(|row| options_list_row(row, cx).into_any_element())
        .collect::<Vec<_>>();
    let panel = options_panel(dialog, cx);
    Some(
        split::screen(
            "options-list",
            "Options",
            dialog.list_focused(),
            scroll,
            rows,
            !dialog.list_focused(),
            panel,
            cx,
        )
        .into_any_element(),
    )
}

fn options_list_row(row: crate::options::SectionRow, cx: &Context<Shell>) -> impl IntoElement {
    let section = row.section;
    split_list_row(
        options_section_id(section),
        row.title.to_string(),
        row.detail.to_string(),
        row.selected,
        cx,
    )
    .on_click(
        cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
            if let Some(dialog) = &mut this.options {
                dialog.select(section);
            }
            this.focus_handle.focus(window, cx);
            cx.notify();
        }),
    )
}

fn options_section_id(section: OptionsSection) -> &'static str {
    match section {
        OptionsSection::Input => "options-section-input",
        OptionsSection::Theme => "options-section-theme",
        OptionsSection::Scraper => "options-section-scraper",
        OptionsSection::Grid => "options-section-grid",
    }
}

fn options_panel(dialog: &Options, cx: &Context<Shell>) -> impl IntoElement {
    match dialog.section() {
        OptionsSection::Input => input_panel(dialog, cx).into_any_element(),
        OptionsSection::Theme => theme_panel(dialog, cx).into_any_element(),
        OptionsSection::Scraper => scraper_panel(dialog, cx).into_any_element(),
        OptionsSection::Grid => grid_options_panel(dialog, cx).into_any_element(),
    }
}

fn input_panel(dialog: &Options, cx: &Context<Shell>) -> impl IntoElement {
    let mut page = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(heading(options::SECTION))
        .child(hint(options::INTRO, cx));
    for row in dialog.input_rows() {
        page = page.child(input_row(row, cx));
    }
    page.child(div().flex().justify_end().child(options_close(dialog, cx)))
}

fn theme_panel(dialog: &Options, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(heading("Theme"))
        .child(hint(
            "System follows Omarchy. LaunchBox is the built-in palette.",
            cx,
        ))
        .child(
            div()
                .id("options-theme")
                .flex()
                .border_1()
                .border_color(if dialog.theme_aimed() {
                    theme.accent
                } else {
                    theme.border
                })
                .child(theme_segment(false, !dialog.theme_launchbox(), cx))
                .child(theme_segment(true, dialog.theme_launchbox(), cx)),
        )
}

fn theme_segment(launchbox: bool, on: bool, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .id(if launchbox {
            "options-theme-launchbox"
        } else {
            "options-theme-system"
        })
        .flex_1()
        .px(px(10.))
        .py(px(6.))
        .cursor_pointer()
        .bg(if on {
            theme.selected_fill()
        } else {
            theme.background
        })
        .text_color(if on { theme.accent } else { theme.foreground })
        .hover(|style| style.bg(theme.hover_fill()))
        .border_l_1()
        .border_color(if launchbox {
            theme.border
        } else {
            theme.background
        })
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                let changed = this
                    .options
                    .as_mut()
                    .is_some_and(|dialog| dialog.choose_theme(launchbox));
                if changed {
                    this.commit_theme_from_options(cx);
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(if launchbox { "LaunchBox" } else { "System" })
}

fn scraper_panel(dialog: &Options, cx: &Context<Shell>) -> impl IntoElement {
    let mut list = div().w_full().flex().flex_col().gap(px(12.));
    for block in scraper_panel_blocks(dialog) {
        list = list.child(scraper_block(block, cx));
    }
    list
}

fn scraper_panel_blocks(dialog: &Options) -> Vec<ScraperBlock> {
    let mut blocks = dialog.scraper().blocks();
    if dialog.list_focused() {
        for block in &mut blocks {
            match block {
                ScraperBlock::Art(line) => line.aimed = false,
                ScraperBlock::Provider(line) => {
                    line.check_aimed = false;
                    line.up_aimed = false;
                    line.down_aimed = false;
                }
                ScraperBlock::Field(line) => line.aimed = false,
                ScraperBlock::Save { aimed } => *aimed = false,
                ScraperBlock::Heading(_) | ScraperBlock::Note(_) | ScraperBlock::Error(_) => {}
            }
        }
    }
    blocks
}

fn grid_options_panel(dialog: &Options, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(heading("Grid"))
        .child(hint(
            "Cover width is the tile size. System order is the sidebar.",
            cx,
        ))
        .child(field_label("Cover width", cx))
        .child(
            div()
                .id("options-cover")
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(6.))
                .border_1()
                .border_color(if dialog.cover_aimed() {
                    theme.accent
                } else {
                    theme.border
                })
                .bg(theme.surface)
                .child(cover_step(-1, cx))
                .child(
                    div()
                        .flex_1()
                        .child(format!("{:.0}px", dialog.cover_width())),
                )
                .child(cover_step(1, cx)),
        )
        .child(field_label("System order", cx))
        .child(
            div()
                .id("options-sort")
                .flex()
                .border_1()
                .border_color(if dialog.sort_aimed() {
                    theme.accent
                } else {
                    theme.border
                })
                .child(options_sort_segment(dialog, SystemSort::Name, false, cx))
                .child(options_sort_segment(dialog, SystemSort::Year, true, cx)),
        )
}

fn cover_step(steps: i32, cx: &Context<Shell>) -> impl IntoElement {
    let label = if steps < 0 { "−" } else { "+" };
    let id = if steps < 0 {
        "options-cover-dec"
    } else {
        "options-cover-inc"
    };
    button(id, label, ButtonVariant::Secondary, cx).on_click(cx.listener(
        move |this: &mut Shell, _: &ClickEvent, window, cx| {
            let changed = this
                .options
                .as_mut()
                .is_some_and(|dialog| dialog.adjust_cover(steps));
            if changed {
                if let Some(width) = this.options.as_ref().map(|dialog| dialog.cover_width()) {
                    this.browse.cover_width = width;
                    this.persist_cover_width();
                }
            }
            this.focus_handle.focus(window, cx);
            cx.notify();
        },
    ))
}

fn options_sort_segment(
    dialog: &Options,
    sort: SystemSort,
    divider: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let on = dialog.system_sort() == sort;
    let mut segment = div()
        .id(match sort {
            SystemSort::Name => "options-sort-name",
            SystemSort::Year => "options-sort-year",
        })
        .flex_1()
        .px(px(10.))
        .py(px(6.))
        .cursor_pointer()
        .bg(if on {
            theme.selected_fill()
        } else {
            theme.background
        })
        .text_color(if on { theme.accent } else { theme.foreground })
        .hover(|style| style.bg(theme.hover_fill()))
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(dialog) = &mut this.options {
                    dialog.set_system_sort(sort);
                }
                this.choose_system_sort(sort);
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(sort.label());
    if divider {
        segment = segment.border_l_1().border_color(theme.border);
    }
    segment
}

fn split_list_row(
    id: impl Into<gpui_kit::ElementId>,
    title: String,
    detail: String,
    selected: bool,
    cx: &Context<Shell>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let theme = cx.omarchy();
    let mut row = split::row_metrics(div().id(id))
        .cursor_pointer()
        .bg(if selected {
            theme.selected_fill()
        } else {
            theme.background
        })
        .border_1()
        .border_color(if selected { theme.accent } else { theme.border })
        .hover(|style| style.bg(theme.hover_fill()))
        .child(split::title_metrics(div()).child(title));
    if !detail.is_empty() {
        row = row.child(
            split::detail_metrics(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .min_h(px(split::DETAIL_LINE))
                    .text_color(theme.secondary)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis(),
            )
            .child(detail),
        );
    }
    row
}

fn field_label(text: &str, cx: &Context<Shell>) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .text_color(cx.omarchy().secondary)
        .child(text.to_string())
}

fn input_row(row: options::Row, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let slot = row.slot;
    div()
        .id(input_row_id(slot))
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(px(8.))
        .py(px(6.))
        .border_1()
        .border_color(if row.aimed {
            theme.accent
        } else {
            theme.border
        })
        .bg(theme.surface)
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(dialog) = &mut this.options {
                    dialog.aim_input(slot);
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child(row.title))
                .child(input_step(slot, -1, cx))
                .child(
                    div()
                        .w(px(64.))
                        .flex()
                        .justify_end()
                        .child(row.value.to_string()),
                )
                .child(input_step(slot, 1, cx)),
        )
        .child(
            div()
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(theme.secondary)
                .child(row.subtitle),
        )
}

fn input_row_id(slot: options::Slot) -> &'static str {
    match slot {
        options::Slot::Starting => "input-starting",
        options::Slot::Slow => "input-slow",
        options::Slot::Fast => "input-fast",
        options::Slot::Ramp => "input-ramp",
        options::Slot::Close => "input-close",
    }
}

fn input_step(slot: options::Slot, steps: i32, cx: &Context<Shell>) -> impl IntoElement {
    let label = if steps < 0 { "−" } else { "+" };
    let id = format!(
        "{}-{}",
        input_row_id(slot),
        if steps < 0 { "dec" } else { "inc" }
    );
    button(id, label, ButtonVariant::Secondary, cx).on_click(cx.listener(
        move |this: &mut Shell, _: &ClickEvent, window, cx| {
            let changed = this
                .options
                .as_mut()
                .is_some_and(|dialog| dialog.step_input(slot, steps));
            if changed {
                this.commit_input();
            }
            this.focus_handle.focus(window, cx);
            cx.notify();
        },
    ))
}

fn options_close(dialog: &Options, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .border_1()
        .border_color(if dialog.input_close_aimed() {
            theme.accent
        } else {
            theme.background
        })
        .child(
            button("options-close", "Close", ButtonVariant::Secondary, cx).on_click(cx.listener(
                |this: &mut Shell, _: &ClickEvent, window, cx| {
                    if let Some(dialog) = &mut this.options {
                        dialog.aim_input(options::Slot::Close);
                    }
                    this.confirm_options(cx);
                    this.focus_handle.focus(window, cx);
                    cx.notify();
                },
            )),
        )
}

fn kind_block(kind: EmulatorKind, aimed: bool, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .id("emulator-kind")
        .flex()
        .border_1()
        .border_color(if aimed { theme.accent } else { theme.border })
        .child(kind_segment(EmulatorKind::Standalone, kind, false, cx))
        .child(kind_segment(EmulatorKind::RetroArch, kind, true, cx))
}

fn kind_segment(
    kind: EmulatorKind,
    selected: EmulatorKind,
    divider: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let on = kind == selected;
    let mut segment = div()
        .id(match kind {
            EmulatorKind::Standalone => "emulator-kind-standalone",
            EmulatorKind::RetroArch => "emulator-kind-retroarch",
        })
        .px(px(10.))
        .py(px(6.))
        .cursor_pointer()
        .bg(if on {
            theme.selected_fill()
        } else {
            theme.background
        })
        .text_color(if on { theme.accent } else { theme.foreground })
        .hover(|style| style.bg(theme.hover_fill()))
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(dialog) = &mut this.emulators {
                    dialog.set_kind(kind);
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(kind.label());
    if divider {
        segment = segment.border_l_1().border_color(theme.border);
    }
    segment
}

fn emulator_editor(
    id: &'static str,
    edit: &crate::game_menu::LineEdit,
    focused: bool,
    placeholder: &'static str,
    field: EmulatorField,
    cx: &Context<Shell>,
) -> impl IntoElement {
    plain_editor(id, edit, focused, placeholder, cx).on_click(cx.listener(
        move |this: &mut Shell, _: &ClickEvent, window, cx| {
            if let Some(dialog) = &mut this.emulators {
                dialog.aim(field);
            }
            this.focus_handle.focus(window, cx);
            cx.notify();
        },
    ))
}

fn system_editor(
    id: &'static str,
    edit: &crate::game_menu::LineEdit,
    focused: bool,
    placeholder: &'static str,
    field: SystemField,
    cx: &Context<Shell>,
) -> impl IntoElement {
    plain_editor(id, edit, focused, placeholder, cx).on_click(cx.listener(
        move |this: &mut Shell, _: &ClickEvent, window, cx| {
            if let Some(dialog) = &mut this.systems {
                dialog.aim(field);
            }
            this.focus_handle.focus(window, cx);
            cx.notify();
        },
    ))
}

fn plain_editor(
    id: impl Into<gpui_kit::ElementId>,
    edit: &crate::game_menu::LineEdit,
    focused: bool,
    placeholder: &'static str,
    cx: &Context<Shell>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let theme = cx.omarchy();
    let mut text = div()
        .flex()
        .flex_row()
        .items_center()
        .overflow_hidden()
        .min_h(px(18.));
    if edit.text.is_empty() {
        if focused {
            text = text.child(
                div()
                    .w(px(1.))
                    .h(px(16.))
                    .flex_shrink_0()
                    .bg(theme.foreground),
            );
        }
        text = text.child(div().text_color(theme.secondary).child(placeholder));
    } else {
        let line = edit.caret_line();
        text = text.child(line.head.clone());
        if focused {
            text = text.child(
                div()
                    .w(px(1.))
                    .h(px(16.))
                    .flex_shrink_0()
                    .bg(theme.foreground),
            );
        }
        text = text.child(line.tail.clone());
    }
    div()
        .id(id)
        .w_full()
        .px(px(8.))
        .py(px(6.))
        .border_1()
        .border_color(if focused { theme.accent } else { theme.border })
        .bg(theme.surface)
        .child(text)
}

fn emulator_press(
    id: String,
    label: String,
    variant: ButtonVariant,
    aimed: bool,
    field: EmulatorField,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .border_1()
        .border_color(if aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(button(id, label, variant, cx).on_click(cx.listener(
            move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(dialog) = &mut this.emulators {
                    dialog.aim(field);
                }
                this.confirm_emulators(cx);
                this.focus_handle.focus(window, cx);
                cx.notify();
            },
        )))
}

fn system_choice(
    id: String,
    label: String,
    aimed: bool,
    field: SystemField,
    menu: Option<&crate::picker::Menu>,
    scroll: &ScrollHandle,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let mut column = div()
        .w_full()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(px(4.))
        .child(system_press(id, label, aimed, field, cx));
    if let Some(menu) = menu {
        column = column.child(picker::menu(
            menu,
            scroll,
            240.,
            cx,
            |this, index, window, cx| {
                this.pick_system(index, cx);
                this.focus_handle.focus(window, cx);
                cx.notify();
            },
        ));
    }
    column
}

fn system_action(
    id: String,
    label: String,
    variant: ButtonVariant,
    aimed: bool,
    field: SystemField,
    enabled: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let control = button(id, label, variant, cx);
    let control = if enabled {
        control.on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(dialog) = &mut this.systems {
                    dialog.aim(field);
                }
                this.confirm_systems(cx);
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
    } else {
        control.disabled(true)
    };
    div()
        .border_1()
        .border_color(if aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(control)
}

fn system_press(
    id: String,
    label: String,
    aimed: bool,
    field: SystemField,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .border_1()
        .border_color(if aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(
            button(id, label, ButtonVariant::Secondary, cx).on_click(cx.listener(
                move |this: &mut Shell, _: &ClickEvent, window, cx| {
                    if let Some(dialog) = &mut this.systems {
                        dialog.click_field(field);
                    }
                    this.focus_handle.focus(window, cx);
                    cx.notify();
                },
            )),
        )
}

fn scraper_block(block: ScraperBlock, cx: &Context<Shell>) -> gpui_kit::AnyElement {
    match block {
        ScraperBlock::Heading(text) => heading(text).into_any_element(),
        ScraperBlock::Note(text) => hint(text, cx).into_any_element(),
        ScraperBlock::Art(line) => scraper_check(line, cx).into_any_element(),
        ScraperBlock::Provider(line) => scraper_provider(line, cx).into_any_element(),
        ScraperBlock::Field(line) => scraper_field(line, cx).into_any_element(),
        ScraperBlock::Error(text) => import_error(&text, cx).into_any_element(),
        ScraperBlock::Save { aimed } => div()
            .flex()
            .justify_end()
            .child(scraper_action(
                "scraper-save",
                "Save",
                ButtonVariant::Primary,
                aimed,
                ScraperSlot::Save,
                cx,
            ))
            .into_any_element(),
    }
}

fn scraper_check(line: crate::scraper_settings::ArtLine, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let id = match line.slot {
        ScraperSlot::BoxArt => "scraper-box-art",
        ScraperSlot::Screenshot => "scraper-screenshot",
        _ => "scraper-art",
    };
    let state = if line.on {
        CheckboxState::Checked
    } else {
        CheckboxState::Unchecked
    };
    let slot = line.slot;
    let entity = cx.entity();
    div()
        .border_1()
        .border_color(if line.aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(
            checkbox(id, line.label, state, cx).on_change(move |state, _, window, cx| {
                entity.update(cx, |this, cx| {
                    if let Some(dialog) = &mut this.options {
                        dialog.focus_panel();
                        let scraper = dialog.scraper_mut();
                        scraper.aim(slot);
                        scraper.set_checked(slot, state == CheckboxState::Checked);
                    }
                    this.focus_handle.focus(window, cx);
                    cx.notify();
                });
            }),
        )
}

fn scraper_provider(
    line: crate::scraper_settings::ProviderLine,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let state = if line.enabled {
        CheckboxState::Checked
    } else {
        CheckboxState::Unchecked
    };
    let index = line.index;
    let enable = ScraperSlot::Provider {
        index,
        part: ScraperPart::Enabled,
    };
    let entity = cx.entity();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .border_1()
                .border_color(if line.check_aimed {
                    theme.accent
                } else {
                    theme.background
                })
                .child(
                    checkbox(("scraper-enable", index), line.label, state, cx).on_change(
                        move |state, _, window, cx| {
                            entity.update(cx, |this, cx| {
                                if let Some(dialog) = &mut this.options {
                                    dialog.focus_panel();
                                    let scraper = dialog.scraper_mut();
                                    scraper.aim(enable);
                                    scraper.set_checked(enable, state == CheckboxState::Checked);
                                }
                                this.focus_handle.focus(window, cx);
                                cx.notify();
                            });
                        },
                    ),
                ),
        )
        .child(scraper_move(index, true, line.up_on, line.up_aimed, cx))
        .child(scraper_move(
            index,
            false,
            line.down_on,
            line.down_aimed,
            cx,
        ))
}

fn scraper_move(
    index: usize,
    up: bool,
    enabled: bool,
    aimed: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let label = if up { "Up" } else { "Down" };
    let id = if up {
        format!("scraper-up-{index}")
    } else {
        format!("scraper-down-{index}")
    };
    let slot = ScraperSlot::Provider {
        index,
        part: if up {
            ScraperPart::Up
        } else {
            ScraperPart::Down
        },
    };
    let control = button(id, label, ButtonVariant::Secondary, cx);
    let control = if enabled {
        control.on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                let aimed = this.options.as_mut().is_some_and(|dialog| {
                    dialog.focus_panel();
                    dialog.scraper_mut().aim(slot)
                });
                if aimed {
                    this.confirm_options(cx);
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
    } else {
        control.disabled(true)
    };
    div()
        .border_1()
        .border_color(if aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(control)
}

fn scraper_field(
    line: crate::scraper_settings::FieldLine,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let id = match line.slot {
        ScraperSlot::User => "scraper-user",
        ScraperSlot::Password => "scraper-password",
        ScraperSlot::ApiKey => "scraper-api-key",
        _ => "scraper-field",
    };
    let slot = line.slot;
    let mut text = div()
        .flex()
        .flex_row()
        .items_center()
        .overflow_hidden()
        .min_h(px(18.));
    if line.edit.text.is_empty() {
        if line.aimed {
            text = text.child(
                div()
                    .w(px(1.))
                    .h(px(16.))
                    .flex_shrink_0()
                    .bg(theme.foreground),
            );
        }
        text = text.child(div().text_color(theme.secondary).child(line.placeholder));
    } else {
        let caret = line.edit.caret_line();
        text = text.child(caret.head);
        if line.aimed {
            text = text.child(
                div()
                    .w(px(1.))
                    .h(px(16.))
                    .flex_shrink_0()
                    .bg(theme.foreground),
            );
        }
        text = text.child(caret.tail);
    }
    div()
        .id(id)
        .w_full()
        .px(px(8.))
        .py(px(6.))
        .border_1()
        .border_color(if line.aimed {
            theme.accent
        } else {
            theme.border
        })
        .bg(theme.surface)
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(dialog) = &mut this.options {
                    dialog.focus_panel();
                    dialog.scraper_mut().aim(slot);
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(text)
}

fn scraper_action(
    id: &'static str,
    label: &'static str,
    variant: ButtonVariant,
    aimed: bool,
    slot: ScraperSlot,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .border_1()
        .border_color(if aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(button(id, label, variant, cx).on_click(cx.listener(
            move |this: &mut Shell, _: &ClickEvent, window, cx| {
                let aimed = this.options.as_mut().is_some_and(|dialog| {
                    dialog.focus_panel();
                    dialog.scraper_mut().aim(slot)
                });
                if aimed {
                    this.confirm_options(cx);
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            },
        )))
}

fn screen_for_menu(item: MenuItem) -> Screen {
    match item {
        MenuItem::Emulators => Screen::Emulators,
        MenuItem::Systems => Screen::Systems,
        MenuItem::Options => Screen::Options(OptionsSection::Input),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Emulators,
    Systems,
    Options(OptionsSection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScreenMenu {
    Stay,
    Section(OptionsSection),
    Open(Screen),
}

fn screen_menu(open: Option<Screen>, target: Screen) -> ScreenMenu {
    match (open, target) {
        (Some(Screen::Options(current)), Screen::Options(next)) if current != next => {
            ScreenMenu::Section(next)
        }
        (Some(current), next) if current == next => ScreenMenu::Stay,
        (_, next) => ScreenMenu::Open(next),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_click_on_the_open_screen_does_nothing() {
        assert_eq!(
            screen_menu(Some(Screen::Systems), Screen::Systems),
            ScreenMenu::Stay
        );
        assert_eq!(
            screen_menu(Some(Screen::Emulators), Screen::Emulators),
            ScreenMenu::Stay
        );
        assert_eq!(
            screen_menu(
                Some(Screen::Options(OptionsSection::Input)),
                Screen::Options(OptionsSection::Input)
            ),
            ScreenMenu::Stay
        );
        assert_eq!(
            screen_menu(
                Some(Screen::Options(OptionsSection::Scraper)),
                Screen::Options(OptionsSection::Scraper)
            ),
            ScreenMenu::Stay
        );
    }

    #[test]
    fn menu_click_moves_within_options() {
        assert_eq!(
            screen_menu(
                Some(Screen::Options(OptionsSection::Input)),
                Screen::Options(OptionsSection::Scraper)
            ),
            ScreenMenu::Section(OptionsSection::Scraper)
        );
        assert_eq!(
            screen_menu(
                Some(Screen::Options(OptionsSection::Scraper)),
                Screen::Options(OptionsSection::Input)
            ),
            ScreenMenu::Section(OptionsSection::Input)
        );
    }

    #[test]
    fn menu_click_leaves_one_screen_for_another() {
        assert_eq!(
            screen_menu(Some(Screen::Systems), Screen::Emulators),
            ScreenMenu::Open(Screen::Emulators)
        );
        assert_eq!(
            screen_menu(Some(Screen::Emulators), Screen::Systems),
            ScreenMenu::Open(Screen::Systems)
        );
        assert_eq!(
            screen_menu(None, Screen::Options(OptionsSection::Scraper)),
            ScreenMenu::Open(Screen::Options(OptionsSection::Scraper))
        );
        assert_eq!(
            screen_menu(
                Some(Screen::Systems),
                Screen::Options(OptionsSection::Input)
            ),
            ScreenMenu::Open(Screen::Options(OptionsSection::Input))
        );
    }
}
