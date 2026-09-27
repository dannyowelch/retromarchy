# Retromarchy

A LaunchBox-style retro game launcher for Omarchy (Arch + Hyprland). The window is a GPUI shell built on [gpui-omarchy](https://github.com/huacnlee/gpui-omarchy).

## Prerequisites

Install Rust, a C compiler, and pkg-config (`base-devel` on Arch). Then install the libraries the GPUI shell links or loads:

```bash
sudo pacman -S base-devel fontconfig libxkbcommon libxkbcommon-x11 libxcb wayland vulkan-icd-loader systemd-libs
```

You also need a Vulkan driver for the GPU. On Omarchy that is `vulkan-radeon`, `vulkan-intel`, or the NVIDIA Vulkan package.

`fontconfig` is required to compile. Its package depends on `freetype2`. `libxkbcommon` and `libxkbcommon-x11` are linked for keyboard input, and `libxcb` is linked for the X11 backend. That backend is compiled in beside Wayland. `wayland` is not linked. The Wayland backend loads `libwayland-client.so.0` when the session is Wayland, which is Hyprland on Omarchy. `vulkan-icd-loader` provides `libvulkan.so.1`, which the renderer loads at run time. `systemd-libs` provides libudev, which gamepad input links.

## Build and run

```bash
cargo run --bin retromarchy
```

The window id is `org.omarchy.Retromarchy`, with server-side decorations, so Hyprland can tile it.

`gpui_omarchy::init` reads the current Omarchy theme from `~/.local/state/omarchy/current` (legacy `~/.config/omarchy/current`). If that theme is missing or invalid, the shell uses Tokyo Night. The theme name is in the header.

## What you see

The window has three panes.

1. The left pane lists consoles from `~/.config/retromarchy/config.toml`.
2. The center pane is a game grid for the selected console. The status-bar **Art** control (or `1` / `2`) picks that console's `grid_art`: box art or screenshot. A tile uses that file when it exists, then the other image. Otherwise it shows initials. Card height follows the slot (3:4 box, 4:3 screenshot). The console name sits under the title.
3. The right pane shows console stats when no game is selected. Select a game and it shows the title, console, play stats, ROM path, CRC32, box art, and screenshot.

If the config has no consoles, the center pane is empty and offers **Import ROMs**. That is the first-run screen. The in-memory demo still appears when the config file or the library database cannot be opened, and it is labeled **Demo library**. The demo does not write games into your library. Placeholder art for one Super Nintendo row lives in `resources/demo/`. Those files are not ROMs.

## Keys

- Arrow keys, the d-pad, and the left stick move the focused pane. Right, or Tab, or A (South) enters the grid. Left on the first column of a row returns to the console list and clears the game. B (East) returns to the console list and keeps the selected game.
- Holding a direction repeats on one clock. The first step is immediate. The next waits `initial_delay_ms`, then the gap eases from `slow_interval_ms` to `fast_interval_ms` over `ramp_ms`. A held arrow wins over the pad on that axis. A, B, and Y do not repeat. Missing fields use 400, 180, 50, and 2000. The header **Options** button edits those four values and writes them to `[input]`. Each step is 10 ms. Starting pause and the transition may be 0. The two repeat intervals may not. Nothing goes past 60 seconds. A faster repeat is pulled down when it would outrun the slower one. The file is written as you change a value, and hold-repeat uses it immediately. Arrows and Tab move. Left and right step. Esc closes. Gamepad B closes. A closes when Close is focused.
- `h` `j` `k` `l` step the grid. They are not on the hold-repeat clock.
- Enter, numpad Enter, or A on a selected game launches it with that system's emulator. The status line names the error when the system has no emulator, or when RetroArch has no core. When the process exits, play count, play time, and last played are written and the details pane refreshes. The wait and the database write stay off the UI thread.
- `/` opens a title filter. It composes with All / Favorites. Esc clears and closes it. `/` again does the same. Arrows and Enter still move and launch while it is open.
- `r` rescans the current system's ROM folders, upserts rows, and drops games whose files are gone. Favorites, custom titles, and play stats stay. The scan runs off the UI thread.
- `d` shows or hides the details pane.
- `f` toggles a favorite on the selected game. Gamepad Y (North) does the same. The header has an All / Favorites control for the current console. A filled heart on the card and in the details pane means favorited. Disk libraries write the flag through the SQLite library; the demo library keeps it in memory until you quit.
- The Menu key, Shift+F10, or Select (Back / View) opens the game menu on the selected game. Right-click a card does the same. The rows are Scrape, Rename, and Delete. Arrows or `j` / `k` move, Enter or A chooses, Escape or B closes. A click outside the menu closes it.
- `s`, or the header Scrape button, opens that same scrape dialog for the selected game. Shift+S, or Scrape Missing, scrapes every game on the current system. The scraper fetches box art and screenshot from ScreenScraper, then TheGamesDB, and skips a kind whose file is already on disk. Favorites do not narrow the system list. Progress is on the status line. The selected game and the scroll position stay put. Credentials come from the config. The demo library does not scrape.
- Ctrl+G, or the header **Scraper** button, opens Scraper settings. Box art and screenshot, the provider list (enable, Up, Down), a ScreenScraper username and password, and a TheGamesDB API key. The password and API key are masked. Save writes `[scraper]` and leaves the rest of `config.toml` alone. Esc closes without saving. The next scrape reads the file, so a restart is not required. Arrows and Tab move. Enter toggles a check, moves a provider, or saves when Save is focused. Gamepad A confirms and B closes.
- The game-menu Scrape row searches by name, then saves box art and screenshot for that one game. Rename writes the display title (`title` and `title_custom`) and leaves the ROM file alone. Delete asks before removing the library row; the ROM and cached artwork stay unless those boxes are checked. The demo library opens the same dialogs and does not write them.
- Escape clears the title filter when it is open. Otherwise it clears the selected game when the menu is closed. B does not.
- `1` and `2` set the current system's grid art to box art or screenshot. The status-bar **Art** control does the same with the mouse. The choice is that console's `grid_art` in `config.toml`. A missing file falls back to the other image, and the card height changes with the slot. A demo library updates the grid and does not write the file.
- `-` and `+` (or `=`, and the numpad equivalents) change the game-grid cover width by 10px. The same width is used for every system and saved as `cover_width` in the config. The range is 120 to 400. The status bar slider does the same thing.
- Ctrl+I opens the ROM import wizard. The header **Import ROMs** button does the same, and so does the button on an empty library. The steps are an ES-DE or EmulationStation root, or one system. Arrows, Tab, and Enter move. Esc closes. Gamepad A confirms and B closes. Scan runs off the UI thread. **Browse…** uses GPUI's folder prompt (`prompt_for_paths`, the XDG desktop portal). The path field is always there, prefilled with `~/ROMs`, when the portal is unavailable.
- Ctrl+M or Ctrl+E opens Manage Emulators. The header button and the empty-library button do the same. The dialog has two tabs. **Emulators** adds, edits, and deletes installed emulators: name, kind (RetroArch or Standalone), executable path, and global arguments. RetroArch is one entry. **Systems** is one row per library system: an emulator, a core (only when that emulator is RetroArch, from cores already on disk), and extra arguments appended to the global ones. Tab, 1, and 2 switch tabs. Arrows move. Left and right change a choice. Enter confirms. Esc closes. Gamepad A confirms and B closes. Changes are written to `config.toml` as you confirm them. Launching a game uses the new assignment without restarting.
- `t` toggles the theme, and so does the header **Theme** button. `system` follows the Omarchy theme. `launchbox` is the built-in dark palette. The choice is saved as `theme`.

## Library code

The import wizard does not assign emulators. The shell reads the d-pad, the left stick, A, B, Y, and Select.

Scan, config, the SQLite library, and launch command building live in the library crate. `cargo test` runs those tests. The `retromarchy` binary is the shell.

## Data

An emulator is `id`, `name`, `kind` (`RetroArch` or `Standalone`), `path`, and `global_args`. Each console stores `emulator` (that id), `core` (a libretro `.so`, only for RetroArch), and `extra_args`. The launch command is the executable, then `global_args`, then `-L <core>` for RetroArch, then `extra_args`, then the ROM. `{rom}` in either argument string is replaced and the ROM is not appended again. An old `[[profiles]]` file is rewritten on load: RetroArch cores become one RetroArch emulator plus a core on each console that used them, and a standalone command becomes an emulator.

- Config: `~/.config/retromarchy/config.toml`
- Library: `~/.local/share/retromarchy/library.db`
- Scraped artwork: `~/.local/share/retromarchy/media/`

`config.example.toml` is a commented sample of the config file. The first run still creates a default config when the file is missing.

This repository does not include a `LICENSE` file. `resources/systems.json` is derived from EmulationStation Desktop Edition. See `ATTRIBUTION.md`.
