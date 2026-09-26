# Retromarchy

A LaunchBox-style retro game launcher for Omarchy (Arch + Hyprland).

This branch replaces the GTK4 window with a GPUI shell that uses [gpui-omarchy](https://github.com/huacnlee/gpui-omarchy). The shell reads the same config and library as the GTK app. To run the GTK app again, check out `main`.

```bash
git checkout main
```

## Run

```bash
cargo run --bin retromarchy
```

The window id is `org.omarchy.Retromarchy`, with server-side decorations, so Hyprland can tile it.

`gpui_omarchy::init` reads the current Omarchy theme from `~/.local/state/omarchy/current` (legacy `~/.config/omarchy/current`). If that theme is missing or invalid, the shell uses Tokyo Night. The theme name is in the header.

A normal Arch desktop already has the libraries GPUI needs (a Vulkan driver, fontconfig, and libxkbcommon). This branch does not link GTK or libadwaita.

## What you see

The window has three panes.

1. The left pane lists consoles from `~/.config/retromarchy/config.toml`.
2. The center pane is a box-art grid for the selected console. A tile uses the console's `grid_art` file when that file exists, then the other scraped image. Otherwise it shows initials.
3. The right pane shows console stats when no game is selected. Select a game and it shows the title, console, play stats, ROM path, CRC32, box art, and screenshot.

If the config has no consoles, the shell shows an in-memory demo and labels it **Demo library**. The demo does not write games into your library. Placeholder art for one Super Nintendo row lives in `resources/demo/`. Those files are not ROMs.

## Keys

- Arrow keys, the d-pad, and the left stick move the focused pane. Right, or Tab, or A (South) enters the grid. Left on the first column of a row returns to the console list and clears the game. B (East) returns to the console list and keeps the selected game.
- Holding a direction repeats on one clock. The first step is immediate. The next waits `initial_delay_ms`, then the gap eases from `slow_interval_ms` to `fast_interval_ms` over `ramp_ms`. A held arrow wins over the pad on that axis. A, B, and Y do not repeat. The shell reads `[input]` from the config at startup, the same keys GTK Options → Input writes. Missing fields use 400, 180, 50, and 2000.
- `h` `j` `k` `l` step the grid. They are not on the hold-repeat clock.
- Enter, or A on a selected game, launches it with the existing profile resolver. The status line names the error when the system has no profile. This shell does not write play time back to the database.
- `d` shows or hides the details pane.
- `f` toggles a favorite on the selected game. Gamepad Y (North) does the same. The header has an All / Favorites control for the current console. A filled heart on the card and in the details pane means favorited. Disk libraries write the flag through the SQLite library; the demo library keeps it in memory until you quit.
- The Menu key, Shift+F10, or Select (Back / View) opens the game menu on the selected game. Right-click a card does the same. The rows are Scrape, Rename, and Delete. Arrows or `j` / `k` move, Enter or A chooses, Escape or B closes. A click outside the menu closes it.
- `s`, or the header Scrape button, opens that same scrape dialog for the selected game. Shift+S, or Scrape Missing, scrapes every game on the current system. The scraper fetches box art and screenshot from ScreenScraper, then TheGamesDB, and skips a kind whose file is already on disk. Favorites do not narrow the system list. Progress is on the status line. The selected game and the scroll position stay put. Credentials come from the config. The demo library does not scrape.
- The game-menu Scrape row searches by name, then saves box art and screenshot for that one game. Rename writes the display title (`title` and `title_custom`) and leaves the ROM file alone. Delete asks before removing the library row; the ROM and cached artwork stay unless those boxes are checked. The demo library opens the same dialogs and does not write them.
- Escape clears the selected game when the menu is closed. B does not.
- `-` and `+` (or `=`, and the numpad equivalents) change the game-grid cover width by 10px. The same width is used for every system and saved as `cover_width` in the config. The range is 120–400. The status bar slider does the same thing.

## Not in this branch

Import, emulator setup, and the LaunchBox theme toggle stay in the GTK app on `main`. This shell reads the d-pad, the left stick, A, B, Y, and Select. `src/ui.rs` and `src/dialogs.rs` are that window. This binary does not compile them.

Scan, config, the SQLite library, and launch command building are the library crate. `cargo test` runs those tests. The binary is the shell.

## Data

- Config: `~/.config/retromarchy/config.toml`
- Library: `~/.local/share/retromarchy/library.db`
- Scraped artwork: `~/.local/share/retromarchy/media/`

`config.example.toml` is a commented sample of the config file. The first run still creates a default config when the file is missing.

This repository does not include a `LICENSE` file. `resources/systems.json` is derived from EmulationStation Desktop Edition. See `ATTRIBUTION.md`.
