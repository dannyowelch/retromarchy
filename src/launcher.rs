use crate::types::{Emulator, EmulatorKind, ResolvedLaunch};
use anyhow::{anyhow, Result};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

/// `path` is a command. The first token is the program and the rest lead the
/// argument list, then global args, then `-L <core>` for RetroArch, then the
/// system's extra args, then the ROM. `{rom}` inside either argument string is
/// replaced and the ROM is not appended again.
pub fn build_launch_command(
    emulator: &Emulator,
    core: Option<&Path>,
    extra_args: &str,
    rom: &Path,
) -> Result<(String, Vec<String>)> {
    let (program, leading) = split_program(&emulator.path)
        .map_err(|_| anyhow!("Emulator \"{}\" has no executable path.", emulator.name))?;
    let rom_text = rom.to_string_lossy();
    let (global, global_rom) = tokenize(&emulator.global_args, &rom_text)?;
    let (extra, extra_rom) = tokenize(extra_args, &rom_text)?;
    let mut args = leading;
    args.extend(global);
    if emulator.kind == EmulatorKind::RetroArch {
        let core = core.ok_or_else(|| {
            anyhow!("RetroArch needs a core for this system. Set one in Manage Systems.")
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

pub struct RunningGame {
    child: Child,
    flatpak_app: Option<String>,
    /// Instance ids of this app that were already running before spawn.
    flatpak_baseline: Option<BTreeSet<String>>,
}

impl RunningGame {
    pub fn wait(mut self) {
        wait_for_game(
            &mut self.child,
            self.flatpak_app.as_deref(),
            self.flatpak_baseline.as_ref(),
        );
    }
}

pub fn launch_game_tracked(launch: &ResolvedLaunch, rom: &Path) -> Result<RunningGame> {
    let (program, args) = build_launch_command(
        &launch.emulator,
        launch.core.as_deref(),
        &launch.extra_args,
        rom,
    )?;
    let flatpak_app = flatpak_app_id(&program, &args);
    // Snapshot before spawn so the instance this launch creates is not baseline.
    let flatpak_baseline = flatpak_app.as_deref().and_then(flatpak_instances);
    let mut command = Command::new(&program);
    command.args(&args);
    // The child pid is the process group id. A `flatpak run` that forks and
    // returns leaves the game in that group unless the sandbox calls setsid.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|err| anyhow!("Failed to launch {program}: {err}"))?;
    Ok(RunningGame {
        child,
        flatpak_app,
        flatpak_baseline,
    })
}

/// Block until the launched game is gone.
///
/// The direct child is waited. `flatpak run` can still return while the game
/// is running, so anything left in the child's process group or recorded as a
/// descendant counts, and so does a Flatpak instance of `flatpak_app` that
/// was not in `flatpak_baseline`.
fn wait_for_game(
    child: &mut Child,
    flatpak_app: Option<&str>,
    flatpak_baseline: Option<&BTreeSet<String>>,
) {
    let leader = child.id();
    let mut descendants = BTreeSet::new();
    let mut tracked_instances = BTreeSet::new();
    loop {
        harvest(leader, &mut descendants);
        match child.try_wait() {
            Ok(None) => thread::sleep(POLL),
            _ => break,
        }
    }

    let mut quiet = 0u8;
    loop {
        let seeds: Vec<u32> = descendants.iter().copied().collect();
        for pid in seeds {
            if pid_alive(pid) {
                harvest(pid, &mut descendants);
            }
        }
        descendants.retain(|pid| pid_alive(*pid));
        let descendant_alive = descendants.iter().any(|pid| *pid != leader);
        let group_alive = group_has_other(leader);
        let flatpak_alive = note_flatpak(flatpak_app, flatpak_baseline, &mut tracked_instances);
        if still_running(false, group_alive, descendant_alive, flatpak_alive) {
            quiet = 0;
            thread::sleep(POLL);
            continue;
        }
        quiet += 1;
        let need = if flatpak_app.is_some()
            && flatpak_baseline.is_some()
            && tracked_instances.is_empty()
        {
            FLATPAK_LATE_POLLS
        } else {
            1
        };
        if quiet >= need {
            break;
        }
        thread::sleep(POLL);
    }
}

const POLL: Duration = Duration::from_millis(25);
const FLATPAK_LATE_POLLS: u8 = 8;

fn still_running(child: bool, group: bool, descendant: bool, flatpak: bool) -> bool {
    child || group || descendant || flatpak
}

fn note_flatpak(
    app: Option<&str>,
    baseline: Option<&BTreeSet<String>>,
    tracked: &mut BTreeSet<String>,
) -> bool {
    let Some(app) = app else {
        return false;
    };
    let Some(baseline) = baseline else {
        return false;
    };
    let Some(now) = flatpak_instances(app) else {
        return false;
    };
    for id in &now {
        if !baseline.contains(id) {
            tracked.insert(id.clone());
        }
    }
    tracked.iter().any(|id| now.contains(id))
}

fn flatpak_app_id(program: &str, args: &[String]) -> Option<String> {
    if Path::new(program).file_name().and_then(OsStr::to_str) != Some("flatpak") {
        return None;
    }
    let run = args.iter().position(|arg| arg == "run")?;
    let mut index = run + 1;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            index += 1;
            break;
        }
        if arg.starts_with('-') {
            if !arg.contains('=') && flatpak_option_takes_value(arg) {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        return Some(arg.clone());
    }
    args.get(index).cloned()
}

fn flatpak_option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "--arch"
            | "--branch"
            | "--command"
            | "--commit"
            | "--runtime"
            | "--runtime-version"
            | "--runtime-commit"
            | "--cwd"
            | "--parent-pid"
            | "--device"
            | "--filesystem"
            | "--share"
            | "--socket"
            | "--unset-env"
            | "--env"
            | "--env-fd"
            | "--talk-name"
            | "--own-name"
            | "--system-talk-name"
            | "--system-own-name"
            | "--add-policy"
            | "--persist"
            | "--usr-path"
            | "--app-path"
            | "--instance-id-fd"
            | "--installation"
    )
}

fn flatpak_instances(app: &str) -> Option<BTreeSet<String>> {
    let text = flatpak_ps_text()?;
    let mut ids = BTreeSet::new();
    for (instance, row_app) in parse_flatpak_ps(&text) {
        if row_app == app {
            ids.insert(instance);
        }
    }
    Some(ids)
}

fn flatpak_ps_text() -> Option<String> {
    for args in [
        ["ps", "--columns=instance,application"].as_slice(),
        ["ps"].as_slice(),
    ] {
        let output = Command::new("flatpak")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if output.status.success() {
            return String::from_utf8(output.stdout).ok();
        }
    }
    None
}

fn parse_flatpak_ps(text: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(instance) = fields.first() else {
            continue;
        };
        if !instance.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let Some(app) = flatpak_app_field(&fields) else {
            continue;
        };
        rows.push(((*instance).to_string(), app.to_string()));
    }
    rows
}

fn flatpak_app_field<'a>(fields: &[&'a str]) -> Option<&'a str> {
    if fields.len() == 2 {
        return Some(fields[1]);
    }
    fields
        .iter()
        .copied()
        .find(|field| field.contains('.') && !field.starts_with('/'))
}

