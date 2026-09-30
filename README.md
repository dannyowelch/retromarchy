# Retromarchy

A LaunchBox-style retro game launcher for Omarchy (Arch + Hyprland). The window is a GPUI shell built on [gpui-omarchy](https://github.com/huacnlee/gpui-omarchy).

## Install

Releases are x86_64. When `/etc/os-release` has `ID=arch` or `ID_LIKE` contains `arch`, and `pacman` is installed, the installer downloads the latest GitHub Release, checks `SHA256SUMS`, and runs `pacman -U`. Anywhere else it checks the same sums and unpacks the tarball under `~/.local`. It rewrites `Exec` to `~/.local/bin/retromarchy`, so the desktop entry does not depend on the working directory or on `~/.local/bin` being on `PATH`.

```bash
curl -fsSL https://raw.githubusercontent.com/dannyowelch/retromarchy/main/install.sh | bash
```

`curl` is required. Arch package install uses `sudo` when you are not already root. RetroArch is optional. A Vulkan driver is still required to open the window: `vulkan-radeon`, `vulkan-intel`, or `nvidia-utils`.

The Arch package also depends on `ca-certificates`, `fontconfig`, `freetype2`, `libglvnd`, `libxcb`, `libxkbcommon`, `libxkbcommon-x11`, `systemd-libs`, `vulkan-icd-loader`, and `wayland`. `pacman -U` installs those. The tarball does not, so install them yourself on other distros.

### Arch package

From the [Releases](https://github.com/dannyowelch/retromarchy/releases) page, after checking `SHA256SUMS` from the same release. `0.1.0` is the version in `Cargo.toml`:

```bash
ver=0.1.0
curl -fLO "https://github.com/dannyowelch/retromarchy/releases/download/v${ver}/retromarchy-${ver}-1-x86_64.pkg.tar.zst"
curl -fLO "https://github.com/dannyowelch/retromarchy/releases/download/v${ver}/SHA256SUMS"
sha256sum -c --ignore-missing SHA256SUMS
sudo pacman -U "retromarchy-${ver}-1-x86_64.pkg.tar.zst"
```

The package installs `/usr/bin/retromarchy`, the desktop file, and the icon.

### Tarball

This copy of the desktop file keeps `Exec=retromarchy`, so `~/.local/bin` has to be on `PATH`:

```bash
ver=0.1.0
curl -fLO "https://github.com/dannyowelch/retromarchy/releases/download/v${ver}/retromarchy-${ver}-x86_64-linux.tar.gz"
curl -fLO "https://github.com/dannyowelch/retromarchy/releases/download/v${ver}/SHA256SUMS"
sha256sum -c --ignore-missing SHA256SUMS
tar -xzf "retromarchy-${ver}-x86_64-linux.tar.gz"
install -Dm755 "retromarchy-${ver}-x86_64-linux/retromarchy" ~/.local/bin/retromarchy
install -Dm644 "retromarchy-${ver}-x86_64-linux/retromarchy.desktop" ~/.local/share/applications/retromarchy.desktop
install -Dm644 "retromarchy-${ver}-x86_64-linux/retromarchy.png" ~/.local/share/icons/hicolor/256x256/apps/retromarchy.png
```

### Uninstall

On Arch, this removes the pacman package when it is installed (`pacman -Rns`). It always removes the `~/.local` binary, desktop file, and icon. Config, the library database, scraped artwork, and the thumbnail cache are left in place.

```bash
curl -fsSL https://raw.githubusercontent.com/dannyowelch/retromarchy/main/install.sh | bash -s -- uninstall
```

### Config and data

The first launch creates a default config when the file is missing. Paths come from the XDG base directories, not from the directory the app was started in. `~` and `~/...` in a ROM folder expand with `HOME`.

- Config: `~/.config/retromarchy/config.toml` (`$XDG_CONFIG_HOME/retromarchy/config.toml`)
- Library: `~/.local/share/retromarchy/library.db` (`$XDG_DATA_HOME/retromarchy/library.db`)
- Scraped artwork: `~/.local/share/retromarchy/media/`
- Card thumbnails: `~/.cache/retromarchy/thumbs` (`$XDG_CACHE_HOME/retromarchy/thumbs`)

`config.example.toml` is a commented sample. The file format is in [Data](#data).

### Build from source

The release workflow builds in `archlinux:base-devel` with these extra packages:

```bash
sudo pacman -S --needed base-devel clang fontconfig freetype2 libxcb libxkbcommon libxkbcommon-x11 pkgconf rust systemd-libs wayland
```

To run the binary you also need the runtime libraries listed above (`libglvnd`, `vulkan-icd-loader`, `ca-certificates`, and a Vulkan driver).

```bash
cargo run --bin retromarchy
```

The window app id is `org.omarchy.Retromarchy`, with server-side decorations, so Hyprland can tile it. The default size is 1280×800.

`gpui_omarchy::init` reads `$HOME/.local/state/omarchy/current/theme/colors.toml`. If that `current` directory is absent, it uses `$HOME/.config/omarchy/current`. A missing home directory, or a current theme that is unreadable or invalid, uses Tokyo Night. An existing but broken state directory does not fall back to the legacy path.

`theme = "launchbox"` is a separate built-in dark palette in this app. It does not follow Omarchy until you switch back to `system`.

## Window

Three panes, plus a header and a status bar.

1. The left pane lists systems from the config. Each row shows the name, launch year (when known), and game count. The **Name** / **Year** control, or `o`, sets the order. Year order uses the launch year in `console_metadata.toml` (`[launch_year]`, otherwise the matching `[[console]]` year). Unknown years sort last. Ties use the name.
2. The center pane is a virtualized grid (`uniform_list` builds the visible rows). The status-bar **Art** control, or `1` / `2`, sets that system's `grid_art` to box art or screenshot. A card uses that file when it exists, then the other image, otherwise initials. Every card in the grid uses one slot: 3:4 for box art, 4:3 for screenshot, at the shared cover width. The system name sits under the title. A filled heart marks a favorite.
3. The right pane is the details pane (`d` shows or hides it for this session). With no game selected it shows the system's manufacturer, year, and description when `console_metadata.toml` has them, then library stats: game count, last played, total plays, total play time, and most played. A selected game shows **Play**, a heart toggle, box art, screenshot, title, system, publisher, year, genre, last played, play count, play time, ROM path, and CRC32. Empty fields are omitted.

The header has **Manage Emulators**, **Manage Systems**, and **Options**, then an **All** / **Favorites** filter for the grid. While the title filter is closed, the header also shows the shortcut hints.

If the config has no systems, the library is empty and the center pane offers **Manage Systems** and **Manage Emulators**. That is the first-run screen. The in-memory demo appears when the config cannot be read, or when the config has systems but the library database cannot be opened. It is labeled **Demo library** and does not write games into your library. Compiled-in placeholder box art and a screenshot (`resources/demo/snes-box.png`, `resources/demo/snes-shot.png`) are attached to Super Mario World and Sonic the Hedgehog and written to `~/.cache/retromarchy/demo/` (`$XDG_CACHE_HOME/retromarchy/demo/`, or a temp directory if that cache cannot be created). Those files are not ROMs.

The empty status line reads `Ctrl+E emulators. Ctrl+P systems. Ctrl+O options.`

## Keyboard

Shortcuts below ignore Ctrl, Alt, and the platform key unless the shortcut is a Ctrl chord. Shift blocks the single-letter shortcuts except where noted.

- Arrow keys move the focused pane. They repeat on one clock: the first step is immediate, the next waits `initial_delay_ms`, then the gap eases from `slow_interval_ms` to `fast_interval_ms` over `ramp_ms`. Missing fields use 400, 180, 50, and 2000. A held arrow wins over the gamepad on that axis.
- Right, Tab, or A from the system list enters the grid. Enter does not. Left on the first column of a row returns to the system list and clears the game. Up on the first system moves into the header: **Manage Emulators**, **Manage Systems**, then **Options**. Left and Right move along that row. Enter or A opens the focused header item. Down returns to the first system (the selected game is cleared if you were not already on that system). Escape or East leaves the header and keeps the selection. Up on the top row of the grid stays on that game. `h` `j` `k` `l` step the grid only, and only while the title filter is closed. They are not on the hold-repeat clock, and `h` on the first column does not return to the system list.
- Enter or numpad Enter launches the selected game. That includes the system list, when a game is still selected after East or B. With no game selected, the status line says to select one. The status line names the error when the system has no emulator, or when RetroArch has no core. When the game exits, play count, play time, and last played are written and the details pane refreshes. The wait and the database write stay off the UI thread. A Flatpak launch is waited out until the game process is gone, not only until `flatpak run` returns. The demo library has no emulators, so launch stops with the missing-emulator message.
- `/` opens a title filter for the system on screen. It composes with All / Favorites. Shift+`/` is not the shortcut. Esc clears and closes it. `/` again does the same. Backspace deletes a character. Arrows, Tab, and Enter still move and launch while it is open. Other unmodified keys, including `1`, `2`, `h` `j` `k` `l`, and the letter shortcuts, are typed into the filter. Ctrl chords still open their screens.
- `r` rescans every ROM folder on the current system. New ROMs are added. Games whose files are gone are dropped. Favorites, play stats, user titles, and scraped metadata stay for games that are still there. The same scan runs from **Rescan** on Manage Systems, and when a ROM folder is added or removed there. It runs off the UI thread. A scan that adds games scrapes those new games only, using the scraper settings already in config. A scan that adds nothing does not scrape. The selected game stays selected when it is still in the list.
- `d` shows or hides the details pane for this session. `details_visible` in the config is only the value used at startup (default true).
- `f` toggles a favorite on the selected game. The header **All** / **Favorites** control filters the current system's grid and is not saved. A filled heart on the card and in the details pane means favorited. Unfavoriting the selected game while Favorites is on clears the selection. Disk libraries write the flag through SQLite. The demo library keeps it in memory until you quit.
- The Menu key, Shift+F10, or right-click opens the game menu on the selected game. The rows are **Scrape…**, **Rename…**, and **Delete…**. Arrows move (and repeat). `j` and `k` move one step. Enter chooses. Escape closes. A click outside the menu closes it. Rename writes `user_title` and clears `title_custom`. A blank title shows the scraped title, then the file name. It does not rename the ROM. Delete removes the library row after a confirm. Cancel is the default. **Delete ROM file from disk** and **Delete scraped assets** start unchecked. Scraped assets are that game's directory under `media/`. The demo library opens the same dialogs and does not write them.
- `s` opens the scrape dialog for the selected game. Shift+S, or an uppercase `S`, scrapes the current system for missing artwork and metadata. See [Scraping](#scraping).
- `1` and `2` set the current system's grid art to box art or screenshot. Numpad `1` / `2` and `digit1` / `digit2` count. `3` does not. The status-bar **Art** control does the same. The choice is that console's `grid_art`. A missing file falls back to the other image, and the card height changes with the slot. A demo library updates the grid and does not write the file.
- `-` and `+` (also `=`, and the numpad equivalents) change the cover width by 10 px. The same width is used for every system and saved as `cover_width`. The range is 120 to 400. The default is 216. The status-bar **Size** slider shows the same width.
- `o` switches the system list between name and year. Shift+O does not. Ctrl+O opens Options. The sidebar **Name** / **Year** control and Options → Grid do the same. The choice is saved as `system_sort`. A demo library updates the list and does not write the file.
- `t` toggles the theme. Shift+T does not. Options → Theme does the same. `system` follows Omarchy. `launchbox` is the built-in dark palette. The choice is saved as `theme`.
- Escape clears the title filter when it is open. Otherwise it clears the selected game when the header menu is not focused. Gamepad East does not clear the game.
- Ctrl+E or Ctrl+M opens Manage Emulators. Ctrl+P opens Manage Systems. Ctrl+O opens Options on Input. Ctrl+G opens Options on Scraper. The header buttons do the same. The empty-library buttons open those two screens. They are separate screens. Switching from one settings screen to another saves the open screen the way Escape does, then opens the other.

## Gamepad

gilrs, SDL / Xbox layout. Directions are holds. South, East, North, and Select are one-shot. A repeated button event does not fire them again.

- D-pad and left stick move. On an axis, a held d-pad button wins over the stick. The stick engages past 0.55 and stays on until the axis falls inside 0.35. X and Y are independent. Positive left-stick Y is up.
- South (A) confirms: enter the grid, launch, open the focused header item, or activate the focused control.
- East (B) goes back: from the grid or the header, focus returns to the system list and the game index stays. On a settings screen or a dialog, B closes it the way Escape does.
- North (Y) toggles a favorite on the library. It does nothing while the game menu or a dialog is open, and nothing on Manage Emulators, Manage Systems, or Options.
- Select (Back / View) opens the game menu. Select again closes it.
- West and Start are not used.

The same hold-repeat settings drive the pad and the arrow keys. A, B, and Y do not repeat.

While a launched game's process is still running, keyboard and gamepad input is ignored. Losing window focus does the same, which covers a game that is still up after the launcher process has exited. Mouse clicks are not gated. After the block ends, input stays ignored through the next poll and until every button is up and the sticks are centered, so a Select still held from quitting the emulator does not open the menu.

## Manage Emulators

Ctrl+E, Ctrl+M, the header button, or the empty-library button. The list is installed emulators, plus **Add**. The panel edits name, kind (RetroArch or Standalone), the launch command, and global arguments.

Up and down move the list. Right, Enter, Tab, or A enters the panel. Left at a control's left edge, or Shift-Tab, returns to the list. Esc saves and closes. A confirms the focused control. B closes and saves.

Opening the screen, and starting the app, adds any RetroArch install that is not already listed. Native detection stores the absolute path of `retroarch` on `PATH`, or `/usr/bin/retroarch` when it is not on `PATH`. Flatpak detection adds a user install and a system install. A single Flatpak install is `flatpak run org.libretro.RetroArch`. When both Flatpak installs are present the commands are `flatpak run --user org.libretro.RetroArch` and `flatpak run --system org.libretro.RetroArch`. An existing `id = "retroarch"` entry stays, and its path says which install it is. An install that is no longer on the machine stays in the list and is marked not found.

The RetroArch panel shows that install's `retroarch.cfg` (stored as `config` when set, otherwise derived) and the cores from its `libretro_directory`. `r`, or **Rescan cores**, reads that cfg again. `r` does nothing on a standalone emulator, and nothing while a text field is focused. One core per name. A missing or empty cores directory names the path it looked at. A core that is not in that directory stays selected and is labeled missing when a system still points at it.

## Manage Systems

Ctrl+P, the header button, or the empty-library button. The list is library systems, plus **Add system**. Add opens a type-ahead list of `resources/systems.json` (folder id, display name, extensions). Popular systems come first. The rest are alphabetical. Types already in the list are hidden. Typing jumps by a folder-id prefix, then by a word in the name, then by a substring of either. Home, End, Page Up, and Page Down move in that list (a page is 8 rows). Enter adds the type and focuses the path field. Launch year shown next to a type comes from `console_metadata.toml`, not from `systems.json`.

The panel sets that system's emulator (or None), its core when the emulator is RetroArch, extra arguments appended to the global ones, and one or more ROM folders. The core list is that emulator's install. Cores whose info matches the system name, the folder id, or a non-generic extension are listed first, then the rest, each group by label. A saved core path outside that directory is repointed to the core with the same name when one is there, and not to another install. A core that is not in the directory stays selected and is labeled missing.

Type a folder and press Enter, or press **Add path**. **Browse…** uses GPUI's folder prompt (`prompt_for_paths`, the XDG desktop portal). `~` and `~/...` expand with `HOME` when the path is stored and again when it is scanned. **Remove** drops a folder from the list. Adding a folder, removing one, or **Rescan** scans that system.

**Scrape Missing** sits with Rescan and Delete system. It scrapes that system's games that are still missing artwork or metadata. While that system's scrape is running the button is disabled and reads Scraping…. A second click does not start another scrape for the same system. One scrape runs at a time. Another system's scrape waits.

**Delete system** asks first. Cancel is the default. Left or Up selects Cancel. Right or Down selects Delete. Enter confirms. Escape or B cancels the prompt and leaves the screen open. ROM files and scraped artwork stay on disk. The system's games and their library metadata (favorites, play stats, renames, scraped rows) are removed from the database.

A system can be added before it has an emulator. Changes are written to `config.toml` as you confirm them, and again when the screen closes. Launching a game uses the new assignment without restarting.

Navigation matches Manage Emulators: list on the left, panel on the right, Esc saves and closes, B does the same.

## Options

Ctrl+O opens Input. Ctrl+G opens Scraper. The header **Options** button opens Input. The left list is Input, Theme, Scraper, and Grid. Up and down move the list. Right, Enter, Tab, or A enters the panel. Left at a control's left edge, or Shift-Tab, returns to the list. Esc saves and closes. B does the same. A confirms the focused control.

- **Input.** The four hold-repeat values, in milliseconds. Each step is 10. Starting pause and the transition may be 0. The two repeat intervals may not. Nothing goes past 60 seconds. A faster repeat is pulled down when it would outrun the slower one. The file is written as you change a value, and hold-repeat uses it immediately.
- **Theme.** System or LaunchBox, saved as you change it.
- **Scraper.** Box art and screenshot checkboxes, the provider list (enable, Up, Down), a ScreenScraper username and password, and a TheGamesDB API key. The password and API key are masked. The ScreenScraper application id (softname) is compiled in. You do not enter it. **Save** writes `[scraper]` and leaves the rest of `config.toml` alone. Esc also writes that draft and closes. If the write fails, Options stays open. The next scrape reads the file, so a restart is not required. The scraper does not download ROMs or BIOS.
- **Grid.** Cover width (10 px steps, 120 to 400) and sidebar order (Name or Year), saved as you change them.

## Scraping

Providers are the list in Options → Scraper, highest priority first. The default is ScreenScraper, then TheGamesDB, both enabled. Each missing artwork type tries that list and stops at the first hit. Metadata uses the same order.

A bulk scrape (Shift+S, **Scrape Missing**, or the scrape started by a scan) skips a game that already has every enabled artwork file and already has metadata. A kind whose file is already on disk is not fetched again. Metadata is fetched when the game has none. Stored fields are title, publisher, year, and genre (`scraped_title`, `publisher`, `year`, `genre`). The display title uses a user title first, then the scraped title, then the file name. The details pane shows publisher, year, and genre when they are present.

The game-menu scrape searches by name, then you pick a match. That replaces box art and screenshot for the enabled kinds and writes metadata for that one game. It does not start while another scrape is running.

Favorites do not narrow which games are scraped. Progress is on the status line. The demo library does not scrape.

A scan's follow-up scrape covers only the games that scan added. If a scrape for that system is already running or waiting, the new games are appended to it.

## Data

An emulator is `id`, `name`, `kind` (`RetroArch` or `Standalone`), `path`, `global_args`, and an optional `config`. `path` is a command. The first token is the program and the rest lead the argument list, so a Flatpak RetroArch entry is `flatpak run org.libretro.RetroArch`. `config` is that install's `retroarch.cfg`. When it is absent, native uses `$XDG_CONFIG_HOME/retroarch/retroarch.cfg` or `~/.config/retroarch/retroarch.cfg`, and Flatpak uses `~/.var/app/org.libretro.RetroArch/config/retroarch/retroarch.cfg`.

Each system stores `emulator` (that id), `core` (a libretro `.so`, only for RetroArch), `extra_args`, `rom_dirs`, `grid_art` (`box_art` or `screenshot`; an old `title_screen` value loads as box art), and `[consoles.media]` toggles. A hand-edited `rom_dir` string or array, or a single string in `rom_dirs`, still loads. Empty paths are dropped and a path already in the list is not stored twice. The next save writes `rom_dirs` as an array. Loading does not rewrite the file just to convert that key.

The launch command is that program, the rest of `path`, then `global_args`, then `-L <core>` for RetroArch, then `extra_args`, then the ROM. `{rom}` in either argument string is replaced and the ROM is not appended again.

Cores come from that install's cfg only. `libretro_directory` and `libretro_info_path` honor quotes, `~`, and a leading `:`. A missing `libretro_directory` uses `/usr/lib/libretro` for native and the cfg's `cores` directory for Flatpak. On native, the stock values `:cores` and `~/.config/retroarch/cores` still mean `/usr/lib/libretro`. Labels come from `libretro_info_path` `.info` files (`display_name`, otherwise `systemname`).

A scan walks each ROM folder recursively and keeps files whose extension is in the system's list. The file name is cleaned into a title (region tags, bracket tags, other parentheses, and underscores). CRC32 is computed on the way in. An existing CRC is kept on a later scan.

The scan also picks up local media when `[consoles.media]` enables that kind (all four default on). Box art and screenshots are png, jpg, or jpeg beside the ROM or in a `box_art/` or `screenshot/` folder, named like the ROM. Manual (pdf, txt) and video (mp4, mkv, avi) are recorded the same way, under `manual/` and `video/`. The window shows box art and screenshots. A local file found on rescan replaces that kind's library row, including a scraped image.

An old `[[profiles]]` file is rewritten on load: RetroArch cores become one RetroArch emulator plus a core on each system that used them, and a standalone command becomes an emulator. The original bytes are copied to `config.toml.pre-emulators.bak` beside the config first. An existing backup is left alone.

`config.example.toml` is a commented sample. The first run still creates a default config when the file is missing.

This repository does not include a `LICENSE` file. `resources/systems.json` is derived from EmulationStation Desktop Edition. See `ATTRIBUTION.md`.

`cargo test` covers the library code, including scan, config, the database, and launch commands.

## Release

Published file names use the `name` and `version` fields in `Cargo.toml`. A published `vX.Y.Z` release contains, for version `0.1.0`:

- `retromarchy-0.1.0-1-x86_64.pkg.tar.zst`
- `retromarchy-0.1.0-x86_64-linux.tar.gz`
- `SHA256SUMS`

`install.sh` reads the latest release tag from the GitHub API. The tag has to start with `v`.

Cut a release by bumping the version, tagging that same number, and pushing the tag:

1. Set `version` in `Cargo.toml` to `X.Y.Z`.
2. Commit the change.
3. Tag it `vX.Y.Z` (`v0.1.0` for version `0.1.0`).
4. Push the tag. The Release workflow runs `cargo build --release --locked` in an Arch Linux container (`archlinux:base-devel`), so the system libraries match Omarchy, then publishes the package, the tarball, and `SHA256SUMS`.

The workflow refuses a tag that does not match `Cargo.toml`. Actions → Release → Run workflow builds the same artifacts and uploads them without publishing a GitHub Release.
