use crate::config::{self, Config};
use crate::types::{Console, Emulator, EmulatorKind};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const FLATPAK_ID: &str = "org.libretro.RetroArch";
pub const FLATPAK_COMMAND: &str = "flatpak run org.libretro.RetroArch";
pub const FALLBACK_CORE_DIR: &str = "/usr/lib/libretro";

/// One libretro core. `name` is the dedupe key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Core {
    pub name: String,
    pub path: PathBuf,
    pub label: String,
    pub systemname: String,
    pub extensions: Vec<String>,
}

/// Cores from the active libretro directory. `cores` has one entry per `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreCatalog {
    pub directory: PathBuf,
    pub cores: Vec<Core>,
}

impl CoreCatalog {
    pub fn empty() -> Self {
        Self {
            directory: PathBuf::from(FALLBACK_CORE_DIR),
            cores: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetroArchInstall {
    Native { path: PathBuf },
    Flatpak,
}

#[derive(Debug, Clone)]
pub struct DetectEnv {
    pub path: Option<std::ffi::OsString>,
    pub home: PathBuf,
    pub xdg_config_home: Option<PathBuf>,
    pub native_bin: PathBuf,
    pub system_flatpak: Option<PathBuf>,
}

impl DetectEnv {
    pub fn process() -> Self {
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty());
        Self {
            path: std::env::var_os("PATH"),
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_default(),
            xdg_config_home: xdg,
            native_bin: PathBuf::from("/usr/bin/retroarch"),
            system_flatpak: Some(PathBuf::from(format!("/var/lib/flatpak/app/{FLATPAK_ID}"))),
        }
    }
}

pub struct Prepared {
    pub catalog: CoreCatalog,
    pub changed: bool,
}

/// Add RetroArch when none is configured, then repoint cores at the active directory.
/// Does not write the file. [`bootstrap`] saves when `changed` is set.
pub fn prepare(config: &mut Config, env: &DetectEnv) -> Prepared {
    let found = find_retroarch(env);
    let added = ensure_retroarch(&mut config.emulators, found);
    let install = config.emulators.iter().find_map(install_of);
    let catalog = discover(install.as_ref(), env);
    let repointed = repoint_consoles(&mut config.consoles, &catalog);
    Prepared {
        catalog,
        changed: added || repointed,
    }
}

/// Startup path. Uses the process environment and saves when the config changed.
pub fn bootstrap(config: &mut Config) -> CoreCatalog {
    let prepared = prepare(config, &DetectEnv::process());
    if prepared.changed {
        let _ = config::save_config(config);
    }
    prepared.catalog
}

pub fn catalog_for(config: &Config) -> CoreCatalog {
    let install = config.emulators.iter().find_map(install_of);
    discover(install.as_ref(), &DetectEnv::process())
}

pub fn find_retroarch(env: &DetectEnv) -> Option<RetroArchInstall> {
    if let Some(path) = env
        .path
        .as_deref()
        .and_then(|path| executable_on_path(path, "retroarch"))
    {
        return Some(RetroArchInstall::Native { path });
    }
    if is_executable(&env.native_bin) {
        return Some(RetroArchInstall::Native {
            path: env.native_bin.clone(),
        });
    }
    if flatpak_in_home(&env.home)
        || env
            .system_flatpak
            .as_ref()
            .is_some_and(|path| path.is_dir())
    {
        return Some(RetroArchInstall::Flatpak);
    }
    None
}

pub fn ensure_retroarch(emulators: &mut Vec<Emulator>, found: Option<RetroArchInstall>) -> bool {
    if emulators
        .iter()
        .any(|emulator| emulator.kind == EmulatorKind::RetroArch)
    {
        return false;
    }
    let Some(found) = found else {
        return false;
    };
    let path = match found {
        RetroArchInstall::Native { path } => path.to_string_lossy().into_owned(),
        RetroArchInstall::Flatpak => FLATPAK_COMMAND.to_string(),
    };
    let id = fresh_id("retroarch", emulators);
    emulators.push(Emulator {
        id,
        name: "RetroArch".to_string(),
        kind: EmulatorKind::RetroArch,
        path,
        global_args: String::new(),
    });
    true
}

pub fn install_of(emulator: &Emulator) -> Option<RetroArchInstall> {
    if emulator.kind != EmulatorKind::RetroArch {
        return None;
    }
    if flatpak_command(&emulator.path) {
        Some(RetroArchInstall::Flatpak)
    } else {
        let path = program_token(&emulator.path).unwrap_or_else(|| emulator.path.clone());
        Some(RetroArchInstall::Native {
            path: PathBuf::from(path),
        })
    }
}

pub fn discover(install: Option<&RetroArchInstall>, env: &DetectEnv) -> CoreCatalog {
    let cfg_path = retroarch_cfg(install, env);
    let text = fs::read_to_string(&cfg_path).unwrap_or_default();
    let cfg_dir = cfg_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    catalog_from_cfg_text(&text, &cfg_dir, Some(&env.home))
}

pub fn retroarch_cfg(install: Option<&RetroArchInstall>, env: &DetectEnv) -> PathBuf {
    if matches!(install, Some(RetroArchInstall::Flatpak)) {
        return env
            .home
            .join(".var/app")
            .join(FLATPAK_ID)
            .join("config/retroarch/retroarch.cfg");
    }
    if let Some(xdg) = &env.xdg_config_home {
        return xdg.join("retroarch/retroarch.cfg");
    }
    env.home.join(".config/retroarch/retroarch.cfg")
}

pub fn catalog_from_cfg_text(text: &str, cfg_dir: &Path, home: Option<&Path>) -> CoreCatalog {
    let parsed = parse_cfg(text, cfg_dir, home);
    let directory = if parsed.default_cores {
        PathBuf::from(FALLBACK_CORE_DIR)
    } else {
        parsed
            .libretro_directory
            .unwrap_or_else(|| PathBuf::from(FALLBACK_CORE_DIR))
    };
    let cores = scan_cores(&directory, parsed.info_directory.as_deref());
    CoreCatalog { directory, cores }
}

pub fn repoint_path(saved: &Path, catalog: &CoreCatalog) -> Option<PathBuf> {
    let parent = saved.parent()?;
    if same_dir(parent, &catalog.directory) {
        return None;
    }
    let name = core_name(saved)?;
    let core = catalog.cores.iter().find(|core| core.name == name)?;
    if core.path == saved {
        None
    } else {
        Some(core.path.clone())
    }
}

pub fn repoint_consoles(consoles: &mut [Console], catalog: &CoreCatalog) -> bool {
    let mut changed = false;
    for console in consoles {
        let Some(path) = console.core.clone() else {
            continue;
        };
        if let Some(next) = repoint_path(&path, catalog) {
            console.core = Some(next);
            changed = true;
        }
    }
    changed
}

pub fn core_name(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".so")?;
    let name = stem.strip_suffix("_libretro").unwrap_or(stem);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

struct ParsedCfg {
    libretro_directory: Option<PathBuf>,
    default_cores: bool,
    info_directory: Option<PathBuf>,
}

fn parse_cfg(text: &str, cfg_dir: &Path, home: Option<&Path>) -> ParsedCfg {
    let mut libretro_raw = None;
    let mut info_raw = None;
    let mut saw_libretro = false;
    for line in text.lines() {
        let Some((key, value)) = parse_cfg_line(line) else {
            continue;
        };
        if key == "libretro_directory" {
            saw_libretro = true;
            libretro_raw = Some(value);
        } else if key == "libretro_info_path" {
            info_raw = Some(value);
        }
    }
    let expanded = libretro_raw
        .as_deref()
        .map(|raw| expand_path(raw, cfg_dir, home));
    let default_cores = !saw_libretro
        || is_default_libretro(
            libretro_raw.as_deref().unwrap_or(""),
            expanded.as_deref(),
            cfg_dir,
            home,
        );
    let info_directory = info_raw
        .as_deref()
        .map(|raw| expand_path(raw, cfg_dir, home))
        .filter(|path| !path.as_os_str().is_empty());
    ParsedCfg {
        libretro_directory: expanded,
        default_cores,
        info_directory,
    }
}

fn is_default_libretro(
    raw: &str,
    expanded: Option<&Path>,
    cfg_dir: &Path,
    home: Option<&Path>,
) -> bool {
    if raw.is_empty()
        || matches!(
            raw,
            ":cores" | ":/cores" | ":\\cores" | "~/.config/retroarch/cores"
        )
    {
        return true;
    }
    let Some(expanded) = expanded else {
        return true;
    };
    if expanded.as_os_str().is_empty() || expanded == &cfg_dir.join("cores") {
        return true;
    }
    home.is_some_and(|home| expanded == &home.join(".config/retroarch/cores"))
}

fn parse_cfg_line(line: &str) -> Option<(String, String)> {
    let line = strip_unquoted_comment(line).trim();
    if line.is_empty() {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let value = unquote(value.trim());
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key.to_string(), value))
}

fn strip_unquoted_comment(line: &str) -> &str {
    let mut quoted = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..index],
            _ => {}
        }
    }
    line
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

