# Retromarchy

A LaunchBox-style retro game launcher for Omarchy (Arch + Hyprland), built with Rust, GTK4, and libadwaita.

The window is keyboard-first: a console sidebar on the left, a game grid in the center, and a collapsible details pane on the right. With nothing selected, the pane shows console info (bundled manufacturer, year, and description when that system is in `console_metadata.toml`, plus library stats). With a game selected, it shows box art, title screen, and screenshot when those files exist, plus title, console, ROM path, CRC32, a **Play** button, and play stats.

The default look follows the Omarchy / libadwaita system theme. A header button (tooltip **Toggle Theme**, or `t`) switches to an optional LaunchBox-like dark theme. The choice is saved as `theme = "system"` or `theme = "launchbox"`.

## Features

- First run writes `~/.config/retromarchy/config.toml` with defaults and no consoles or profiles. An empty library shows **No games found** with **Import ROMs** and **Manage Emulators**.
- ROM import stores folder paths and scans them. Files are not copied. Import does not scrape artwork.
- ES-DE / EmulationStation multi-system import (immediate subfolders matched to systems) and one-system import.
- Emulator profiles: RetroArch (libretro core path, optional RetroArch config file) and standalone (`{rom}` in the command).
- When `retroarch` is on `PATH`, **Manage Emulators** lists local cores and can add a profile or add-and-assign it to a known system. You can still type a path or **Browse…** for a `.so`. Cores are not downloaded.
- Launch uses the game’s profile, or the console’s default profile. After the process exits, last played, play count, and play time are updated.
- ROM scan computes CRC32 and discovers local sidecars (box art, screenshot, manual, video) when those toggles are on.
- Manual artwork scrape only: box art, title screen, and screenshot (one file per kind). Providers are ScreenScraper, then TheGamesDB, in that order unless you change it. The scraper does not download ROMs or BIOS.

## Build (Arch)

```bash
sudo pacman -S rust gtk4 libadwaita
cargo build --release
```

`rust-toolchain.toml` selects the stable toolchain.

## Run

```bash
cargo run --release
```

Or install the binary:

```bash
cargo install --path .
retromarchy
```

## Getting started

You do not need to copy `config.example.toml` first. Start the app; a missing config file is created with defaults (no consoles, no profiles, scraper kinds on, ScreenScraper then TheGamesDB).

1. **Import ROMs** (header button or the empty-state button, or Ctrl+I).
   - **Import ES-DE / EmulationStation library** — point at a ROMs root (the dialog suggests `~/ROMs`). Immediate subfolders are matched to the bundled system catalog. **Scan folders**, check the systems you want, remap unmatched folders, then **Import**.
   - **Add one system** — search the catalog, **Choose folder**, **Browse…**, **Add system**.
   Paths are stored. ROM files are not copied. Nothing is scraped.
2. **Manage Emulators** (header, empty state, Ctrl+E, or Ctrl+M).
   - If `retroarch` is on `PATH`, **Discovered cores** lists `.so` files from `/usr/lib/libretro`, `/usr/lib/libretro/cores`, `/usr/share/libretro/cores`, `~/.config/retroarch/cores`, and `libretro_directory` in `~/.config/retroarch/retroarch.cfg`. **Add profile** saves a RetroArch profile. **Add & assign to …** also sets that system’s default profile. Cores are not downloaded.
   - Or add a profile by hand: **Standalone** with a command that contains `{rom}`, or **RetroArch** with a core path (**Browse…** filters to `*.so`).
   - **Default profile per system** assigns the profile used at launch. Without one, Play / Enter says to configure an emulator.
3. Select a console in the sidebar, then a game. **Play** or Enter launches it.
4. Artwork is optional and manual. **Scraper** (Ctrl+G) stores credentials. **Scrape** (`s`) fetches missing artwork for the selected game. **Scrape Missing** (Shift+S) does the same for every game on the current system. Kinds that already have a file are skipped.

The header combo (tooltip **Grid artwork for this system**) chooses what the grid prefers for the current console: **Box art**, **Title screen**, or **Screenshot** (`1` / `2` / `3`). If that file is missing, the grid falls back to the other two. `/` opens the filter (**Filter games...**). Escape closes the filter, or clears the game selection and returns the details pane to console info.

## Configuration

The app reads and writes `~/.config/retromarchy/config.toml`. The dialogs above are the normal way to change it. The same file is hand-editable; `config.example.toml` is a commented sample of the on-disk shape, not a required setup step.

Top-level keys the app uses:

