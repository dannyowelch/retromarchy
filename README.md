# Retromarchy

A LaunchBox-style retro game launcher for Omarchy Linux (Arch + Hyprland), built with Rust, GTK4, and libadwaita.

## Features

- Console-based game library organization
- ROM scanning with CRC32 checksums
- Local media support (box art, screenshots, manuals, videos)
- Artwork scraper for box art, title screens, and screenshots (ScreenScraper and TheGamesDB)
- RetroArch and standalone emulator support
- Keyboard-first navigation optimized for tiling window managers
- System theme integration via libadwaita

## Build Dependencies (Arch Linux)

```bash
sudo pacman -S rust gtk4 libadwaita
```

## Building

```bash
cargo build --release
```

## Running

```bash
cargo run --release
```

Or install the binary:

```bash
cargo install --path .
retromarchy
```

## Configuration

Copy `config.example.toml` to `~/.config/retromarchy/config.toml` and adjust paths to your ROM directories and emulator cores.

### Emulator Profiles

**RetroArch:**
```toml
[[profiles]]
type = "RetroArch"
id = "retroarch-snes9x"
core = "/usr/lib/libretro/snes9x_libretro.so"
config = "/path/to/retroarch.cfg"  # Optional
```

**Standalone:**
```toml
[[profiles]]
type = "Standalone"
id = "dolphin"
command = "dolphin-emu -b -e {rom}"
```

### Console Configuration

```toml
[[consoles]]
id = "snes"
name = "Super Nintendo"
rom_dirs = ["/home/user/roms/snes"]
extensions = ["sfc", "smc", "zip"]
profile = "retroarch-snes9x"
grid_art = "box_art"

[consoles.media]
box_art = true
screenshot = true
manual = false
video = false
```

### Artwork scraper

Global settings live in `config.toml` under `[scraper]`. The app edits the same file from **Scraper** (Ctrl+G). Nothing here downloads ROMs or BIOS.

```toml
[scraper]
box_art = true
title_screen = true
screenshot = true

[[scraper.providers]]
id = "screenscraper"   # or "thegamesdb"
enabled = true

[scraper.credentials]
screenscraper_user = ""
screenscraper_password = ""
screenscraper_dev_id = ""
screenscraper_dev_password = ""
thegamesdb_api_key = ""
```

Provider order is priority: the first enabled provider that returns a kind wins, then that kind stops. Kinds that already have a file are skipped. ScreenScraper’s HTTP API requires a developer id and password in addition to the member username and password.

Scraped images are stored at `~/.local/share/retromarchy/media/<console>/<game-id>/<kind>.<ext>` and recorded in the `media` table of `~/.local/share/retromarchy/library.db`.

### Media Discovery Convention

Retromarchy looks for media files matching the ROM filename:

- **Box art**: `<rom_name>.png/jpg/jpeg` in same directory or `box_art/` subdirectory
- **Screenshots**: `<rom_name>.png/jpg/jpeg` in same directory or `screenshot/` subdirectory
- **Manuals**: `<rom_name>.pdf/txt` in same directory or `manual/` subdirectory
- **Videos**: `<rom_name>.mp4/mkv/avi` in same directory or `video/` subdirectory

## Keybindings

- **Arrow keys / hjkl**: Navigate game grid
- **Enter**: Launch selected game
- **r**: Rescan current console for ROMs
- **s**: Scrape artwork for the selected game
- **Shift+S**: Scrape missing artwork for the current system
- **Ctrl+G**: Scraper settings
- **1 / 2 / 3**: Grid artwork for this system (box art, title screen, screenshot)
- **Tab**: Switch between sidebar and grid (GTK default)

## Database

Games and media metadata are stored in `~/.local/share/retromarchy/library.db` (SQLite). Scraped artwork files are stored in `~/.local/share/retromarchy/media/`.

## License

See LICENSE file for details.
