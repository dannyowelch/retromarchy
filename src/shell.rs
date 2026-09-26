use crate::browse::{
    clamp_cover_width, columns_for, cover_path, file_for, format_play_time, image_aspect,
    key_from_parts, resolve_profile, row_of, scrape_chord, Browse, Confirm, Key, LibraryKind, Pane,
    ScrapeChord, TileFrame, COVER_WIDTH_MAX, COVER_WIDTH_MIN, COVER_WIDTH_STEP, DETAILS_PAD,
    DETAILS_WIDTH, GRID_PAD, SIDEBAR_WIDTH, TILE_GAP,
};
use crate::config::{self, InputSettings};
use crate::cores;
use crate::database;
use crate::emulators::{emulator_key, Block, EmulatorCommand, Emulators, Kind, Slot};
use crate::game_menu::{
    game_menu_key, Aim, DeleteSlot, Overlay, OverlayCommand, RenameSlot, ScrapeSlot,
};
use crate::gamepad::{NavDir, PadAction, PadHeld};
use crate::import_wizard::{
    import_key, ChooseSlot, FolderSlot, ImportCommand, ImportFocus, ImportWizard, PickSlot,
    RootSlot, SystemsSlot, View as ImportView,
};
use crate::importer::{self, ImportUpdate};
use crate::input_repeat::HoldRepeat;
use crate::launcher;
use crate::scraper::{self, NameSearch, ScrapeUpdate};
use crate::types::{DeleteOptions, Game, GridArt, GridFilter, Media, MediaKind};
use gpui_kit::base::slider::{SliderEvent, SliderState};
use gpui_kit::base::CheckboxState;
use gpui_kit::{
    anchored, deferred, div, img, point, px, rgb, App, AppContext, ClickEvent, Context, Edges,
    Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke,
    MouseButton, MouseDownEvent, ObjectFit, ParentElement, PathPromptOptions, Render, ScrollHandle,
    StatefulInteractiveElement, Styled, StyledImage, Subscription, Task, Window, WindowBounds,
    WindowDecorations, WindowOptions,
};
use gpui_omarchy::{
    badge, button, checkbox, empty_state, focus_scope, keycap, separator, slider,
    vertical_separator, ActiveTheme, ButtonVariant, Status,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct Shell {
    browse: Browse,
    focus_handle: FocusHandle,
    armed: bool,
    grid_scroll: ScrollHandle,
    sidebar_scroll: ScrollHandle,
    revealed_console: Option<usize>,
    revealed_game: Option<usize>,
    revealed_columns: usize,
    cover_slider: Entity<SliderState>,
    _cover_slider_sub: Subscription,
    gilrs: Option<gilrs::Gilrs>,
    pad: PadHeld,
    /// Arrow keys and the pad share this clock. Rates come from config `[input]`.
    hold: HoldRepeat,
    input: InputSettings,
    nav_started: Instant,
    _nav_poll: Task<()>,
    search: Option<std::sync::mpsc::Receiver<NameSearch>>,
    apply: Option<std::sync::mpsc::Receiver<ScrapeUpdate>>,
    scrape_scroll: ScrollHandle,
    /// GTK Import ROMs. First run with no consoles reaches it from the empty pane.
    wizard: Option<ImportWizard>,
    import_rx: Option<std::sync::mpsc::Receiver<ImportUpdate>>,
    import_scroll: ScrollHandle,
    pick_scroll: ScrollHandle,
    /// GTK Manage Emulators. Writes `config.toml` as each change is confirmed.
    emulators: Option<Emulators>,
    emulator_scroll: ScrollHandle,
    picking_core: bool,
    /// Closing the dialog drops whatever control had focus. The next frame
    /// puts the keyboard back on the shell.
    refocus: bool,
}

/// GTK favorite red (`#e01b24`).
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
            grid_scroll: ScrollHandle::new(),
            sidebar_scroll: ScrollHandle::new(),
            revealed_console: None,
            revealed_game: None,
            revealed_columns: 0,
            cover_slider,
            _cover_slider_sub: cover_slider_sub,
            gilrs,
            pad: PadHeld::default(),
            hold: HoldRepeat::default(),
            input: load_input(),
            nav_started,
            _nav_poll: nav_poll,
            search: None,
            apply: None,
            scrape_scroll: ScrollHandle::new(),
            wizard: None,
            import_rx: None,
            import_scroll: ScrollHandle::new(),
            pick_scroll: ScrollHandle::new(),
            emulators: None,
            emulator_scroll: ScrollHandle::new(),
            picking_core: false,
            refocus: false,
        }
    }

    /// Face buttons are edges. Held directions step through [`HoldRepeat`].
    /// South enters the grid or launches. East returns to the system list.
    /// North toggles a favorite. Select opens the GTK game menu, and closes it.
    fn poll_nav(&mut self, cx: &mut Context<Self>) {
        let events = self.drain_pad_events();
        let mut changed = self.poll_jobs();
        if self.emulators.is_some() {
            changed |= self.poll_emulator_pad(&events, cx);
            if changed {
                cx.notify();
            }
            return;
        }
        if self.wizard.is_some() {
            changed |= self.poll_wizard_pad(&events, cx);
            if changed {
                cx.notify();
            }
            return;
        }
        for event in &events {
            match self.pad.apply(event) {
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
            None => false,
        }
    }

    fn drain_pad_events(&mut self) -> Vec<gilrs::EventType> {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
            events.push(event);
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
        if self.emulators.is_some() {
            self.on_emulator_key(&event.keystroke, false, cx);
            return;
        }
        if self.wizard.is_some() {
            self.on_wizard_key(&event.keystroke, false, cx);
            return;
        }
        if self.browse.overlay_open() {
            self.on_overlay_key(&event.keystroke, false, cx);
            return;
        }
        if !event.is_held
            && import_key(
                event.keystroke.key.as_str(),
                event.keystroke.key_char.as_deref(),
                event.keystroke.modifiers.control,
            )
        {
            self.open_import();
            cx.notify();
            cx.stop_propagation();
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
        if self.on_arrow(&event.keystroke, false, cx) {
            return;
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
        let Some(key) = key_from_parts(
            keystroke.key.as_ref(),
            keystroke.key_char.as_deref(),
            modified,
        ) else {
            return;
        };
        let before = self.browse.cover_width;
        if key == Key::Launch {
            self.launch_selected();
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
        if self.emulators.is_some() {
            self.on_emulator_key(&event.keystroke, true, cx);
            return;
        }
        if self.wizard.is_some() {
            self.on_wizard_key(&event.keystroke, true, cx);
            return;
        }
        if self.browse.overlay_open() {
            self.on_overlay_key(&event.keystroke, true, cx);
            return;
        }
        self.on_arrow(&event.keystroke, true, cx);
    }

    /// Arrows and Tab move the wizard. Enter confirms the focused control.
    /// Esc closes, except while a scan is writing the library.
    fn on_wizard_key(&mut self, keystroke: &Keystroke, release: bool, cx: &mut Context<Self>) {
        if let Some(dir) = arrow_dir(keystroke) {
            let modified = keystroke.modifiers.control
                || keystroke.modifiers.alt
                || keystroke.modifiers.platform;
            if !(modified && !release) {
                let now = monotonic_ms(self.nav_started);
                if release {
                    self.hold.release(&self.input, dir, now);
                } else if self.hold.press(&self.input, dir, now) > 0 {
                    if let Some(wizard) = &mut self.wizard {
                        wizard.move_dir(dir);
                    }
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
        let modified =
            keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform;
        if key == "escape" {
            self.dismiss_import();
            cx.notify();
        } else if key == "enter" {
            self.confirm_import(cx);
            cx.notify();
        } else if key == "tab" {
            if let Some(wizard) = &mut self.wizard {
                wizard.tab(keystroke.modifiers.shift);
            }
            cx.notify();
        } else if key == "backspace" {
            if let Some(wizard) = &mut self.wizard {
                wizard.backspace();
            }
            cx.notify();
        } else if key == "delete" {
            if let Some(wizard) = &mut self.wizard {
                wizard.delete_forward();
            }
            cx.notify();
        } else if key == "space" && !modified {
            let typing = self
                .wizard
                .as_ref()
                .is_some_and(|wizard| wizard.accepts_text());
            let toggled = self
                .wizard
                .as_mut()
                .is_some_and(|wizard| wizard.toggle_focused());
            if typing {
                if let Some(wizard) = &mut self.wizard {
                    wizard.type_text(" ");
                }
            } else if !toggled {
                self.confirm_import(cx);
            }
            cx.notify();
        } else if !modified {
            if let Some(text) = typed_text(keystroke) {
                if let Some(wizard) = &mut self.wizard {
                    wizard.type_text(text);
                }
                cx.notify();
            }
        }
        cx.stop_propagation();
    }

    fn poll_wizard_pad(&mut self, events: &[gilrs::EventType], cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for event in events {
            match self.pad.apply(event) {
                Some(PadAction::Confirm) => {
                    self.confirm_import(cx);
                    changed = true;
                }
                Some(PadAction::Back) => {
                    self.dismiss_import();
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
            if let Some(wizard) = &mut self.wizard {
                wizard.move_dir(dir);
                changed = true;
            }
        }
        changed
    }

    fn poll_emulator_pad(&mut self, events: &[gilrs::EventType], cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for event in events {
            match self.pad.apply(event) {
                Some(PadAction::Confirm) => {
                    self.confirm_emulators(cx);
                    changed = true;
                }
                Some(PadAction::Back) => {
                    self.close_emulators();
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
            let write = self
                .emulators
                .as_mut()
                .is_some_and(|dialog| dialog.move_dir(dir));
            if write {
                self.write_emulators();
            }
            changed = true;
        }
        changed
    }

    /// Arrows and Tab move. Enter confirms the focused control. Esc closes.
    /// A console's left/right choice is written immediately, same as GTK's combo.
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
                    let write = self
                        .emulators
                        .as_mut()
                        .is_some_and(|dialog| dialog.move_dir(dir));
                    if write {
                        self.write_emulators();
                    }
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
            self.close_emulators();
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
            if let Some(text) = typed_text(keystroke) {
                if let Some(dialog) = &mut self.emulators {
                    dialog.type_text(text);
                }
                cx.notify();
            }
        }
        cx.stop_propagation();
    }

    fn close_emulators(&mut self) {
        self.emulators = None;
        self.picking_core = false;
        self.refocus = true;
    }

    fn open_emulators(&mut self) {
        if self.import_rx.is_some() || self.wizard.is_some() || self.emulators.is_some() {
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
        let show_cores = crate::emulators::retroarch_on_path();
        let cores = if show_cores {
            cores::discover_cores()
        } else {
            Vec::new()
        };
        self.picking_core = false;
        self.emulators = Some(Emulators::open(&config, cores, show_cores));
    }

    fn confirm_emulators(&mut self, cx: &mut Context<Self>) {
        let Some(command) = self.emulators.as_mut().map(|dialog| dialog.confirm()) else {
            return;
        };
        match command {
            EmulatorCommand::None => {}
            EmulatorCommand::Close => self.close_emulators(),
            EmulatorCommand::Browse => self.pick_core(cx),
            EmulatorCommand::Write => self.write_emulators(),
        }
    }

    fn write_emulators(&mut self) {
        let (profiles, consoles) = {
            let Some(dialog) = &self.emulators else {
                return;
            };
            (dialog.profiles().to_vec(), dialog.consoles().to_vec())
        };
        let mut config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                if let Some(dialog) = &mut self.emulators {
                    dialog.set_error(format!("Could not read config ({err})."));
                }
                return;
            }
        };
        crate::emulators::apply_assignments(&mut config, &profiles, &consoles);
        if let Err(err) = config::save_config(&config) {
            if let Some(dialog) = &mut self.emulators {
                dialog.set_error(format!("Could not save config ({err})."));
            }
            return;
        }
        self.browse.library.profiles = config.profiles.clone();
        for shelf in &mut self.browse.library.shelves {
            if let Some(console) = config.consoles.iter().find(|c| c.id == shelf.console.id) {
                shelf.console.profile = console.profile.clone();
            }
        }
    }

    fn open_import(&mut self) {
        if self.import_rx.is_some() || self.wizard.is_some() || self.emulators.is_some() {
            return;
        }
        self.browse.close_overlay();
        self.search = None;
        self.wizard = Some(ImportWizard::open());
    }

    fn dismiss_import(&mut self) {
        if self.import_rx.is_some()
            || self
                .wizard
                .as_ref()
                .is_some_and(|wizard| wizard.blocks_escape())
        {
            return;
        }
        self.wizard = None;
    }

    fn confirm_import(&mut self, cx: &mut Context<Self>) {
        if self.import_rx.is_some() {
            return;
        }
        let Some(command) = self.wizard.as_mut().map(|wizard| wizard.confirm()) else {
            return;
        };
        match command {
            ImportCommand::None => {}
            ImportCommand::Close => self.wizard = None,
            ImportCommand::Discover(path) => {
                self.import_rx = Some(importer::spawn_discover(path));
            }
            ImportCommand::Apply(choices) => {
                self.import_rx = Some(importer::spawn_apply(choices));
            }
            ImportCommand::Browse => self.pick_folder(cx),
        }
    }

    /// GTK's Browse button is a folder chooser. GPUI's is the XDG desktop portal
    /// via [`App::prompt_for_paths`]. The path field stays editable either way.
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
                this.finish_pick(picked);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_pick(&mut self, picked: Picked) {
        match picked {
            Picked::Cancel => {}
            Picked::Path(path) => {
                if let Some(wizard) = &mut self.wizard {
                    wizard.set_path(path);
                }
            }
            Picked::Failed(message) => {
                if let Some(wizard) = &mut self.wizard {
                    wizard.set_error(message);
                } else {
                    self.browse.status = message;
                }
            }
        }
    }

    /// GTK's core Browse is an open-file chooser filtered to `*.so`. GPUI's
    /// prompt has no filter, so the path field stays editable.
    fn pick_core(&mut self, cx: &mut Context<Self>) {
        if self.picking_core {
            return;
        }
        self.picking_core = true;
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
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
                    Picked::Failed(format!("Core picker unavailable ({err}). Type the path."))
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.finish_core_pick(picked);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_core_pick(&mut self, picked: Picked) {
        self.picking_core = false;
        match picked {
            Picked::Cancel => {}
            Picked::Path(path) => {
                if let Some(dialog) = &mut self.emulators {
                    dialog.set_core_path(path);
                }
            }
            Picked::Failed(message) => {
                if let Some(dialog) = &mut self.emulators {
                    dialog.set_error(message);
                }
            }
        }
    }

    fn reload_library(&mut self, status: String) {
        let cover = self.browse.cover_width;
        let details = self.browse.details_open;
        let filter = self.browse.filter;
        let library = match config::load_config() {
            Ok(config) => match database::init_db() {
                Ok(conn) => crate::browse::from_config(&config, &conn),
                Err(err) => {
                    self.browse.status = format!("Could not open the library database ({err}).");
                    return;
                }
            },
            Err(err) => {
                self.browse.status = format!("Could not read config ({err}).");
                return;
            }
        };
        self.browse = Browse::with_cover_width(library, cover);
        self.browse.details_open = details;
        self.browse.filter = filter;
        self.revealed_console = None;
        self.revealed_game = None;
        self.revealed_columns = 0;
        self.input = load_input();
        self.browse.status = status;
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
                removed.game.title,
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
        let Some(targets) = self.browse.missing_scrape_games() else {
            return;
        };
        if self.apply.is_some() {
            self.browse.status = "A scrape is already running.".into();
            return;
        }
        let config = match config::load_config() {
            Ok(config) => config,
            Err(err) => {
                self.browse.status = format!("Could not read scraper settings ({err}).");
                return;
            }
        };
        self.browse.status = "Scraping artwork…".into();
        self.apply = Some(scraper::spawn_scrape(
            targets,
            config.scraper,
            scraper::fixture_dir_from_env(),
        ));
    }

    fn start_apply(&mut self, game_id: String, candidate: crate::scraper::ScrapeCandidate) {
        self.search = None;
        if self.apply.is_some() {
            self.browse.status = "A scrape is already running.".into();
            return;
        }
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
                Ok(ScrapeUpdate::Done(text)) => {
                    self.apply = None;
                    self.browse.status = text;
                }
                Err(()) => {
                    self.apply = None;
                    self.browse.status = "Scrape stopped.".into();
                }
            }
            changed = true;
        }
        changed |= self.poll_import();
        changed
    }

    fn poll_import(&mut self) -> bool {
        let update = self.import_rx.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(update) => Some(update),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(ImportUpdate::Failed("Import stopped.".into()))
            }
        });
        let Some(update) = update else {
            return false;
        };
        match update {
            ImportUpdate::Discover(found) => {
                self.import_rx = None;
                if let Some(wizard) = &mut self.wizard {
                    wizard.show_folders(found);
                }
            }
            ImportUpdate::Status(text) => {
                if let Some(wizard) = &mut self.wizard {
                    wizard.note_progress(text);
                }
            }
            ImportUpdate::Done { systems, games } => {
                self.import_rx = None;
                self.wizard = None;
                let status = if systems == 0 {
                    "Nothing new to import.".into()
                } else {
                    format!("Scanned {games} games into {systems} systems.")
                };
                self.reload_library(status);
            }
            ImportUpdate::Failed(message) => {
                self.import_rx = None;
                if let Some(wizard) = &mut self.wizard {
                    wizard.fail(message);
                } else {
                    self.browse.status = message;
                }
            }
        }
        true
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
        self.browse.remember_media(game_id, media);
    }

    /// Arrow keys use the same clock as the pad. `release` clears a hold.
    /// A modified arrow is left for the platform. Returns whether this was an arrow.
    fn on_arrow(&mut self, keystroke: &Keystroke, release: bool, cx: &mut Context<Self>) -> bool {
        let Some(dir) = arrow_dir(keystroke) else {
            return false;
        };
        let modified =
            keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform;
        if modified && !release {
            return true;
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
        true
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

    fn launch_selected(&mut self) {
        let Some(game) = self.browse.selected_game().cloned() else {
            self.browse.status = "Select a game, then press Enter.".into();
            return;
        };
        let Some(profile) = resolve_profile(&self.browse.library, &game) else {
            self.browse.status =
                "This system has no emulator profile. Use Manage Emulators to add one and assign it."
                    .into();
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

    fn reveal_emulators(&mut self) {
        let Some(dialog) = &self.emulators else {
            return;
        };
        if self.emulator_scroll.bounds().size.height > px(0.) {
            self.emulator_scroll.scroll_to_item(dialog.scroll_index());
        }
    }

    fn reveal_import(&mut self) {
        let Some(wizard) = &self.wizard else {
            return;
        };
        if self.import_scroll.bounds().size.height > px(0.) {
            if let Some(row) = wizard.systems_row() {
                self.import_scroll.scroll_to_item(row);
            }
        }
        if self.pick_scroll.bounds().size.height > px(0.) {
            if let Some(row) = wizard.pick_row() {
                self.pick_scroll.scroll_to_item(row);
            }
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width.as_f32();
        let frame = self.browse.tile_frame();
        self.browse
            .set_columns(columns_for(width, self.browse.details_open, frame.width));
        self.reveal_selection();
        self.reveal_import();
        self.reveal_emulators();
        if let Overlay::Scrape(prompt) = &self.browse.overlay {
            if prompt.slot == ScrapeSlot::Results {
                self.scrape_scroll.scroll_to_item(prompt.cursor);
            }
        }
        if !self.armed {
            self.armed = true;
            self.focus_handle.focus(window, cx);
        }
        if self.refocus {
            self.refocus = false;
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
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .capture_key_up(cx.listener(|this, event: &KeyUpEvent, _window, cx| {
                this.on_key_up(event, cx);
            }))
            .child(header(&self.browse, &theme_name, cx))
            .children(note_bar(&self.browse.library.note, cx))
            .child(body(
                &self.browse,
                &self.grid_scroll,
                &self.sidebar_scroll,
                cx,
            ))
            .child(status_line(&self.browse, &self.cover_slider, window, cx))
            .children(game_dialog(&self.browse, &self.scrape_scroll, cx))
            .children(import_dialog(
                self.wizard.as_ref(),
                &self.import_scroll,
                &self.pick_scroll,
                cx,
            ))
            .children(emulator_dialog(
                self.emulators.as_ref(),
                &self.emulator_scroll,
                cx,
            ))
    }
}

fn header(browse: &Browse, theme_name: &str, cx: &Context<Shell>) -> impl IntoElement {
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
    row.child(
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(4.))
            .child(import_button(cx))
            .child(emulators_button(cx))
            .child(scrape_button(false, cx))
            .child(scrape_button(true, cx)),
    )
    .child(div().flex_1())
    .child(filter_control(browse, cx))
    .child(
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
            .child(keycap("f", cx))
            .child("favorite")
            .child(keycap("s", cx))
            .child("scrape")
            .child(keycap("S", cx))
            .child("missing")
            .child(keycap("menu", cx))
            .child("game")
            .child(keycap("esc", cx))
            .child("clear")
            .child(keycap("-/+", cx))
            .child("size"),
    )
}

fn emulators_button(cx: &Context<Shell>) -> impl IntoElement {
    button(
        "manage-emulators",
        "Manage Emulators",
        ButtonVariant::Secondary,
        cx,
    )
    .flex_shrink_0()
    .on_click(cx.listener(|this: &mut Shell, _: &ClickEvent, window, cx| {
        this.open_emulators();
        this.focus_handle.focus(window, cx);
        cx.notify();
    }))
}

fn import_button(cx: &Context<Shell>) -> impl IntoElement {
    button("import-roms", "Import ROMs", ButtonVariant::Secondary, cx)
        .flex_shrink_0()
        .on_click(cx.listener(|this: &mut Shell, _: &ClickEvent, window, cx| {
            this.open_import();
            this.focus_handle.focus(window, cx);
            cx.notify();
        }))
}

fn scrape_button(missing: bool, cx: &Context<Shell>) -> impl IntoElement {
    let (id, label) = if missing {
        ("scrape-missing", "Scrape Missing")
    } else {
        ("scrape-selected", "Scrape")
    };
    button(id, label, ButtonVariant::Secondary, cx)
        .flex_shrink_0()
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if missing {
                    this.scrape_missing();
                } else {
                    this.scrape_selected();
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
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
        .px(px(10.))
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
        return pane.child(import_empty(cx));
    };
    let visible: Vec<&Game> = shelf
        .games
        .iter()
        .filter(|game| browse.filter.matches(game))
        .collect();
    if visible.is_empty() {
        if shelf.games.is_empty() {
            return pane.child(import_empty(cx));
        }
        return pane.child(empty_state(
            "No favorites",
            "Show All, select a game, and press f.",
            cx,
        ));
    }
    let columns = browse.columns.max(1);
    let art = shelf.console.grid_art;
    let demo = browse.library.kind == LibraryKind::Demo;
    let count = visible.len();
    for start in (0..count).step_by(columns) {
        let end = (start + columns).min(count);
        let mut row = div().flex().flex_row().flex_shrink_0().gap(px(TILE_GAP));
        for index in start..end {
            let menu = (browse.game == Some(index))
                .then(|| browse.menu_cursor())
                .flatten();
            row = row.child(tile(
                index,
                visible[index],
                art,
                browse.cover_width,
                browse.game == Some(index),
                menu,
                demo,
                cx,
            ));
        }
        pane = pane.child(row);
    }
    pane
}

fn tile(
    index: usize,
    game: &Game,
    art: GridArt,
    cover_width: f32,
    selected: bool,
    menu: Option<usize>,
    demo: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let frame = TileFrame::for_art(art, cover_width);
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
        .min_h(px(40.))
        .w(px(frame.width))
        .text_size(px(12.))
        .line_clamp(2)
        .child(title);
    if demo {
        caption = caption.child(div().text_color(theme.secondary).child("Placeholder"));
    }
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

fn detail_art(path: std::path::PathBuf, kind: MediaKind) -> impl IntoElement {
    let ratio = image_aspect(&path).unwrap_or(match kind {
        MediaKind::Screenshot => 4.0 / 3.0,
        _ => 3.0 / 4.0,
    });
    // Border box includes padding and the 1px left border. Height follows the
    // file so Contain fills the slot instead of a tall letterbox.
    let width = (DETAILS_WIDTH - DETAILS_PAD * 2.0 - 1.0).max(1.0);
    let height = (width / ratio.max(0.05)).max(1.0);
    img(path)
        .w_full()
        .h(px(height))
        .flex_none()
        .aspect_ratio(ratio)
        .object_fit(ObjectFit::Contain)
}

fn heading(text: &str) -> impl IntoElement {
    div()
        .flex_none()
        .font_weight(gpui_kit::FontWeight::BOLD)
        .text_size(px(16.))
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
        "Ctrl+I imports ROMs. Ctrl+M or Ctrl+E manages emulators."
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
        .child(cover_width_control(browse, cover_slider, window, cx))
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
                .font_weight(gpui_kit::FontWeight::BOLD)
                .text_size(px(16.))
                .child(title.to_string()),
        )
}

fn hint(text: &str, cx: &Context<Shell>) -> impl IntoElement {
    div()
        .text_size(px(12.))
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
    let theme = cx.omarchy();
    let mut page = dialog_page("rename-dialog", "Rename", 420., cx)
        .child(hint(
            "Library title. This does not rename the ROM file.",
            cx,
        ))
        .child(line_editor(
            "rename-title",
            &rename.edit,
            rename.slot == RenameSlot::Title,
            Aim::Rename(RenameSlot::Title),
            cx,
        ));
    if let Some(error) = &rename.error {
        page = page.child(
            div()
                .text_size(px(12.))
                .text_color(theme.danger)
                .child(error.clone()),
        );
    }
    page.child(
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

fn monotonic_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
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

fn import_empty(cx: &Context<Shell>) -> impl IntoElement {
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
                .max_w(px(420.))
                .text_center()
                .text_color(theme.secondary)
                .whitespace_normal()
                .child(
                    "Import a ROM folder to add systems to the sidebar and scan games. Only paths are stored.",
                ),
        )
        .child(
            button(
                "empty-import",
                "Import ROMs",
                ButtonVariant::Primary,
                cx,
            )
            .on_click(cx.listener(|this: &mut Shell, _: &ClickEvent, window, cx| {
                this.open_import();
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
                this.open_emulators();
                this.focus_handle.focus(window, cx);
                cx.notify();
            })),
        )
}

fn import_dialog(
    wizard: Option<&ImportWizard>,
    import_scroll: &ScrollHandle,
    pick_scroll: &ScrollHandle,
    cx: &Context<Shell>,
) -> Option<gpui_kit::AnyElement> {
    let wizard = wizard?;
    let error = wizard.error().map(str::to_string);
    let snap = match wizard.view() {
        ImportView::Choose { slot } => ImportSnap::Choose(slot),
        ImportView::Root { edit, slot } => ImportSnap::Root(edit.clone(), slot),
        ImportView::Systems { .. } => ImportSnap::Systems,
        ImportView::Pick { edit, slot } => ImportSnap::Pick(edit.clone(), slot),
        ImportView::Folder { name, edit, slot } => {
            ImportSnap::Folder(name.to_string(), edit.clone(), slot)
        }
        ImportView::Working { message } => ImportSnap::Working(message.to_string()),
    };
    let page = match snap {
        ImportSnap::Choose(slot) => choose_page(slot, error.as_deref(), cx).into_any_element(),
        ImportSnap::Root(edit, slot) => {
            root_page(&edit, slot, error.as_deref(), cx).into_any_element()
        }
        ImportSnap::Systems => {
            let (lines, slot) = wizard
                .system_lines()
                .unwrap_or_else(|| (Vec::new(), SystemsSlot::Import));
            systems_page(&lines, slot, import_scroll, error.as_deref(), cx).into_any_element()
        }
        ImportSnap::Pick(edit, slot) => {
            let matches: Vec<(String, String)> = wizard
                .pick_matches()
                .into_iter()
                .map(|(id, name)| (id.to_string(), name.to_string()))
                .collect();
            pick_page(&edit, slot, &matches, pick_scroll, error.as_deref(), cx).into_any_element()
        }
        ImportSnap::Folder(name, edit, slot) => {
            folder_page(&name, &edit, slot, error.as_deref(), cx).into_any_element()
        }
        ImportSnap::Working(message) => working_page(&message, cx).into_any_element(),
    };
    Some(modal(page).into_any_element())
}

enum ImportSnap {
    Choose(ChooseSlot),
    Root(crate::game_menu::LineEdit, RootSlot),
    Systems,
    Pick(crate::game_menu::LineEdit, PickSlot),
    Folder(String, crate::game_menu::LineEdit, FolderSlot),
    Working(String),
}

fn choose_page(slot: ChooseSlot, error: Option<&str>, cx: &Context<Shell>) -> impl IntoElement {
    let mut page = dialog_page("import-dialog", "Import ROMs", 560., cx)
        .child(
            div()
                .text_size(px(18.))
                .font_weight(gpui_kit::FontWeight::BOLD)
                .child("How would you like to import ROMs?"),
        )
        .child(hint("Paths are stored. ROM files are not copied.", cx))
        .child(import_action(
            "import-esde",
            "Import ES-DE / EmulationStation library",
            true,
            slot == ChooseSlot::Esde,
            ImportFocus::Choose(ChooseSlot::Esde),
            cx,
        ))
        .child(import_action(
            "import-single",
            "Add one system",
            false,
            slot == ChooseSlot::Single,
            ImportFocus::Choose(ChooseSlot::Single),
            cx,
        ));
    if let Some(error) = error {
        page = page.child(import_error(error, cx));
    }
    page
}

fn root_page(
    edit: &crate::game_menu::LineEdit,
    slot: RootSlot,
    error: Option<&str>,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let mut page = dialog_page("import-dialog", "Import ROMs", 560., cx)
        .child(hint(
            "ROMs root (immediate subfolders are matched to systems)",
            cx,
        ))
        .child(import_path(
            "import-root",
            edit,
            slot == RootSlot::Path,
            None,
            ImportFocus::Root(RootSlot::Path),
            cx,
        ))
        .child(import_action(
            "import-browse",
            "Browse…",
            false,
            slot == RootSlot::Browse,
            ImportFocus::Root(RootSlot::Browse),
            cx,
        ))
        .child(import_action(
            "import-scan",
            "Scan folders",
            true,
            slot == RootSlot::Scan,
            ImportFocus::Root(RootSlot::Scan),
            cx,
        ));
    if let Some(error) = error {
        page = page.child(import_error(error, cx));
    }
    page
}

fn systems_page(
    lines: &[crate::import_wizard::SystemLine],
    slot: SystemsSlot,
    scroll: &ScrollHandle,
    error: Option<&str>,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut list = div()
        .id("import-systems")
        .w_full()
        .h(px(280.))
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col()
        .gap(px(6.));
    for (index, line) in lines.iter().enumerate() {
        let check = matches!(slot, SystemsSlot::Check(row) if row == index);
        let map = matches!(slot, SystemsSlot::Map(row) if row == index);
        let state = if line.row.include {
            CheckboxState::Checked
        } else {
            CheckboxState::Unchecked
        };
        let entity = cx.entity();
        list = list.child(
            div()
                .id(("import-row", index))
                .flex()
                .items_center()
                .gap(px(8.))
                .w_full()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .border_1()
                        .border_color(if check {
                            theme.accent
                        } else {
                            theme.background
                        })
                        .child(
                            checkbox(
                                ("import-check", index),
                                line.row.discovered.clone(),
                                state,
                                cx,
                            )
                            .on_change(move |state, _, _, cx| {
                                entity.update(cx, |this, cx| {
                                    if let Some(wizard) = &mut this.wizard {
                                        wizard.set_included(index, state == CheckboxState::Checked);
                                    }
                                    cx.notify();
                                });
                            }),
                        )
                        .child(
                            div()
                                .px(px(8.))
                                .text_size(px(12.))
                                .text_color(theme.secondary)
                                .child(format!(
                                    "{} · {} files",
                                    line.row.folder_name, line.row.file_count
                                )),
                        ),
                )
                .child(
                    div()
                        .w(px(240.))
                        .flex_shrink_0()
                        .border_1()
                        .border_color(if map { theme.accent } else { theme.background })
                        .child(
                            button(
                                ("import-map", index),
                                line.mapped.clone(),
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this: &mut Shell, _: &ClickEvent, window, cx| {
                                    if let Some(wizard) = &mut this.wizard {
                                        wizard.aim(ImportFocus::Systems(SystemsSlot::Map(index)));
                                        wizard.cycle_focused(1);
                                    }
                                    this.focus_handle.focus(window, cx);
                                    cx.notify();
                                },
                            )),
                        ),
                ),
        );
    }
    let mut page = dialog_page("import-dialog", "Import ROMs", 720., cx)
        .child(hint(
            "Check systems to import. Unmatched folders stay listed so you can remap them.",
            cx,
        ))
        .child(list)
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(import_action(
                    "import-cancel",
                    "Cancel",
                    false,
                    slot == SystemsSlot::Cancel,
                    ImportFocus::Systems(SystemsSlot::Cancel),
                    cx,
                ))
                .child(import_action(
                    "import-go",
                    "Import",
                    true,
                    slot == SystemsSlot::Import,
                    ImportFocus::Systems(SystemsSlot::Import),
                    cx,
                )),
        );
    if let Some(error) = error {
        page = page.child(import_error(error, cx));
    }
    page
}

fn pick_page(
    edit: &crate::game_menu::LineEdit,
    slot: PickSlot,
    matches: &[(String, String)],
    scroll: &ScrollHandle,
    error: Option<&str>,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut list = div()
        .id("import-pick")
        .w_full()
        .h(px(280.))
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col();
    for (index, (id, name)) in matches.iter().enumerate() {
        let chosen = matches!(slot, PickSlot::Row(row) if row == index);
        let mut row = div()
            .id(("import-system", index))
            .w_full()
            .px(px(8.))
            .py(px(6.))
            .cursor_pointer()
            .on_click(
                cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                    if let Some(wizard) = &mut this.wizard {
                        wizard.aim(ImportFocus::Pick(PickSlot::Row(index)));
                    }
                    this.confirm_import(cx);
                    this.focus_handle.focus(window, cx);
                    cx.notify();
                }),
            );
        if chosen {
            row = row.bg(theme.selected_fill()).text_color(theme.accent);
        } else {
            row = row.hover(|style| style.bg(theme.hover_fill()));
        }
        list = list.child(row.child(format!("{name} ({id})")));
    }
    let mut page = dialog_page("import-dialog", "Import ROMs", 560., cx)
        .child(import_path(
            "import-search",
            edit,
            slot == PickSlot::Search,
            Some("Search systems"),
            ImportFocus::Pick(PickSlot::Search),
            cx,
        ))
        .child(list)
        .child(import_action(
            "import-choose-folder",
            "Choose folder",
            true,
            slot == PickSlot::Choose,
            ImportFocus::Pick(PickSlot::Choose),
            cx,
        ));
    if let Some(error) = error {
        page = page.child(import_error(error, cx));
    }
    page
}

fn folder_page(
    name: &str,
    edit: &crate::game_menu::LineEdit,
    slot: FolderSlot,
    error: Option<&str>,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let mut page = dialog_page("import-dialog", "Import ROMs", 560., cx).child(hint(
        &format!(
            "Folder for {name}. Individual ROM files are not copied; the parent folder is stored and filtered by extension."
        ),
        cx,
    ))
    .child(import_path(
        "import-folder",
        edit,
        slot == FolderSlot::Path,
        None,
        ImportFocus::Folder(FolderSlot::Path),
        cx,
    ))
    .child(import_action(
        "import-folder-browse",
        "Browse…",
        false,
        slot == FolderSlot::Browse,
        ImportFocus::Folder(FolderSlot::Browse),
        cx,
    ))
    .child(import_action(
        "import-add",
        "Add system",
        true,
        slot == FolderSlot::Add,
        ImportFocus::Folder(FolderSlot::Add),
        cx,
    ));
    if let Some(error) = error {
        page = page.child(import_error(error, cx));
    }
    page
}

fn working_page(message: &str, cx: &Context<Shell>) -> impl IntoElement {
    dialog_page("import-dialog", "Import ROMs", 560., cx)
        .child(hint("Paths are stored. ROM files are not copied.", cx))
        .child(message.to_string())
}

fn import_path(
    id: &'static str,
    edit: &crate::game_menu::LineEdit,
    focused: bool,
    placeholder: Option<&'static str>,
    aim: ImportFocus,
    cx: &Context<Shell>,
) -> impl IntoElement {
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
        if let Some(placeholder) = placeholder {
            text = text.child(div().text_color(theme.secondary).child(placeholder));
        }
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
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(wizard) = &mut this.wizard {
                    wizard.aim(aim);
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(text)
}

fn import_action(
    id: &'static str,
    label: &'static str,
    primary: bool,
    aimed: bool,
    aim: ImportFocus,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let variant = if primary {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Secondary
    };
    div()
        .border_1()
        .border_color(if aimed {
            theme.accent
        } else {
            theme.background
        })
        .child(button(id, label, variant, cx).on_click(cx.listener(
            move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(wizard) = &mut this.wizard {
                    wizard.aim(aim);
                }
                this.confirm_import(cx);
                this.focus_handle.focus(window, cx);
                cx.notify();
            },
        )))
}

fn import_error(text: &str, cx: &Context<Shell>) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .text_color(cx.omarchy().danger)
        .child(text.to_string())
}

fn emulator_dialog(
    dialog: Option<&Emulators>,
    scroll: &ScrollHandle,
    cx: &Context<Shell>,
) -> Option<gpui_kit::AnyElement> {
    let dialog = dialog?;
    let mut list = div()
        .id("emulator-list")
        .w_full()
        .h(px(520.))
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col()
        .gap(px(8.));
    for block in dialog.blocks() {
        list = list.child(emulator_block(block, cx));
    }
    Some(
        modal(
            dialog_page("emulator-dialog", "Manage Emulators", 820., cx)
                .child(hint(
                    "Arrows move. Left and right change a choice. Enter confirms. Esc closes.",
                    cx,
                ))
                .child(list),
        )
        .into_any_element(),
    )
}

fn emulator_block(block: Block, cx: &Context<Shell>) -> gpui_kit::AnyElement {
    match block {
        Block::Heading(text) => heading(text).into_any_element(),
        Block::Note(text) => hint(text, cx).into_any_element(),
        Block::Error(text) => import_error(&text, cx).into_any_element(),
        Block::Core(line) => core_block(line, cx).into_any_element(),
        Block::Profile(line) => profile_block(line, cx).into_any_element(),
        Block::Kind { kind, aimed } => kind_block(kind, aimed, cx).into_any_element(),
        Block::Id { edit, aimed } => {
            emulator_field("emulator-id", &edit, aimed, "Profile id", Slot::Id, cx)
                .into_any_element()
        }
        Block::Detail {
            edit,
            field_aimed,
            browse_aimed,
        } => div()
            .flex()
            .gap(px(8.))
            .items_center()
            .child(emulator_field(
                "emulator-detail",
                &edit,
                field_aimed,
                "Command with {rom}, or core path",
                Slot::Detail,
                cx,
            ))
            .child(emulator_action(
                "emulator-browse".to_string(),
                "Browse…".to_string(),
                ButtonVariant::Secondary,
                browse_aimed,
                Slot::Browse,
                cx,
            ))
            .into_any_element(),
        Block::Add { aimed } => emulator_action(
            "emulator-add".to_string(),
            "Add profile".to_string(),
            ButtonVariant::Primary,
            aimed,
            Slot::Add,
            cx,
        )
        .into_any_element(),
        Block::Console(line) => console_block(line, cx).into_any_element(),
        Block::Close { aimed } => div()
            .flex()
            .justify_end()
            .child(emulator_action(
                "emulator-close".to_string(),
                "Close".to_string(),
                ButtonVariant::Secondary,
                aimed,
                Slot::Close,
                cx,
            ))
            .into_any_element(),
    }
}

fn core_block(line: crate::emulators::CoreLine, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    let mut actions = div().flex().flex_wrap().gap(px(6.)).items_center();
    for (index, core_button) in line.buttons.into_iter().enumerate() {
        let id = format!("emulator-core-{}-{index}", line.index);
        if let Some(slot) = core_button.slot {
            let variant = if core_button.primary {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            };
            actions = actions.child(emulator_action(
                id,
                core_button.label,
                variant,
                core_button.aimed,
                slot,
                cx,
            ));
        } else {
            actions = actions
                .child(button(id, core_button.label, ButtonVariant::Secondary, cx).disabled(true));
        }
    }
    div()
        .flex()
        .gap(px(8.))
        .items_center()
        .p(px(8.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .child(line.name)
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.secondary)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(line.path),
                ),
        )
        .child(actions)
}

fn profile_block(line: crate::emulators::ProfileLine, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .p(px(8.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .child(line.label),
        )
        .child(emulator_action(
            format!("emulator-delete-{}", line.index),
            "Delete".to_string(),
            ButtonVariant::Danger,
            line.aimed,
            Slot::Delete(line.index),
            cx,
        ))
}

fn console_block(line: crate::emulators::ConsoleLine, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .p(px(8.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(div().flex_1().child(line.name))
        .child(emulator_action(
            format!("emulator-console-{}", line.index),
            line.value,
            ButtonVariant::Secondary,
            line.aimed,
            Slot::Console(line.index),
            cx,
        ))
}

fn kind_block(kind: Kind, aimed: bool, cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.omarchy();
    div()
        .id("emulator-kind")
        .flex()
        .border_1()
        .border_color(if aimed { theme.accent } else { theme.border })
        .child(kind_segment(Kind::Standalone, kind, false, cx))
        .child(kind_segment(Kind::RetroArch, kind, true, cx))
}

fn kind_segment(
    kind: Kind,
    selected: Kind,
    divider: bool,
    cx: &Context<Shell>,
) -> impl IntoElement {
    let theme = cx.omarchy();
    let on = kind == selected;
    let mut segment = div()
        .id(match kind {
            Kind::Standalone => "emulator-kind-standalone",
            Kind::RetroArch => "emulator-kind-retroarch",
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

fn emulator_field(
    id: &'static str,
    edit: &crate::game_menu::LineEdit,
    focused: bool,
    placeholder: &'static str,
    slot: Slot,
    cx: &Context<Shell>,
) -> impl IntoElement {
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
        .flex_1()
        .min_w(px(0.))
        .px(px(8.))
        .py(px(6.))
        .border_1()
        .border_color(if focused { theme.accent } else { theme.border })
        .bg(theme.surface)
        .on_click(
            cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
                if let Some(dialog) = &mut this.emulators {
                    dialog.aim(slot);
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
        .child(text)
}

fn emulator_action(
    id: String,
    label: String,
    variant: ButtonVariant,
    aimed: bool,
    slot: Slot,
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
                let aimed = this
                    .emulators
                    .as_mut()
                    .is_some_and(|dialog| dialog.aim(slot));
                if aimed {
                    this.confirm_emulators(cx);
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            },
        )))
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
