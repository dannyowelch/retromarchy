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

/// Cores from one install's libretro directory. `cores` has one entry per `name`.
/// `note` is set when that directory is missing or has no cores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreCatalog {
    pub directory: PathBuf,
    pub cores: Vec<Core>,
    pub cfg: PathBuf,
    pub note: Option<String>,
}

impl CoreCatalog {
    pub fn empty() -> Self {
        Self {
            directory: PathBuf::from(FALLBACK_CORE_DIR),
            cores: Vec::new(),
            cfg: PathBuf::new(),
            note: None,
        }
    }
}

/// Which RetroArch this entry launches. Native and Flatpak do not share a cfg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Native,
    Flatpak,
}

/// User and system Flatpaks share `~/.var/app/<id>` but they are different installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatpakScope {
    User,
    System,
}

/// A launch command plus the cfg that install reads. Cores come from `cfg` only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetroArchInstall {
    Native {
        command: String,
        cfg: PathBuf,
    },
    Flatpak {
        command: String,
        cfg: PathBuf,
        scope: FlatpakScope,
    },
}

impl RetroArchInstall {
    pub fn command(&self) -> &str {
        match self {
            Self::Native { command, .. } | Self::Flatpak { command, .. } => command,
        }
    }

    pub fn cfg(&self) -> &Path {
        match self {
            Self::Native { cfg, .. } | Self::Flatpak { cfg, .. } => cfg,
        }
    }