/// `~` is the home directory. `:` is RetroArch's prefix for "relative to this cfg file".
fn expand_path(value: &str, cfg_dir: &Path, home: Option<&Path>) -> PathBuf {
    if value == "~" {
        return home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        if let Some(home) = home {
            return home.join(rest);
        }
    }
    if let Some(rest) = value.strip_prefix(':') {
        let rest = rest.trim_start_matches(['/', '\\']);
        return if rest.is_empty() {
            cfg_dir.to_path_buf()
        } else {
            cfg_dir.join(rest)
        };
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        cfg_dir.join(path)
    }
}

struct InfoText {
    display_name: String,
    systemname: String,
    extensions: Vec<String>,
}

fn scan_cores(dir: &Path, info_dir: Option<&Path>) -> Vec<Core> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = core_name(&path) else {
            continue;
        };
        files.push((name, path));
    }
    files.sort_by(|left, right| {
        let libretro = |path: &Path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("_libretro"))
        };
        left.0
            .to_ascii_lowercase()
            .cmp(&right.0.to_ascii_lowercase())
            .then_with(|| libretro(&right.1).cmp(&libretro(&left.1)))
            .then_with(|| left.1.cmp(&right.1))
    });
    let mut cores = Vec::new();
    let mut seen = HashSet::new();
    for (name, path) in files {
        if !seen.insert(name.clone()) {
            continue;
        }
        let info = read_info(&path, info_dir);
        let label = if !info.display_name.is_empty() {
            info.display_name
        } else if !info.systemname.is_empty() {
            info.systemname.clone()
        } else {
            name.clone()
        };
        cores.push(Core {
            name,
            path,
            label,
            systemname: info.systemname,
            extensions: info.extensions,
        });
    }
    cores.sort_by(|left, right| {
        left.label
            .to_ascii_lowercase()
            .cmp(&right.label.to_ascii_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    cores
}

fn read_info(so_path: &Path, info_dir: Option<&Path>) -> InfoText {
    let Some(info_dir) = info_dir else {
        return InfoText::empty();
    };
    let Some(stem) = so_path.file_stem().and_then(|stem| stem.to_str()) else {
        return InfoText::empty();
    };
    let text = fs::read_to_string(info_dir.join(format!("{stem}.info"))).unwrap_or_default();
    parse_info(&text)
}

impl InfoText {
    fn empty() -> Self {
        Self {
            display_name: String::new(),
            systemname: String::new(),
            extensions: Vec::new(),
        }
    }
}

fn parse_info(text: &str) -> InfoText {
    let mut info = InfoText::empty();
    for line in text.lines() {
        let Some((key, value)) = parse_cfg_line(line) else {
            continue;
        };
        match key.as_str() {
            "display_name" => info.display_name = value,
            "systemname" => info.systemname = value,
            "supported_extensions" => {
                info.extensions = value
                    .split('|')
                    .map(str::trim)
                    .filter(|ext| !ext.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            _ => {}
        }
    }
    info
}

fn same_dir(parent: &Path, dir: &Path) -> bool {
    let parent_key = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    let dir_key = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    parent_key == dir_key
}

fn executable_on_path(path_env: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path_env).find_map(|dir| {
        let candidate = dir.join(name);
        is_executable(&candidate).then_some(candidate)
    })
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return meta.permissions().mode() & 0o111 != 0;
    }
    #[cfg(not(unix))]
    true
}

fn flatpak_in_home(home: &Path) -> bool {
    home.join(".local/share/flatpak/app")
        .join(FLATPAK_ID)
        .is_dir()
        || home.join(".var/app").join(FLATPAK_ID).is_dir()
}

fn flatpak_command(path: &str) -> bool {
    let Ok(parts) = shell_words::split(path) else {
        return false;
    };
    parts.first().is_some_and(|program| program == "flatpak")
        && parts.iter().any(|part| part == FLATPAK_ID)
}

fn program_token(path: &str) -> Option<String> {
    shell_words::split(path).ok()?.into_iter().next()
}

fn fresh_id(base: &str, emulators: &[Emulator]) -> String {
    if !emulators.iter().any(|emulator| emulator.id == base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !emulators.iter().any(|emulator| emulator.id == candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GridArt, MediaToggles};
    use std::os::unix::fs::PermissionsExt;

    fn exec_file(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"#!/bin/sh\n").unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    fn console(core: PathBuf) -> Console {
        Console {
            id: "snes".into(),
            name: "Super Nintendo".into(),
            rom_dirs: vec![PathBuf::from("/tmp/roms/snes")],
            extensions: vec!["sfc".into()],
            emulator: Some("retroarch".into()),
            core: Some(core),
            extra_args: String::new(),
            grid_art: GridArt::BoxArt,
            media: MediaToggles::default(),
        }
    }

    #[test]
    fn cfg_keeps_quoted_values_tilde_and_colon_prefixes() {
        let home = Path::new("/home/player");
        let cfg_dir = Path::new("/home/player/.config/retroarch");
        let text = r#"
# libretro_directory = "/ignored"
libretro_directory = "~/retro-cores"
video_driver = "gl"
libretro_info_path = ":info" # trailing comment
libretro_directory = "local-cores"
"#;
        let custom = catalog_from_cfg_text(text, cfg_dir, Some(home));
        assert_eq!(custom.directory, cfg_dir.join("local-cores"));

        let quoted = "libretro_directory = \"~/retro-cores\"\nlibretro_info_path = \"~/.config/retroarch/info\"\n";
        let parsed = parse_cfg(quoted, cfg_dir, Some(home));
        assert_eq!(
            parsed.libretro_directory.as_deref(),
            Some(home.join("retro-cores").as_path())
        );
        assert!(!parsed.default_cores);
        assert_eq!(
            parsed.info_directory.as_deref(),
            Some(home.join(".config/retroarch/info").as_path())
        );

        let hashed = "libretro_directory = \"/cores/dir#name\"\n";
        let hashed = catalog_from_cfg_text(hashed, cfg_dir, Some(home));
        assert_eq!(hashed.directory, PathBuf::from("/cores/dir#name"));

        let colon = "libretro_directory = \":/custom\"\n";
        let colon = catalog_from_cfg_text(colon, cfg_dir, Some(home));
        assert_eq!(colon.directory, cfg_dir.join("custom"));

        let stock = "libretro_directory = \":cores\"\n";
        let stock = catalog_from_cfg_text(stock, cfg_dir, Some(home));
        assert_eq!(stock.directory, PathBuf::from(FALLBACK_CORE_DIR));

        let tilde_stock = "libretro_directory = \"~/.config/retroarch/cores\"\n";
        let tilde_stock = catalog_from_cfg_text(tilde_stock, cfg_dir, Some(home));
        assert_eq!(tilde_stock.directory, PathBuf::from(FALLBACK_CORE_DIR));
    }

    #[test]
    fn one_directory_lists_each_core_once_and_skips_a_second_directory() {
        let root = tempfile::tempdir().unwrap();
        let active = root.path().join("active");
        let other = root.path().join("other");
        let info = root.path().join("info");
        fs::create_dir_all(&active).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::create_dir_all(&info).unwrap();
        fs::write(active.join("snes9x_libretro.so"), b"core").unwrap();
        fs::write(active.join("snes9x.so"), b"dup").unwrap();
        fs::write(active.join("notes.txt"), b"no").unwrap();
        fs::write(other.join("snes9x_libretro.so"), b"other").unwrap();
        fs::write(other.join("fceumm_libretro.so"), b"nes").unwrap();
        fs::write(
            info.join("snes9x_libretro.info"),
            "display_name = \"Nintendo - SNES / SFC (Snes9x)\"\nsystemname = \"Super Nintendo Entertainment System\"\nsupported_extensions = \"sfc|smc\"\n",
        )
        .unwrap();
        let cfg = format!(
            "libretro_directory = \"{}\"\nlibretro_info_path = \"{}\"\n",
            active.display(),
            info.display()
        );
        let catalog = catalog_from_cfg_text(&cfg, root.path(), Some(root.path()));
        assert_eq!(catalog.cores.len(), 1);
        assert_eq!(catalog.cores[0].name, "snes9x");
        assert_eq!(catalog.cores[0].path, active.join("snes9x_libretro.so"));
        assert_eq!(catalog.cores[0].label, "Nintendo - SNES / SFC (Snes9x)");
        assert_eq!(
            catalog.cores[0].systemname,
            "Super Nintendo Entertainment System"
        );
        assert_eq!(catalog.cores[0].extensions, vec!["sfc", "smc"]);
        assert!(catalog
            .cores
            .iter()
            .all(|core| core.path.starts_with(&active)));
    }

    #[test]
    fn a_stock_directory_is_not_the_core_source() {
        let root = tempfile::tempdir().unwrap();
        let cfg_dir = root.path().join(".config/retroarch");
        fs::create_dir_all(cfg_dir.join("cores")).unwrap();
        fs::write(cfg_dir.join("cores").join("only_stock_libretro.so"), b"x").unwrap();
        let catalog = catalog_from_cfg_text(
            "libretro_directory = \":cores\"\n",
            &cfg_dir,
            Some(root.path()),
        );
        assert!(catalog.cores.iter().all(|core| core.name != "only_stock"));
        assert_eq!(catalog.directory, PathBuf::from(FALLBACK_CORE_DIR));
    }

    #[test]
    fn repoint_moves_a_stem_into_the_active_directory_and_keeps_a_miss() {
        let root = tempfile::tempdir().unwrap();
        let active = root.path().join("active");
        fs::create_dir_all(&active).unwrap();
        fs::write(active.join("snes9x_libretro.so"), b"core").unwrap();
        let cfg = format!("libretro_directory = \"{}\"\n", active.display());
        let catalog = catalog_from_cfg_text(&cfg, root.path(), Some(root.path()));
        let outside = root.path().join("old/snes9x_libretro.so");
        assert_eq!(
            repoint_path(&outside, &catalog).as_deref(),
            Some(active.join("snes9x_libretro.so").as_path())
        );
        assert!(repoint_path(&active.join("snes9x_libretro.so"), &catalog).is_none());
        let missing = root.path().join("old/missing_libretro.so");
        assert!(repoint_path(&missing, &catalog).is_none());

        let mut consoles = vec![console(outside), console(missing.clone())];
        consoles[1].id = "missing".into();
        assert!(repoint_consoles(&mut consoles, &catalog));
        assert_eq!(
            consoles[0].core.as_deref(),
            Some(active.join("snes9x_libretro.so").as_path())
        );
        assert_eq!(consoles[1].core.as_deref(), Some(missing.as_path()));
        assert!(!repoint_consoles(&mut consoles, &catalog));
    }

    #[test]
    fn detection_prefers_path_then_usr_bin_then_flatpak_and_does_not_duplicate() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        let on_path = bin.join("retroarch");
        exec_file(&on_path);
        let usr = root.path().join("usr/bin/retroarch");
        exec_file(&usr);
        fs::create_dir_all(
            root.path()
                .join(".local/share/flatpak/app")
                .join(FLATPAK_ID),
        )
        .unwrap();
        let env = DetectEnv {
            path: Some(bin.as_os_str().into()),
            home: root.path().to_path_buf(),
            xdg_config_home: None,
            native_bin: usr.clone(),
            system_flatpak: None,
        };
        match find_retroarch(&env) {
            Some(RetroArchInstall::Native { path }) => assert_eq!(path, on_path),
            other => panic!("expected the PATH binary, got {other:?}"),
        }

        let no_path = DetectEnv {
            path: Some(root.path().join("empty").into()),
            home: root.path().to_path_buf(),
            xdg_config_home: None,
            native_bin: usr.clone(),
            system_flatpak: None,
        };
        match find_retroarch(&no_path) {
            Some(RetroArchInstall::Native { path }) => assert_eq!(path, usr),
            other => panic!("expected /usr-style binary, got {other:?}"),
        }

        let flatpak_only = DetectEnv {
            path: Some(root.path().join("empty").into()),
            home: root.path().to_path_buf(),
            xdg_config_home: None,
            native_bin: root.path().join("missing"),
            system_flatpak: None,
        };
        assert_eq!(
            find_retroarch(&flatpak_only),
            Some(RetroArchInstall::Flatpak)
        );

        let mut emulators = Vec::new();
        assert!(ensure_retroarch(
            &mut emulators,
            Some(RetroArchInstall::Flatpak)
        ));
        assert_eq!(emulators[0].path, FLATPAK_COMMAND);
        assert_eq!(emulators[0].kind, EmulatorKind::RetroArch);
        assert!(!ensure_retroarch(
            &mut emulators,
            Some(RetroArchInstall::Native { path: on_path })
        ));
        assert_eq!(emulators.len(), 1);

        let mut existing = vec![Emulator {
            id: "custom".into(),
            name: "My RetroArch".into(),
            kind: EmulatorKind::RetroArch,
            path: "/opt/retroarch".into(),
            global_args: String::new(),
        }];
        assert!(!ensure_retroarch(
            &mut existing,
            Some(RetroArchInstall::Flatpak)
        ));
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].path, "/opt/retroarch");
    }

    #[test]
    fn prepare_adds_retroarch_and_repoints_without_a_second_entry() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        let retroarch = bin.join("retroarch");
        exec_file(&retroarch);
        let xdg = root.path().join("xdg");
        let active = root.path().join("active");
        fs::create_dir_all(xdg.join("retroarch")).unwrap();
        fs::create_dir_all(&active).unwrap();
        fs::write(active.join("snes9x_libretro.so"), b"core").unwrap();
        fs::write(
            xdg.join("retroarch/retroarch.cfg"),
            format!("libretro_directory = \"{}\"\n", active.display()),
        )
        .unwrap();
        let mut config = Config::default();
        config
            .consoles
            .push(console(root.path().join("old/snes9x_libretro.so")));
        let env = DetectEnv {
            path: Some(bin.as_os_str().into()),
            home: root.path().to_path_buf(),
            xdg_config_home: Some(xdg),
            native_bin: root.path().join("missing"),
            system_flatpak: None,
        };
        let prepared = prepare(&mut config, &env);
        assert!(prepared.changed);
        assert_eq!(config.emulators.len(), 1);
        assert_eq!(config.emulators[0].path, retroarch.to_string_lossy());
        assert_eq!(prepared.catalog.cores.len(), 1);
        assert_eq!(
            config.consoles[0].core.as_deref(),
            Some(active.join("snes9x_libretro.so").as_path())
        );
        let again = prepare(&mut config, &env);
        assert!(!again.changed);
        assert_eq!(config.emulators.len(), 1);
    }

    #[test]
    fn flatpak_cfg_is_under_the_var_app_directory() {
        let home = Path::new("/home/player");
        let env = DetectEnv {
            path: None,
            home: home.to_path_buf(),
            xdg_config_home: Some(PathBuf::from("/xdg")),
            native_bin: PathBuf::from("/usr/bin/retroarch"),
            system_flatpak: None,
        };
        assert_eq!(
            retroarch_cfg(Some(&RetroArchInstall::Flatpak), &env),
            home.join(format!(
                ".var/app/{FLATPAK_ID}/config/retroarch/retroarch.cfg"
            ))
        );
        assert_eq!(
            retroarch_cfg(None, &env),
            PathBuf::from("/xdg/retroarch/retroarch.cfg")
        );
        let no_xdg = DetectEnv {
            xdg_config_home: None,
            ..env
        };
        assert_eq!(
            retroarch_cfg(None, &no_xdg),
            home.join(".config/retroarch/retroarch.cfg")
        );
    }
}