struct ProcStat {
    ppid: u32,
    pgrp: u32,
}

fn proc_stat(text: &str) -> Option<ProcStat> {
    let rest = text.rsplit_once(')')?.1.trim_start();
    let mut fields = rest.split_whitespace();
    let _state = fields.next()?;
    let ppid = fields.next()?.parse().ok()?;
    let pgrp = fields.next()?.parse().ok()?;
    Some(ProcStat { ppid, pgrp })
}

fn pid_name(name: &OsStr) -> Option<u32> {
    name.to_str()?.parse().ok()
}

fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

fn harvest(root: u32, into: &mut BTreeSet<u32>) {
    let mut stack = vec![root];
    let mut guard = 0u32;
    while let Some(pid) = stack.pop() {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        for child in children_of(pid) {
            if into.insert(child) {
                stack.push(child);
            }
        }
    }
}

fn children_of(parent: u32) -> Vec<u32> {
    let mut kids = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return kids;
    };
    for entry in entries.flatten() {
        let Some(pid) = pid_name(&entry.file_name()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        if proc_stat(&text).is_some_and(|stat| stat.ppid == parent) {
            kids.push(pid);
        }
    }
    kids
}

fn group_has_other(pgid: u32) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let Some(pid) = pid_name(&entry.file_name()) else {
            continue;
        };
        if pid == pgid {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        if proc_stat(&text).is_some_and(|stat| stat.pgrp == pgid) {
            return true;
        }
    }
    false
}