    pub fn kind(&self) -> InstallKind {
        match self {
            Self::Native { .. } => InstallKind::Native,
            Self::Flatpak { .. } => InstallKind::Flatpak,
        }
    }
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

/// Add every RetroArch install that is not already configured, then repoint
/// each system's core inside that system's install. Does not write the file.
pub fn prepare(config: &mut Config, env: &DetectEnv) -> Prepared {
    let found = find_installs(env);
    let added = ensure_installs(&mut config.emulators, &found);
    let repointed = repoint_owned(&mut config.consoles, &config.emulators, env);
    let catalog = config
        .emulators
        .iter()
        .find_map(|emulator| install_of(emulator, env))
        .map(|install| discover(&install, env))
        .unwrap_or_else(CoreCatalog::empty);
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

pub fn catalog_for_emulator(emulator: &Emulator, env: &DetectEnv) -> CoreCatalog {
    match install_of(emulator, env) {
        Some(install) => discover(&install, env),
        None => CoreCatalog::empty(),
    }
}

pub fn catalogs_for(config: &Config, env: &DetectEnv) -> Vec<CoreCatalog> {
    config
        .emulators
        .iter()
        .map(|emulator| catalog_for_emulator(emulator, env))
        .collect()
}

/// Every install that is present. Native, user Flatpak, and system Flatpak.
pub fn find_installs(env: &DetectEnv) -> Vec<RetroArchInstall> {
    let mut installs = Vec::new();
    if let Some(path) = native_executable(env) {
        installs.push(RetroArchInstall::Native {
            command: path.to_string_lossy().into_owned(),
            cfg: native_cfg(env),
        });
    }
    let scopes = flatpak_scopes(env);
    let both = scopes.len() > 1;
    for scope in scopes {
        installs.push(RetroArchInstall::Flatpak {
            command: flatpak_launch(scope, both),
            cfg: flatpak_cfg(env),
            scope,
        });
    }
    installs
}

/// Append installs that no existing RetroArch entry already represents.
/// An entry whose binary is gone stays in the list.
pub fn ensure_installs(emulators: &mut Vec<Emulator>, found: &[RetroArchInstall]) -> bool {
    let mut changed = false;
    for install in found {
        if emulators
            .iter()
            .any(|emulator| covers(emulator, install, found))
        {
            continue;
        }
        let (id_base, name) = install_label(install, found);
        let id = fresh_id(id_base, emulators);
        emulators.push(Emulator {
            id,
            name,
            kind: EmulatorKind::RetroArch,
            path: install.command().to_string(),
            global_args: String::new(),
            config: Some(install.cfg().to_path_buf()),
        });
        changed = true;
    }
    changed
}

pub fn install_of(emulator: &Emulator, env: &DetectEnv) -> Option<RetroArchInstall> {
    if emulator.kind != EmulatorKind::RetroArch {
        return None;
    }
    let cfg = emulator
        .config
        .clone()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| derived_cfg(&emulator.path, env));
    if flatpak_command(&emulator.path) {
        Some(RetroArchInstall::Flatpak {
            command: emulator.path.clone(),
            cfg,
            scope: scope_flag(&emulator.path).unwrap_or(FlatpakScope::User),
        })
    } else {
        Some(RetroArchInstall::Native {
            command: emulator.path.clone(),
            cfg,
        })
    }
}

/// The command's program is still on disk, or the Flatpak scope is still installed.
pub fn command_present(path: &str, env: &DetectEnv, found: &[RetroArchInstall]) -> bool {
    if flatpak_command(path) {
        return found
            .iter()
            .any(|install| covers_path(path, install, found));
    }
    program_resolves(path, env)
}

pub fn discover(install: &RetroArchInstall, env: &DetectEnv) -> CoreCatalog {
    let cfg_path = install.cfg();
    let text = fs::read_to_string(cfg_path).unwrap_or_default();
    let cfg_dir = cfg_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut catalog = catalog_from_cfg_text(&text, &cfg_dir, Some(&env.home), install.kind());
    catalog.cfg = cfg_path.to_path_buf();
    catalog
}

pub fn native_cfg(env: &DetectEnv) -> PathBuf {
    if let Some(xdg) = &env.xdg_config_home {
        xdg.join("retroarch/retroarch.cfg")
    } else {
        env.home.join(".config/retroarch/retroarch.cfg")
    }
}

pub fn flatpak_cfg(env: &DetectEnv) -> PathBuf {
    env.home
        .join(".var/app")
        .join(FLATPAK_ID)
        .join("config/retroarch/retroarch.cfg")
}

pub fn catalog_from_cfg_text(
    text: &str,
    cfg_dir: &Path,
    home: Option<&Path>,
    kind: InstallKind,
) -> CoreCatalog {
    let parsed = parse_cfg(text, cfg_dir, home);
    let directory = core_directory(&parsed, cfg_dir, kind);
    let (cores, note) = read_cores(&directory, parsed.info_directory.as_deref());
    CoreCatalog {
        directory,
        cores,
        cfg: cfg_dir.join("retroarch.cfg"),
        note,
    }
}

fn core_directory(parsed: &ParsedCfg, cfg_dir: &Path, kind: InstallKind) -> PathBuf {
    let flatpak_default = cfg_dir.join("cores");
    match kind {
        InstallKind::Flatpak => {
            if parsed.saw_libretro {
                parsed.libretro_directory.clone().unwrap_or(flatpak_default)
            } else {
                flatpak_default
            }
        }
        InstallKind::Native if parsed.default_cores => PathBuf::from(FALLBACK_CORE_DIR),
        InstallKind::Native => parsed
            .libretro_directory
            .clone()
            .unwrap_or_else(|| PathBuf::from(FALLBACK_CORE_DIR)),
    }
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

pub fn repoint_id(consoles: &mut [Console], emulator_id: &str, catalog: &CoreCatalog) -> bool {
    let mut changed = false;
    for console in consoles {
        if console.emulator.as_deref() != Some(emulator_id) {
            continue;
        }
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

/// Repoint each system using only the emulator that system is assigned to.
pub fn repoint_owned(consoles: &mut [Console], emulators: &[Emulator], env: &DetectEnv) -> bool {
    let mut catalogs: Vec<(String, CoreCatalog)> = Vec::new();
    let mut changed = false;
    for console in consoles {
        let Some(id) = console.emulator.clone() else {
            continue;
        };
        let Some(emulator) = emulators.iter().find(|emulator| emulator.id == id) else {
            continue;
        };
        if !catalogs.iter().any(|(known, _)| known == &id) {
            let catalog = catalog_for_emulator(emulator, env);
            catalogs.push((id.clone(), catalog));
        }
        let Some((_, catalog)) = catalogs.iter().find(|(known, _)| known == &id) else {
            continue;
        };
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
    saw_libretro: bool,
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
        saw_libretro,
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

fn read_cores(dir: &Path, info_dir: Option<&Path>) -> (Vec<Core>, Option<String>) {
    if !dir.is_dir() {
        return (
            Vec::new(),
            Some(format!("No core directory at {}.", dir.display())),
        );
    }
    let cores = scan_cores(dir, info_dir);
    if cores.is_empty() {
        (cores, Some(format!("No cores in {}.", dir.display())))
    } else {
        (cores, None)
    }
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

fn native_executable(env: &DetectEnv) -> Option<PathBuf> {
    if let Some(path) = env
        .path
        .as_deref()
        .and_then(|path| executable_on_path(path, "retroarch"))
    {
        return Some(path);
    }
    is_executable(&env.native_bin).then(|| env.native_bin.clone())
}

fn derived_cfg(command: &str, env: &DetectEnv) -> PathBuf {
    if flatpak_command(command) {
        flatpak_cfg(env)
    } else {
        native_cfg(env)
    }
}

fn flatpak_launch(scope: FlatpakScope, both: bool) -> String {
    if !both {
        return FLATPAK_COMMAND.to_string();
    }
    match scope {
        FlatpakScope::User => format!("flatpak run --user {FLATPAK_ID}"),
        FlatpakScope::System => format!("flatpak run --system {FLATPAK_ID}"),
    }
}

fn install_label(install: &RetroArchInstall, found: &[RetroArchInstall]) -> (&'static str, String) {
    match install {
        RetroArchInstall::Native { .. } => ("retroarch", "RetroArch".to_string()),
        RetroArchInstall::Flatpak {
            scope: FlatpakScope::System,
            ..
        } if found.iter().any(|item| {
            matches!(
                item,
                RetroArchInstall::Flatpak {
                    scope: FlatpakScope::User,
                    ..
                }
            )
        }) =>
        {
            (
                "retroarch-flatpak-system",
                "RetroArch (Flatpak system)".to_string(),
            )
        }
        RetroArchInstall::Flatpak { .. } => {
            ("retroarch-flatpak", "RetroArch (Flatpak)".to_string())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Native,
    User,
    System,
    Plain,
}

fn entry_kind(path: &str) -> EntryKind {
    if !flatpak_command(path) {
        return EntryKind::Native;
    }
    match scope_flag(path) {
        Some(FlatpakScope::System) => EntryKind::System,
        Some(FlatpakScope::User) => EntryKind::User,
        None => EntryKind::Plain,
    }
}

fn scope_flag(path: &str) -> Option<FlatpakScope> {
    let Ok(parts) = shell_words::split(path) else {
        return None;
    };
    if parts.iter().any(|part| part == "--system") {
        Some(FlatpakScope::System)
    } else if parts.iter().any(|part| part == "--user") {
        Some(FlatpakScope::User)
    } else {
        None
    }
}

fn covers(emulator: &Emulator, install: &RetroArchInstall, found: &[RetroArchInstall]) -> bool {
    emulator.kind == EmulatorKind::RetroArch && covers_path(&emulator.path, install, found)
}

fn covers_path(path: &str, install: &RetroArchInstall, found: &[RetroArchInstall]) -> bool {
    let user_found = found.iter().any(|item| {
        matches!(
            item,
            RetroArchInstall::Flatpak {
                scope: FlatpakScope::User,
                ..
            }
        )
    });
    match (entry_kind(path), install) {
        (EntryKind::Native, RetroArchInstall::Native { .. }) => true,
        (
            EntryKind::User | EntryKind::Plain,
            RetroArchInstall::Flatpak {
                scope: FlatpakScope::User,
                ..
            },
        ) => true,
        (
            EntryKind::System,
            RetroArchInstall::Flatpak {
                scope: FlatpakScope::System,
                ..
            },
        ) => true,
        (
            EntryKind::Plain,
            RetroArchInstall::Flatpak {
                scope: FlatpakScope::System,
                ..
            },
        ) => !user_found,
        _ => false,
    }
}

fn program_resolves(path: &str, env: &DetectEnv) -> bool {
    let Some(token) = program_token(path) else {
        return false;
    };
    let program = Path::new(&token);
    if program.is_absolute() {
        return is_executable(program);
    }
    env.path
        .as_deref()
        .and_then(|path_env| executable_on_path(path_env, &token))
        .is_some()
}

fn flatpak_scopes(env: &DetectEnv) -> Vec<FlatpakScope> {
    if let Some(bin) = env
        .path
        .as_deref()
        .and_then(|path| executable_on_path(path, "flatpak"))
    {
        return flatpak_scopes_from_cli(&bin);
    }
    directory_scopes(env)
}

fn flatpak_scopes_from_cli(bin: &Path) -> Vec<FlatpakScope> {
    let list = command_stdout(bin, &["list", "--app"]).unwrap_or_default();
    let mut scopes = parse_flatpak_list(&list);
    let info = command_stdout(bin, &["info", FLATPAK_ID]).unwrap_or_default();
    if scopes.is_empty() {
        if let Some(scope) = parse_flatpak_info(&info) {
            scopes.push(scope);
        }
    }
    scopes
}

fn command_stdout(program: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_flatpak_list(text: &str) -> Vec<FlatpakScope> {
    let mut scopes = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if !tokens.iter().any(|token| *token == FLATPAK_ID) {
            continue;
        }
        let scope = tokens.iter().rev().find_map(|token| match *token {
            "user" => Some(FlatpakScope::User),
            "system" => Some(FlatpakScope::System),
            _ => None,
        });
        if let Some(scope) = scope {
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
    }
    scopes
}

fn parse_flatpak_info(text: &str) -> Option<FlatpakScope> {
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("Installation") {
            return match value.trim() {
                "user" => Some(FlatpakScope::User),
                "system" => Some(FlatpakScope::System),
                _ => None,
            };
        }
    }
    None
}

fn directory_scopes(env: &DetectEnv) -> Vec<FlatpakScope> {
    let mut scopes = Vec::new();
    if env
        .home
        .join(".local/share/flatpak/app")
        .join(FLATPAK_ID)
        .is_dir()
    {
        scopes.push(FlatpakScope::User);
    }
    if env
        .system_flatpak
        .as_ref()
        .is_some_and(|path| path.is_dir())
    {
        scopes.push(FlatpakScope::System);
    }
    if scopes.is_empty() && env.home.join(".var/app").join(FLATPAK_ID).is_dir() {
        scopes.push(FlatpakScope::User);
    }
    scopes
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
        let custom = catalog_from_cfg_text(text, cfg_dir, Some(home), InstallKind::Native);
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
        let hashed = catalog_from_cfg_text(hashed, cfg_dir, Some(home), InstallKind::Native);
        assert_eq!(hashed.directory, PathBuf::from("/cores/dir#name"));

        let colon = "libretro_directory = \":/custom\"\n";
        let colon = catalog_from_cfg_text(colon, cfg_dir, Some(home), InstallKind::Native);
        assert_eq!(colon.directory, cfg_dir.join("custom"));

        let stock = "libretro_directory = \":cores\"\n";
        let stock = catalog_from_cfg_text(stock, cfg_dir, Some(home), InstallKind::Native);
        assert_eq!(stock.directory, PathBuf::from(FALLBACK_CORE_DIR));

        let tilde_stock = "libretro_directory = \"~/.config/retroarch/cores\"\n";
        let tilde_stock =
            catalog_from_cfg_text(tilde_stock, cfg_dir, Some(home), InstallKind::Native);
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
        let catalog =
            catalog_from_cfg_text(&cfg, root.path(), Some(root.path()), InstallKind::Native);
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
            InstallKind::Native,
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
        let catalog =
            catalog_from_cfg_text(&cfg, root.path(), Some(root.path()), InstallKind::Native);
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

    fn retroarch_emulator(id: &str, path: &str) -> Emulator {
        Emulator {
            id: id.into(),
            name: "RetroArch".into(),
            kind: EmulatorKind::RetroArch,
            path: path.into(),
            global_args: String::new(),
            config: None,
        }
    }

    #[test]
    fn detection_finds_each_install_and_does_not_duplicate() {
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
        let both = find_installs(&env);
        assert_eq!(both.len(), 2);
        assert_eq!(both[0].command(), on_path.to_string_lossy());
        assert_eq!(both[0].cfg(), native_cfg(&env));
        assert_eq!(both[1].command(), FLATPAK_COMMAND);
        assert_eq!(both[1].cfg(), flatpak_cfg(&env));

        let no_path = DetectEnv {
            path: Some(root.path().join("empty").into()),
            home: root.path().to_path_buf(),
            xdg_config_home: None,
            native_bin: usr.clone(),
            system_flatpak: None,
        };
        let from_usr = find_installs(&no_path);
        assert_eq!(from_usr[0].command(), usr.to_string_lossy());
        assert!(matches!(from_usr[1], RetroArchInstall::Flatpak { .. }));

        let flatpak_only = DetectEnv {
            path: Some(root.path().join("empty").into()),
            home: root.path().to_path_buf(),
            xdg_config_home: None,
            native_bin: root.path().join("missing"),
            system_flatpak: None,
        };
        let only = find_installs(&flatpak_only);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].command(), FLATPAK_COMMAND);

        let mut emulators = Vec::new();
        assert!(ensure_installs(&mut emulators, &both));
        assert_eq!(emulators.len(), 2);
        assert_eq!(emulators[0].id, "retroarch");
        assert_eq!(emulators[0].name, "RetroArch");
        assert_eq!(emulators[1].id, "retroarch-flatpak");
        assert_eq!(emulators[1].name, "RetroArch (Flatpak)");
        assert_eq!(
            emulators[1].config.as_deref(),
            Some(flatpak_cfg(&env).as_path())
        );
        assert!(!ensure_installs(&mut emulators, &both));
        assert_eq!(emulators.len(), 2);

        let mut existing = vec![retroarch_emulator("retroarch", FLATPAK_COMMAND)];
        assert!(ensure_installs(&mut existing, &both));
        assert_eq!(existing.len(), 2);
        assert_eq!(existing[0].id, "retroarch");
        assert_eq!(existing[0].path, FLATPAK_COMMAND);
        assert_eq!(existing[1].path, on_path.to_string_lossy());

        let mut custom = vec![retroarch_emulator("custom", "/opt/retroarch")];
        assert!(ensure_installs(&mut custom, &both));
        assert_eq!(custom.len(), 2);
        assert_eq!(custom[0].path, "/opt/retroarch");
        assert_eq!(custom[1].path, FLATPAK_COMMAND);
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
    fn user_layout_flatpak_cores_survive_a_leftover_native_cfg() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let native_cfg = home.join(".config/retroarch/retroarch.cfg");
        fs::create_dir_all(native_cfg.parent().unwrap()).unwrap();
        fs::write(&native_cfg, "video_driver = \"gl\"\n").unwrap();

        let flatpak_dir = home
            .join(".var/app")
            .join(FLATPAK_ID)
            .join("config/retroarch");
        let cores = flatpak_dir.join("cores");
        let info = flatpak_dir.join("info");
        fs::create_dir_all(&cores).unwrap();
        fs::create_dir_all(&info).unwrap();
        for index in 0..91 {
            fs::write(cores.join(format!("core{index:02}_libretro.so")), b"x").unwrap();
        }
        let cores_rel = cores.strip_prefix(home).unwrap().display().to_string();
        let info_rel = info.strip_prefix(home).unwrap().display().to_string();
        fs::write(
            flatpak_dir.join("retroarch.cfg"),
            format!(
                "libretro_directory = \"~/{cores_rel}\"\nlibretro_info_path = \"~/{info_rel}\"\n"
            ),
        )
        .unwrap();

        let env = DetectEnv {
            path: Some(home.join("empty-path").into()),
            home: home.to_path_buf(),
            xdg_config_home: None,
            native_bin: home.join("missing-retroarch"),
            system_flatpak: None,
        };
        let text = fs::read_to_string(flatpak_dir.join("retroarch.cfg")).unwrap();
        let parsed = parse_cfg(&text, &flatpak_dir, Some(home));
        assert_eq!(parsed.libretro_directory.as_deref(), Some(cores.as_path()));
        assert!(parsed.default_cores);

        let flatpak_install = RetroArchInstall::Flatpak {
            command: FLATPAK_COMMAND.into(),
            cfg: flatpak_dir.join("retroarch.cfg"),
            scope: FlatpakScope::User,
        };
        let flatpak = discover(&flatpak_install, &env);
        assert_eq!(flatpak.directory, cores);
        assert_eq!(flatpak.cores.len(), 91);
        assert!(flatpak.note.is_none());

        let native_install = RetroArchInstall::Native {
            command: "retroarch".into(),
            cfg: native_cfg.clone(),
        };
        let native = discover(&native_install, &env);
        assert!(native.cores.is_empty());
        assert_eq!(native.directory, PathBuf::from(FALLBACK_CORE_DIR));
        assert_eq!(
            native.note.as_deref(),
            Some("No core directory at /usr/lib/libretro.")
        );
        assert_ne!(flatpak_install.cfg(), native_install.cfg());

        let entry = retroarch_emulator("retroarch", FLATPAK_COMMAND);
        let from_entry = catalog_for_emulator(&entry, &env);
        assert_eq!(from_entry.cores.len(), 91);
        assert_eq!(from_entry.directory, cores);
        assert_eq!(from_entry.cfg, flatpak_dir.join("retroarch.cfg"));
    }

    #[test]
    fn cfg_resolution_uses_each_installs_default_when_the_key_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let flatpak_dir = home
            .join(".var/app")
            .join(FLATPAK_ID)
            .join("config/retroarch");
        fs::create_dir_all(flatpak_dir.join("cores")).unwrap();
        fs::write(flatpak_dir.join("cores").join("fceumm_libretro.so"), b"nes").unwrap();
        let missing = catalog_from_cfg_text(
            "video_driver = \"gl\"\n",
            &flatpak_dir,
            Some(home),
            InstallKind::Flatpak,
        );
        assert_eq!(missing.directory, flatpak_dir.join("cores"));
        assert_eq!(missing.cores.len(), 1);
        assert_eq!(missing.cores[0].name, "fceumm");

        let quoted = catalog_from_cfg_text(
            "libretro_directory = \":/custom\"\nlibretro_info_path = \":/info\"\n",
            &flatpak_dir,
            Some(home),
            InstallKind::Flatpak,
        );
        assert_eq!(quoted.directory, flatpak_dir.join("custom"));

        let native_dir = home.join(".config/retroarch");
        let native = catalog_from_cfg_text("", &native_dir, Some(home), InstallKind::Native);
        assert_eq!(native.directory, PathBuf::from(FALLBACK_CORE_DIR));
        assert_eq!(
            native.note.as_deref(),
            Some("No core directory at /usr/lib/libretro.")
        );

        let empty_dir = home.join("empty-cores");
        fs::create_dir_all(&empty_dir).unwrap();
        let empty = catalog_from_cfg_text(
            &format!("libretro_directory = \"{}\"\n", empty_dir.display()),
            home,
            Some(home),
            InstallKind::Native,
        );
        assert_eq!(empty.cores.len(), 0);
        assert_eq!(
            empty.note.as_deref(),
            Some(format!("No cores in {}.", empty_dir.display())).as_deref()
        );
    }

    #[test]
    fn both_installs_keep_their_own_cores_and_repoint_stays_on_the_system() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let bin = home.join("bin");
        exec_file(&bin.join("retroarch"));
        fs::create_dir_all(home.join(".local/share/flatpak/app").join(FLATPAK_ID)).unwrap();
        let native_cores = home.join("native-cores");
        let flatpak_cores = home
            .join(".var/app")
            .join(FLATPAK_ID)
            .join("config/retroarch/cores");
        fs::create_dir_all(&native_cores).unwrap();
        fs::create_dir_all(&flatpak_cores).unwrap();
        fs::write(native_cores.join("snes9x_libretro.so"), b"native").unwrap();
        fs::write(native_cores.join("fceumm_libretro.so"), b"nes").unwrap();
        fs::write(flatpak_cores.join("snes9x_libretro.so"), b"flatpak").unwrap();
        let xdg = home.join("xdg");
        fs::create_dir_all(xdg.join("retroarch")).unwrap();
        fs::write(
            xdg.join("retroarch/retroarch.cfg"),
            format!("libretro_directory = \"{}\"\n", native_cores.display()),
        )
        .unwrap();
        let flatpak_cfg_path = home
            .join(".var/app")
            .join(FLATPAK_ID)
            .join("config/retroarch/retroarch.cfg");
        fs::create_dir_all(flatpak_cfg_path.parent().unwrap()).unwrap();
        fs::write(
            &flatpak_cfg_path,
            format!("libretro_directory = \"{}\"\n", flatpak_cores.display()),
        )
        .unwrap();
        let env = DetectEnv {
            path: Some(bin.as_os_str().into()),
            home: home.to_path_buf(),
            xdg_config_home: Some(xdg),
            native_bin: home.join("missing"),
            system_flatpak: None,
        };
        let mut config = Config::default();
        let mut snes = console(native_cores.join("snes9x_libretro.so"));
        snes.emulator = Some("retroarch-flatpak".into());
        let mut nes = console(flatpak_cores.join("fceumm_libretro.so"));
        nes.id = "nes".into();
        nes.emulator = Some("retroarch".into());
        config.consoles.push(snes);
        config.consoles.push(nes);
        let prepared = prepare(&mut config, &env);
        assert!(prepared.changed);
        assert_eq!(config.emulators.len(), 2);
        let native = catalog_for_emulator(&config.emulators[0], &env);
        let flatpak = catalog_for_emulator(&config.emulators[1], &env);
        assert_eq!(
            native
                .cores
                .iter()
                .map(|core| core.name.as_str())
                .collect::<Vec<_>>(),
            vec!["fceumm", "snes9x"]
        );
        assert!(native
            .cores
            .iter()
            .all(|core| core.path.starts_with(&native_cores)));
        assert_eq!(flatpak.cores.len(), 1);
        assert_eq!(
            flatpak.cores[0].path,
            flatpak_cores.join("snes9x_libretro.so")
        );
        assert_eq!(
            config.consoles[0].core.as_deref(),
            Some(flatpak_cores.join("snes9x_libretro.so").as_path())
        );
        assert_eq!(
            config.consoles[1].core.as_deref(),
            Some(native_cores.join("fceumm_libretro.so").as_path())
        );
    }

    #[test]
    fn flatpak_list_and_info_find_user_and_system_installs() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let log = root.path().join("flatpak.log");
        let stub = bin.join("flatpak");
        fs::write(
            &stub,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nif [ \"$1\" = list ]; then\n  echo 'RetroArch {id} 1.22.2 stable user'\n  echo 'RetroArch {id} 1.22.2 stable system'\n  exit 0\nfi\nif [ \"$1\" = info ]; then\n  echo 'ID: {id}'\n  echo 'Installation: user'\n  exit 0\nfi\nexit 1\n",
                log.display(),
                id = FLATPAK_ID,
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub, perms).unwrap();
        let env = DetectEnv {
            path: Some(bin.as_os_str().into()),
            home: root.path().to_path_buf(),
            xdg_config_home: None,
            native_bin: root.path().join("missing"),
            system_flatpak: None,
        };
        let found = find_installs(&env);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].command(),
            format!("flatpak run --user {FLATPAK_ID}")
        );
        assert_eq!(
            found[1].command(),
            format!("flatpak run --system {FLATPAK_ID}")
        );
        assert_eq!(found[0].cfg(), found[1].cfg());
        let calls = fs::read_to_string(&log).unwrap();
        assert!(calls.contains("list --app"));
        assert!(calls.contains(&format!("info {FLATPAK_ID}")));

        let mut emulators = vec![retroarch_emulator("retroarch", FLATPAK_COMMAND)];
        assert!(ensure_installs(&mut emulators, &found));
        assert_eq!(emulators.len(), 2);
        assert_eq!(emulators[0].path, FLATPAK_COMMAND);
        assert_eq!(emulators[1].name, "RetroArch (Flatpak system)");
        assert!(!command_present("/usr/bin/retroarch", &env, &found));
        assert!(command_present(FLATPAK_COMMAND, &env, &found));
        assert!(command_present(
            &format!("flatpak run --system {FLATPAK_ID}"),
            &env,
            &found
        ));
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
            flatpak_cfg(&env),
            home.join(format!(
                ".var/app/{FLATPAK_ID}/config/retroarch/retroarch.cfg"
            ))
        );
        assert_eq!(
            native_cfg(&env),
            PathBuf::from("/xdg/retroarch/retroarch.cfg")
        );
        let no_xdg = DetectEnv {
            xdg_config_home: None,
            ..env
        };
        assert_eq!(
            native_cfg(&no_xdg),
            home.join(".config/retroarch/retroarch.cfg")
        );
    }
}
