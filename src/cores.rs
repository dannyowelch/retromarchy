use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A libretro `.so` found on disk. The Systems tab lists these as core choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredCore {
    /// File name with `_libretro` and `.so` removed (`stella_libretro.so` → `stella`).
    pub name: String,
    pub path: PathBuf,
}

impl DiscoveredCore {
    pub fn from_path(path: PathBuf) -> Option<Self> {
        let file_name = path.file_name()?.to_str()?;
        let name = display_name(file_name)?;
        Some(Self { name, path })
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

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
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
}
