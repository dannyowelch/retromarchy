use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
pub struct SystemEntry {
    pub folder_id: String,
    pub display_name: String,
    pub extensions: Vec<String>,
}

static CATALOG: OnceLock<Vec<SystemEntry>> = OnceLock::new();

pub fn systems() -> &'static [SystemEntry] {
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/systems.json"))
            .expect("bundled systems catalog is valid JSON")
    })
}

pub fn by_id() -> HashMap<&'static str, &'static SystemEntry> {
    systems().iter().map(|s| (s.folder_id.as_str(), s)).collect()
}

const POPULAR: &[&str] = &[
    "nes", "snes", "n64", "gc", "wii", "gb", "gbc", "gba", "nds", "n3ds", "genesis", "megadrive",
    "mastersystem", "gamegear", "sega32x", "segacd", "saturn", "dreamcast", "neogeo", "ngp", "ngpc",
    "psx", "ps2", "psp", "atari2600", "atari7800", "atarilynx", "pcengine", "tg16", "wonderswan",
    "virtualboy", "xbox", "switch",
];

pub fn ordered() -> Vec<&'static SystemEntry> {
    let all = systems();
    let mut out = Vec::with_capacity(all.len());
    for id in POPULAR {
        if let Some(entry) = all.iter().find(|s| s.folder_id == *id) {
            out.push(entry);
        }
    }
    let mut rest: Vec<_> = all.iter().filter(|s| !POPULAR.contains(&s.folder_id.as_str())).collect();
    rest.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    out.extend(rest);
    out
}

pub fn match_folder(name: &str) -> Option<&'static SystemEntry> {
    let key = name.to_ascii_lowercase();
    if let Some(entry) = systems().iter().find(|s| s.folder_id.eq_ignore_ascii_case(&key)) {
        return Some(entry);
    }
    let alias = match key.as_str() {
        "md" | "smd" | "mega drive" => "megadrive",
        "gen" => "genesis",
        "sms" => "mastersystem",
        "gg" => "gamegear",
        "ps1" | "psone" | "playstation" => "psx",
        "playstation2" => "ps2",
        "sfc" | "supernintendo" | "super nintendo" => "snes",
        "gcn" | "gamecube" => "gc",
        "3ds" => "n3ds",
        "ds" => "nds",
        "lynx" => "atarilynx",
        "vb" => "virtualboy",
        "ws" => "wonderswan",
        "wsc" => "wonderswancolor",
        "megacd" => "segacd",
        "32x" => "sega32x",
        _ => return None,
    };
    systems().iter().find(|s| s.folder_id == alias)
}
