use crate::types::EmulatorProfile;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A libretro `.so` found on disk. [`DiscoveredCore::to_profile`] is the only
/// way a discovered core becomes a RetroArch profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredCore {
    /// File name with `_libretro` and `.so` removed (`stella_libretro.so` → `stella`).
    pub name: String,
    pub path: PathBuf,
}

/// Discovered core → RetroArch profile, given the profiles already saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreProfile {
    /// Some RetroArch profile already points at this core file.
    AlreadyAdded { id: String },
    /// New profile. The id is the core name, or `name-2`, `name-3`, … when taken.
    New(EmulatorProfile),
}

impl DiscoveredCore {
    pub fn from_path(path: PathBuf) -> Option<Self> {
        let file_name = path.file_name()?.to_str()?;
        let name = display_name(file_name)?;
        Some(Self { name, path })
    }

    pub fn to_profile(&self, existing: &[EmulatorProfile]) -> CoreProfile {
        if let Some(id) = existing.iter().find_map(|profile| match profile {
            EmulatorProfile::RetroArch { id, core, .. } if same_file(core, &self.path) => {
                Some(id.clone())
            }
            _ => None,
        }) {
            return CoreProfile::AlreadyAdded { id };
        }
        CoreProfile::New(EmulatorProfile::RetroArch {
            id: unique_id(&self.name, existing),
            core: self.path.clone(),
            config: None,
        })
    }

    /// Catalog system ids this core clearly runs. Callers may assign only
    /// systems that are already present in the user's config.
    pub fn system_ids(&self) -> &'static [&'static str] {
        system_ids_for_core(&self.name)
    }
}

pub fn discover_cores() -> Vec<DiscoveredCore> {
    discover_in_dirs(&candidate_dirs(home_dir().as_deref()))
}

pub fn discover_in_dirs(dirs: &[PathBuf]) -> Vec<DiscoveredCore> {
    let mut cores = Vec::new();
    let mut seen = HashSet::new();
    for dir in dirs {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(core) = DiscoveredCore::from_path(path) else {
                continue;
            };
            let key = fs::canonicalize(&core.path).unwrap_or_else(|_| core.path.clone());
            if seen.insert(key) {
                cores.push(core);
            }
        }
    }
    cores.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    cores
}

fn candidate_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/lib/libretro"),
        PathBuf::from("/usr/lib/libretro/cores"),
        PathBuf::from("/usr/share/libretro/cores"),
    ];
    if let Some(home) = home {
        dirs.push(home.join(".config/retroarch/cores"));
        let cfg = home.join(".config/retroarch/retroarch.cfg");
        if let Some(dir) = libretro_directory(&cfg, Some(home)) {
            dirs.push(dir);
        }
    }
    dedupe_dirs(dirs)
}

fn libretro_directory(cfg_path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let text = fs::read_to_string(cfg_path).ok()?;
    let cfg_dir = cfg_path.parent().unwrap_or(Path::new("."));
    directory_from_cfg_text(&text, cfg_dir, home)
}

fn directory_from_cfg_text(text: &str, cfg_dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut found = None;
    for line in text.lines() {
        let Some((key, value)) = parse_cfg_line(line) else {
            continue;
        };
        if key == "libretro_directory" {
            found = Some(value);
        }
    }
    let raw = found?;
    let path = expand_path(&raw, cfg_dir, home);
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
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

fn expand_path(value: &str, cfg_dir: &Path, home: Option<&Path>) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = home {
            return home.join(rest);
        }
    }
    if value == "~" {
        if let Some(home) = home {
            return home.to_path_buf();
        }
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        cfg_dir.join(path)
    }
}

fn dedupe_dirs(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for dir in dirs {
        let key = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if seen.insert(key) {
            out.push(dir);
        }
    }
    out
}