fn split_program(path: &str) -> Result<(String, Vec<String>)> {
    let parts =
        shell_words::split(path.trim()).map_err(|err| anyhow!("Failed to parse command: {err}"))?;
    let mut parts = parts.into_iter();
    let Some(program) = parts.next() else {
        return Err(anyhow!("empty command"));
    };
    if program.is_empty() {
        return Err(anyhow!("empty command"));
    }
    Ok((program, parts.collect()))
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
            config: None,
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
            config: None,
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
            config: None,
        };
        let rom = PathBuf::from("/roms/game with spaces.iso");
        let (program, args) = build_launch_command(&emulator, None, "", &rom).unwrap();
        assert_eq!(program, "pcsx2");
        assert_eq!(args, vec!["--fullscreen", "/roms/game with spaces.iso"]);
    }

    #[test]
    fn flatpak_retroarch_splits_the_command_and_passes_the_core() {
        let emulator = Emulator {
            id: "retroarch".into(),
            name: "RetroArch".into(),
            kind: EmulatorKind::RetroArch,
            path: "flatpak run org.libretro.RetroArch".into(),
            global_args: String::new(),
            config: None,
        };
        let rom = PathBuf::from("/roms/game.sfc");
        let core = PathBuf::from("/cores/active/snes9x_libretro.so");
        let (program, args) = build_launch_command(&emulator, Some(&core), "", &rom).unwrap();
        assert_eq!(program, "flatpak");
        assert_eq!(
            args,
            vec![
                "run",
                "org.libretro.RetroArch",
                "-L",
                "/cores/active/snes9x_libretro.so",
                "/roms/game.sfc",
            ]
        );
    }

    #[test]
    fn quoted_rom_placeholder_keeps_spaces() {
        let emulator = Emulator {
            id: "pcsx2".into(),
            name: "PCSX2".into(),
            kind: EmulatorKind::Standalone,
            path: "pcsx2".into(),
            global_args: "--fullscreen \"{rom}\"".into(),
            config: None,
        };
        let rom = PathBuf::from("/roms/game with spaces.iso");
        let (program, args) = build_launch_command(&emulator, None, "-f", &rom).unwrap();
        assert_eq!(program, "pcsx2");
        assert_eq!(
            args,
            vec!["--fullscreen", "/roms/game with spaces.iso", "-f"]
        );
    }

    #[test]
    fn flatpak_app_id_is_the_ref_after_run_options() {
        let retroarch = |path: &str| {
            let emulator = Emulator {
                id: "retroarch".into(),
                name: "RetroArch".into(),
                kind: EmulatorKind::RetroArch,
                path: path.into(),
                global_args: String::new(),
                config: None,
            };
            let core = PathBuf::from("/cores/snes9x_libretro.so");
            build_launch_command(&emulator, Some(&core), "", Path::new("/roms/game.sfc")).unwrap()
        };
        let (program, args) = retroarch("flatpak run org.libretro.RetroArch");
        assert_eq!(
            flatpak_app_id(&program, &args).as_deref(),
            Some("org.libretro.RetroArch")
        );
        let (program, args) = retroarch("flatpak run --user org.libretro.RetroArch");
        assert_eq!(
            flatpak_app_id(&program, &args).as_deref(),
            Some("org.libretro.RetroArch")
        );
        let (program, args) =
            retroarch("flatpak run --system --filesystem=host org.libretro.RetroArch");
        assert_eq!(
            flatpak_app_id(&program, &args).as_deref(),
            Some("org.libretro.RetroArch")
        );
        let (program, args) =
            retroarch("/usr/bin/flatpak run --command=retroarch org.libretro.RetroArch");
        assert_eq!(
            flatpak_app_id(&program, &args).as_deref(),
            Some("org.libretro.RetroArch")
        );
        let (program, args) = retroarch("retroarch");
        assert_eq!(flatpak_app_id(&program, &args), None);
    }

    #[test]
    fn flatpak_ps_rows_skip_the_header_and_keep_the_application() {
        let text = "\
Instance PID Application Runtime
4284582911 12345 org.libretro.RetroArch org.freedesktop.Platform

Instance Application
7 org.example.App
not a row
";
        assert_eq!(
            parse_flatpak_ps(text),
            vec![
                (
                    "4284582911".to_string(),
                    "org.libretro.RetroArch".to_string()
                ),
                ("7".to_string(), "org.example.App".to_string()),
            ]
        );
    }

    #[test]
    fn a_flatpak_instance_keeps_the_game_running_after_the_cli_exits() {
        assert!(!still_running(false, false, false, false));
        assert!(still_running(true, false, false, false));
        assert!(still_running(false, true, false, false));
        assert!(still_running(false, false, true, false));
        assert!(still_running(false, false, false, true));
    }

    #[test]
    fn stat_line_reads_ppid_and_pgrp_when_comm_contains_parens() {
        let stat = proc_stat("12 (foo) bar) S 34 56 78 0").unwrap();
        assert_eq!(stat.ppid, 34);
        assert_eq!(stat.pgrp, 56);
        let self_stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let parsed = proc_stat(&self_stat).unwrap();
        assert!(parsed.pgrp > 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_background_sleep_keeps_the_launch_alive_until_it_exits() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("emu.sh");
        std::fs::write(&script, "#!/bin/sh\ntrap '' HUP\nsleep 0.5 &\nexit 0\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        let mut command = Command::new(&script);
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        let child = command.spawn().unwrap();
        let started = std::time::Instant::now();
        RunningGame {
            child,
            flatpak_app: None,
            flatpak_baseline: None,
        }
        .wait();
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(400),
            "forked child was not waited: {elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(3), "wait hung: {elapsed:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_direct_child_is_waited_until_it_exits() {
        let mut command = Command::new("sleep");
        command.arg("0.3");
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        let child = command.spawn().unwrap();
        let started = std::time::Instant::now();
        RunningGame {
            child,
            flatpak_app: None,
            flatpak_baseline: None,
        }
        .wait();
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(250), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    }
}
