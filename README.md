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

- Arrow keys move the focused pane. Right, or Tab, enters the grid. Left on the first column of a row returns to the console list.
- `h` `j` `k` `l` step the grid.
- Enter launches the selected game with the existing profile resolver. The status line names the error when the system has no profile. This shell does not write play time back to the database.
- `d` shows or hides the details pane.
- Escape clears the selected game.
- `-` and `+` (or `=`, and the numpad equivalents) change the game-grid cover width by 10px. The same width is used for every system and saved as `cover_width` in the config. The range is 120–400. The status bar slider does the same thing.

## Not in this branch

Import, scrape, rename, delete, emulator setup, the favorites filter, the LaunchBox theme toggle, and gamepad hold-repeat stay in the GTK app on `main`. `src/ui.rs` and `src/dialogs.rs` are that window. This binary does not compile them.

Scan, config, the SQLite library, and launch command building are the library crate. `cargo test` runs those tests. The binary is the shell.

## Data

- Config: `~/.config/retromarchy/config.toml`
- Library: `~/.local/share/retromarchy/library.db`
- Scraped artwork: `~/.local/share/retromarchy/media/`

`config.example.toml` is a commented sample of the config file. The first run still creates a default config when the file is missing.

This repository does not include a `LICENSE` file. `resources/systems.json` is derived from EmulationStation Desktop Edition. See `ATTRIBUTION.md`.