fn display_name(file_name: &str) -> Option<String> {
    let stem = file_name.strip_suffix(".so")?;
    let name = stem.strip_suffix("_libretro").unwrap_or(stem);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn unique_id(base: &str, existing: &[EmulatorProfile]) -> String {
    if !existing.iter().any(|profile| profile.id() == base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !existing.iter().any(|profile| profile.id() == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// Core stem → catalog system ids. Longest matching stem wins.
const CORE_SYSTEMS: &[(&str, &[&str])] = &[
    ("nestopia", &["nes"]),
    ("fceumm", &["nes"]),
    ("quicknes", &["nes"]),
    ("mesen", &["nes"]),
    ("snes9x", &["snes"]),
    ("bsnes", &["snes"]),
    ("mesen-s", &["snes"]),
    ("stella", &["atari2600"]),
    ("stella2014", &["atari2600"]),
    ("prosystem", &["atari7800"]),
    ("a5200", &["atari5200"]),
    ("gambatte", &["gb", "gbc"]),
    ("sameboy", &["gb", "gbc"]),
    ("gearboy", &["gb", "gbc"]),
    ("mgba", &["gba"]),
    ("vbam", &["gba"]),
    ("gpsp", &["gba"]),
    ("mednafen_gba", &["gba"]),
    ("mupen64plus_next", &["n64"]),
    ("mupen64plus", &["n64"]),
    ("parallel_n64", &["n64"]),
    ("genesis_plus_gx", &["genesis", "megadrive"]),
    ("picodrive", &["genesis", "megadrive"]),
    ("pcsx_rearmed", &["psx"]),
    ("swanstation", &["psx"]),
    ("beetle_psx_hw", &["psx"]),
    ("beetle_psx", &["psx"]),
    ("mednafen_pce", &["pcengine"]),
    ("melonds", &["nds"]),
    ("desmume", &["nds"]),
    ("flycast", &["dreamcast"]),
    ("ppsspp", &["psp"]),
    ("handy", &["atarilynx"]),
    ("mednafen_wswan", &["wonderswan", "wonderswancolor"]),
    ("fbneo", &["fbneo", "neogeo", "arcade"]),
    ("opera", &["3do"]),
    ("virtualjaguar", &["atarijaguar"]),
];

fn system_ids_for_core(name: &str) -> &'static [&'static str] {
    let key = name.to_ascii_lowercase();
    let mut best: Option<(usize, &'static [&'static str])> = None;
    for &(core, systems) in CORE_SYSTEMS {
        if !core_name_matches(&key, core) {
            continue;
        }
        if best.map(|(len, _)| core.len() > len).unwrap_or(true) {
            best = Some((core.len(), systems));
        }
    }
    best.map(|(_, systems)| systems).unwrap_or(&[])
}

fn core_name_matches(key: &str, core: &str) -> bool {
    let Some(rest) = key.strip_prefix(core) else {
        return false;
    };
    rest.is_empty()
        || rest.starts_with('_')
        || rest.starts_with('-')
        || rest.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn strips_libretro_suffix_for_the_display_name() {
        let core = DiscoveredCore::from_path(PathBuf::from("/usr/lib/libretro/snes9x_libretro.so"))
            .unwrap();
        assert_eq!(core.name, "snes9x");
        assert!(DiscoveredCore::from_path(PathBuf::from("/tmp/readme.txt")).is_none());
        assert!(DiscoveredCore::from_path(PathBuf::from("/tmp/_libretro.so")).is_none());
    }

    #[test]
    fn discovered_core_becomes_retroarch_profile() {
        let core = DiscoveredCore::from_path(PathBuf::from("/usr/lib/libretro/stella_libretro.so"))
            .unwrap();
        let CoreProfile::New(profile) = core.to_profile(&[]) else {
            panic!("expected a new profile");
        };
        match &profile {
            EmulatorProfile::RetroArch { id, core, config } => {
                assert_eq!(id, "stella");
                assert_eq!(core, &PathBuf::from("/usr/lib/libretro/stella_libretro.so"));
                assert!(config.is_none());
            }
            EmulatorProfile::Standalone { .. } => panic!("expected RetroArch"),
        }

        #[derive(serde::Serialize)]
        struct Profiles {
            profiles: Vec<EmulatorProfile>,
        }
        let text = toml::to_string_pretty(&Profiles {
            profiles: vec![profile],
        })
        .unwrap();
        assert!(text.contains("type = 'RetroArch'"), "{text}");
        assert!(text.contains("id = 'stella'"), "{text}");
        assert!(
            text.contains("core = '/usr/lib/libretro/stella_libretro.so'"),
            "{text}"
        );
        assert!(!text.contains("config"), "{text}");
    }

    #[test]
    fn profile_id_is_unique_and_same_file_is_already_added() {
        let core = DiscoveredCore::from_path(PathBuf::from("/cores/nestopia_libretro.so")).unwrap();
        let taken = EmulatorProfile::Standalone {
            id: "nestopia".to_string(),
            command: "echo {rom}".to_string(),
        };
        let CoreProfile::New(profile) = core.to_profile(&[taken]) else {
            panic!("expected a new profile");
        };
        assert_eq!(profile.id(), "nestopia-2");

        let existing = EmulatorProfile::RetroArch {
            id: "nes-core".to_string(),
            core: PathBuf::from("/cores/nestopia_libretro.so"),
            config: None,
        };
        match core.to_profile(&[existing]) {
            CoreProfile::AlreadyAdded { id } => assert_eq!(id, "nes-core"),
            CoreProfile::New(_) => panic!("expected the existing profile"),
        }
    }

    #[test]
    fn scan_lists_so_files_and_skips_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("stella_libretro.so"), b"core").unwrap();
        fs::write(dir.path().join("notes.txt"), b"no").unwrap();
        fs::create_dir(dir.path().join("nested.so")).unwrap();
        let missing = dir.path().join("does-not-exist");
        let cores = discover_in_dirs(&[dir.path().to_path_buf(), missing]);
        assert_eq!(cores.len(), 1);
        assert_eq!(cores[0].name, "stella");
    }

    #[test]
    fn scan_dedupes_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("fceumm_libretro.so");
        fs::write(&real, b"core").unwrap();
        let other = tempfile::tempdir().unwrap();
        symlink(&real, other.path().join("fceumm_libretro.so")).unwrap();
        let cores = discover_in_dirs(&[dir.path().to_path_buf(), other.path().to_path_buf()]);
        assert_eq!(cores.len(), 1);
        assert_eq!(cores[0].name, "fceumm");
    }

    #[test]
    fn cfg_libretro_directory_last_value_wins_and_expands() {
        let home = Path::new("/home/player");
        let cfg_dir = Path::new("/home/player/.config/retroarch");
        let text = r#"
# libretro_directory = "/ignored"
libretro_directory = "~/retro-cores"
video_driver = "gl"
libretro_directory = "local-cores" # trailing comment
"#;
        let path = directory_from_cfg_text(text, cfg_dir, Some(home)).unwrap();
        assert_eq!(path, cfg_dir.join("local-cores"));

        let quoted = "libretro_directory = \"~/retro-cores\"\n";
        let path = directory_from_cfg_text(quoted, cfg_dir, Some(home)).unwrap();
        assert_eq!(path, home.join("retro-cores"));
        assert!(directory_from_cfg_text("video_driver = \"gl\"\n", cfg_dir, Some(home)).is_none());
    }

    #[test]
    fn candidate_dirs_include_home_and_cfg_without_duplicates() {
        let home = tempfile::tempdir().unwrap();
        let retroarch = home.path().join(".config/retroarch");
        fs::create_dir_all(&retroarch).unwrap();
        fs::write(
            retroarch.join("retroarch.cfg"),
            "libretro_directory = \"~/.config/retroarch/cores\"\n",
        )
        .unwrap();
        let dirs = candidate_dirs(Some(home.path()));
        assert!(dirs.iter().any(|dir| dir == Path::new("/usr/lib/libretro")));
        assert!(dirs
            .iter()
            .any(|dir| dir == Path::new("/usr/lib/libretro/cores")));
        assert!(dirs
            .iter()
            .any(|dir| dir == Path::new("/usr/share/libretro/cores")));
        let home_cores = home.path().join(".config/retroarch/cores");
        assert_eq!(dirs.iter().filter(|dir| *dir == &home_cores).count(), 1);
    }

    #[test]
    fn unreadable_cfg_is_skipped() {
        let missing = Path::new("/this/config/does/not/exist/retroarch.cfg");
        assert!(libretro_directory(missing, Some(Path::new("/home/player"))).is_none());
    }

    #[test]
    fn core_names_map_to_known_systems_only() {
        assert_eq!(system_ids_for_core("nestopia"), &["nes"]);
        assert_eq!(system_ids_for_core("fceumm"), &["nes"]);
        assert_eq!(system_ids_for_core("snes9x"), &["snes"]);
        assert_eq!(system_ids_for_core("snes9x2010"), &["snes"]);
        assert_eq!(system_ids_for_core("stella"), &["atari2600"]);
        assert_eq!(system_ids_for_core("mesen"), &["nes"]);
        assert_eq!(system_ids_for_core("mesen-s"), &["snes"]);
        assert!(system_ids_for_core("not_a_core").is_empty());
    }
}
