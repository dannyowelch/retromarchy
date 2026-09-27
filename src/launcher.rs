use crate::types::{Emulator, EmulatorKind, ResolvedLaunch};
use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::{Child, Command};

/// `program` is the emulator path. Arguments are global args, then `-L <core>`
/// for RetroArch, then the system's extra args, then the ROM.
/// `{rom}` inside either argument string is replaced and the ROM is not appended again.
pub fn build_launch_command(
    emulator: &Emulator,
    core: Option<&Path>,
    extra_args: &str,
    rom: &Path,
) -> Result<(String, Vec<String>)> {
    let program = emulator.path.trim();
    if program.is_empty() {
        return Err(anyhow!(
            "Emulator \"{}\" has no executable path.",
            emulator.name
        ));
    }
    let rom_text = rom.to_string_lossy();
    let (global, global_rom) = tokenize(&emulator.global_args, &rom_text)?;
    let (extra, extra_rom) = tokenize(extra_args, &rom_text)?;
    let mut args = global;
    if emulator.kind == EmulatorKind::RetroArch {
        let core = core.ok_or_else(|| {
            anyhow!("RetroArch needs a core for this system. Set one in Manage Emulators.")
        })?;
        args.push("-L".to_string());
        args.push(core.to_string_lossy().into_owned());
    }
    args.extend(extra);
    if !(global_rom || extra_rom) {
        args.push(rom_text.into_owned());
    }
    Ok((program.to_string(), args))
}

pub fn launch_game_tracked(launch: &ResolvedLaunch, rom: &Path) -> Result<Child> {
    let (program, args) = build_launch_command(
        &launch.emulator,
        launch.core.as_deref(),
        &launch.extra_args,
        rom,
    )?;
    Command::new(&program)
        .args(&args)
        .spawn()
        .map_err(|err| anyhow!("Failed to launch {program}: {err}"))
}

fn tokenize(args: &str, rom: &str) -> Result<(Vec<String>, bool)> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok((Vec::new(), false));
    }
    let used = trimmed.contains("{rom}");
    let substituted = trimmed.replace("{rom}", rom);
    let parts = shell_words::split(&substituted)
        .map_err(|err| anyhow!("Failed to parse command: {err}"))?;
    Ok((parts, used))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EmulatorKind;
    use std::path::PathBuf;

    fn retroarch(global_args: &str) -> Emulator {
        Emulator {
            id: "retroarch".into(),
            name: "RetroArch".into(),
            kind: EmulatorKind::RetroArch,
            path: "/usr/bin/retroarch".into(),
            global_args: global_args.into(),
        }
    }

    #[test]
    fn retroarch_command_is_path_global_core_extra_and_rom() {
        let emulator = retroarch("--verbose");
        let rom = PathBuf::from("/roms/game.sfc");
        let core = PathBuf::from("/usr/lib/libretro/snes9x_libretro.so");
        let (program, args) =
            build_launch_command(&emulator, Some(&core), "--config /cfg/snes.cfg", &rom).unwrap();
        assert_eq!(program, "/usr/bin/retroarch");
        assert_eq!(
            args,
            vec![
                "--verbose",
                "-L",
                "/usr/lib/libretro/snes9x_libretro.so",
                "--config",
                "/cfg/snes.cfg",
                "/roms/game.sfc",
            ]
        );
    }

    #[test]
    fn retroarch_without_a_core_fails() {
        let emulator = retroarch("");
        let err =
            build_launch_command(&emulator, None, "", Path::new("/roms/game.sfc")).unwrap_err();
        assert!(err.to_string().contains("needs a core"), "{err}");
    }

    #[test]
    fn standalone_substitutes_rom_placeholder_and_does_not_append_it() {
        let emulator = Emulator {
            id: "dolphin".into(),
            name: "Dolphin".into(),
            kind: EmulatorKind::Standalone,
            path: "dolphin-emu".into(),
            global_args: "-b -e {rom}".into(),
        };
        let rom = PathBuf::from("/roms/game.iso");
        let (program, args) = build_launch_command(&emulator, None, "", &rom).unwrap();
        assert_eq!(program, "dolphin-emu");
        assert_eq!(args, vec!["-b", "-e", "/roms/game.iso"]);
    }

    #[test]
    fn standalone_appends_the_rom_when_no_placeholder_is_present() {
        let emulator = Emulator {
            id: "pcsx2".into(),
            name: "PCSX2".into(),
            kind: EmulatorKind::Standalone,
            path: "pcsx2".into(),
            global_args: "--fullscreen".into(),
        };
        let rom = PathBuf::from("/roms/game with spaces.iso");
        let (program, args) = build_launch_command(&emulator, None, "", &rom).unwrap();
        assert_eq!(program, "pcsx2");
        assert_eq!(args, vec!["--fullscreen", "/roms/game with spaces.iso"]);
    }

    #[test]
    fn quoted_rom_placeholder_keeps_spaces() {
        let emulator = Emulator {
            id: "pcsx2".into(),
            name: "PCSX2".into(),
            kind: EmulatorKind::Standalone,
            path: "pcsx2".into(),
            global_args: "--fullscreen \"{rom}\"".into(),
        };
        let rom = PathBuf::from("/roms/game with spaces.iso");
        let (program, args) = build_launch_command(&emulator, None, "-f", &rom).unwrap();
        assert_eq!(program, "pcsx2");
        assert_eq!(
            args,
            vec!["--fullscreen", "/roms/game with spaces.iso", "-f"]
        );
    }
}
