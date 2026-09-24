use crate::types::EmulatorProfile;
use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Command;

pub fn build_launch_command(profile: &EmulatorProfile, rom: &Path) -> Result<(String, Vec<String>)> {
    match profile {
        EmulatorProfile::RetroArch { core, config, .. } => {
            let mut args = vec!["-L".to_string(), core.to_string_lossy().to_string()];
            if let Some(cfg) = config {
                args.push("--config".to_string());
                args.push(cfg.to_string_lossy().to_string());
            }
            args.push(rom.to_string_lossy().to_string());
            Ok(("retroarch".to_string(), args))
        }
        EmulatorProfile::Standalone { command, .. } => {
            let substituted = command.replace("{rom}", &rom.to_string_lossy());
            let parts = shell_words::split(&substituted)
                .map_err(|e| anyhow!("Failed to parse command: {}", e))?;
            if parts.is_empty() {
                return Err(anyhow!("Empty command"));
            }
            let (program, args) = parts.split_first().unwrap();
            Ok((program.to_string(), args.to_vec()))
        }
    }
}

pub fn launch_game(profile: &EmulatorProfile, rom: &Path) -> Result<()> {
    let (program, args) = build_launch_command(profile, rom)?;
    Command::new(&program)
        .args(&args)
        .spawn()
        .map_err(|e| anyhow!("Failed to launch {}: {}", program, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_retroarch_command() {
        let profile = EmulatorProfile::RetroArch {
            id: "snes9x".to_string(),
            core: PathBuf::from("/usr/lib/libretro/snes9x_libretro.so"),
            config: None,
        };
        let rom = PathBuf::from("/roms/game.sfc");
        let (program, args) = build_launch_command(&profile, &rom).unwrap();
        assert_eq!(program, "retroarch");
        assert_eq!(args, vec!["-L", "/usr/lib/libretro/snes9x_libretro.so", "/roms/game.sfc"]);
    }

    #[test]
    fn test_retroarch_with_config() {
        let profile = EmulatorProfile::RetroArch {
            id: "snes9x".to_string(),
            core: PathBuf::from("/usr/lib/libretro/snes9x_libretro.so"),
            config: Some(PathBuf::from("/home/user/.config/retroarch/snes.cfg")),
        };
        let rom = PathBuf::from("/roms/game.sfc");
        let (program, args) = build_launch_command(&profile, &rom).unwrap();
        assert_eq!(program, "retroarch");
        assert_eq!(
            args,
            vec![
                "-L",
                "/usr/lib/libretro/snes9x_libretro.so",
                "--config",
                "/home/user/.config/retroarch/snes.cfg",
                "/roms/game.sfc"
            ]
        );
    }

    #[test]
    fn test_standalone_command() {
        let profile = EmulatorProfile::Standalone {
            id: "dolphin".to_string(),
            command: "dolphin-emu -b -e {rom}".to_string(),
        };
        let rom = PathBuf::from("/roms/game.iso");
        let (program, args) = build_launch_command(&profile, &rom).unwrap();
        assert_eq!(program, "dolphin-emu");
        assert_eq!(args, vec!["-b", "-e", "/roms/game.iso"]);
    }

    #[test]
    fn test_standalone_with_quotes() {
        let profile = EmulatorProfile::Standalone {
            id: "pcsx2".to_string(),
            command: "pcsx2 --fullscreen \"{rom}\"".to_string(),
        };
        let rom = PathBuf::from("/roms/game with spaces.iso");
        let (program, args) = build_launch_command(&profile, &rom).unwrap();
        assert_eq!(program, "pcsx2");
        assert_eq!(args, vec!["--fullscreen", "/roms/game with spaces.iso"]);
    }
}