- `theme`: `"system"` (default) or `"launchbox"`.
- `details_visible`: whether the right pane starts open (default `true`).
- `profiles`, `consoles`, `scraper`.

**RetroArch profile** (launch is `retroarch -L <core> [--config <file>] <rom>`):

```toml
[[profiles]]
type = "RetroArch"
id = "retroarch-snes9x"
core = "/usr/lib/libretro/snes9x_libretro.so"
# config = "/home/user/.config/retroarch/snes.cfg"  # optional
```

Profiles created in **Manage Emulators** omit `config`.

**Standalone profile** (`{rom}` is replaced, then the string is split on shell words):

```toml
[[profiles]]
type = "Standalone"
id = "dolphin"
command = "dolphin-emu -b -e {rom}"
```

**Console** (`grid_art` is `box_art`, `title_screen`, or `screenshot`; default `box_art`). `media` only controls local sidecar discovery, not the scraper:

```toml
[[consoles]]
id = "snes"
name = "Super Nintendo"
rom_dirs = ["/home/user/ROMs/snes"]
extensions = ["sfc", "smc", "zip"]
profile = "retroarch-snes9x"
grid_art = "box_art"

[consoles.media]
box_art = true
screenshot = true
manual = true
video = true
```

Systems added by import use the catalog id, display name, and extensions, with all four media toggles on and `grid_art` at the default.

### Local media discovery

On scan, next to the ROM or in a subdirectory of the ROM’s folder (first match wins):

- **Box art**: `<rom_stem>.png`, `.jpg`, or `.jpeg`, or `box_art/<rom_stem>.<ext>`
- **Screenshot**: same image extensions, or `screenshot/`
- **Manual**: `<rom_stem>.pdf` or `.txt`, or `manual/`
- **Video**: `<rom_stem>.mp4`, `.mkv`, or `.avi`, or `video/`

Title screens are not picked up as local sidecars. They come from the scraper. Manuals and videos are local only; the scraper does not fetch them.

## Scraper

Open **Scraper** in the header or press Ctrl+G (**Scraper settings**). It saves the `[scraper]` table. Scrapes run only from **Scrape** / `s` and **Scrape Missing** / Shift+S. Import and rescan do not call the network.

Enabled kinds are box art, title screen, and screenshot. Each missing kind walks the provider list and stops at the first hit. Default order is ScreenScraper, then TheGamesDB. Reorder with **Up** / **Down**, or edit the list.

```toml
[scraper]
box_art = true
title_screen = true
screenshot = true

[[scraper.providers]]
id = "screenscraper"
enabled = true

[[scraper.providers]]
id = "thegamesdb"
enabled = true

[scraper.credentials]
screenscraper_user = ""
screenscraper_password = ""
screenscraper_dev_id = ""
screenscraper_dev_password = ""
thegamesdb_api_key = ""
```

ScreenScraper calls need all four ScreenScraper fields: username, password, developer id (`devid`), and developer password (`devpassword`). TheGamesDB needs only `thegamesdb_api_key`, so it works without ScreenScraper developer credentials. Credentials stay in this file; they are not built into the binary.

Images are written to `~/.local/share/retromarchy/media/<console>/<game-id>/<kind>.<ext>` (`box_art`, `title_screen`, or `screenshot`; png, jpg, gif, or webp). One file per kind. The `media` table in the library database records the path. Archive, ROM, BIOS, manual, and video URLs are rejected.

## Keybindings

Ignored while the filter entry is focused, except Escape, `/`, and Tab.

- **Arrow keys / h j k l**: Move in the grid (up and down jump six tiles)
- **Enter**: Launch the selected game
- **r**: Rescan the current console
- **s**: Scrape artwork for the selected game
- **Shift+S**: Scrape missing artwork for the current system
- **Ctrl+G**: Scraper settings
- **Ctrl+I**: Import ROMs
- **Ctrl+E** or **Ctrl+M**: Manage Emulators
- **1 / 2 / 3**: Grid artwork for this system (box art, title screen, screenshot)
- **/**: Toggle the game filter
- **Escape**: Close the filter, or clear the selection
- **d**: Show or hide the details pane
- **t**: Toggle system theme and the LaunchBox dark theme
- **Tab**: Focus the grid and select the first game if none is selected

## Data locations

- Config: `~/.config/retromarchy/config.toml`
- Library: `~/.local/share/retromarchy/library.db` (SQLite)
- Scraped artwork: `~/.local/share/retromarchy/media/`

## License

This repository does not include a `LICENSE` file. `resources/systems.json` is derived from EmulationStation Desktop Edition; see `ATTRIBUTION.md`.
