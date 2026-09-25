# Retromarchy

A LaunchBox-style retro game launcher for Omarchy Linux (Arch + Hyprland), built with Rust, GTK4, and libadwaita.

## Features

- Console-based game library organization
- ROM scanning with CRC32 checksums
- Local media support (box art, screenshots, manuals, videos)
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

[consoles.media]
box_art = true
screenshot = true
manual = false
video = false
```

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
- **Tab**: Switch between sidebar and grid (GTK default)

## Database

Games and media metadata are stored in `~/.local/share/retromarchy/library.db` (SQLite).

## License

See LICENSE file for details.
