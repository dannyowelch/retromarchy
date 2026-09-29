//! Card-sized cover thumbs.
//!
//! The grid reads an in-memory bitmap. A miss is resized on the GPUI background
//! executor and stored under the XDG cache directory so the next launch skips
//! the full-size decode.

use gpui_kit::{DevicePixels, RenderImage};
use image::ImageEncoder;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::SystemTime;

const MAX_IN_FLIGHT: usize = 4;
const MAX_QUEUED: usize = 48;

static UI_DECODE_NS: AtomicU64 = AtomicU64::new(0);
static BG_DECODE_NS: AtomicU64 = AtomicU64::new(0);
static MEM_HITS: AtomicU64 = AtomicU64::new(0);
static DISK_HITS: AtomicU64 = AtomicU64::new(0);
static GENERATED: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub struct ProfileStats {
    pub ui_decode_ns: u64,
    pub bg_decode_ns: u64,
    pub mem_hits: u64,
    pub disk_hits: u64,
    pub generated: u64,
}

pub fn profile_stats() -> ProfileStats {
    ProfileStats {
        ui_decode_ns: UI_DECODE_NS.load(Ordering::Relaxed),
        bg_decode_ns: BG_DECODE_NS.load(Ordering::Relaxed),
        mem_hits: MEM_HITS.load(Ordering::Relaxed),
        disk_hits: DISK_HITS.load(Ordering::Relaxed),
        generated: GENERATED.load(Ordering::Relaxed),
    }
}

pub fn reset_profile() {
    UI_DECODE_NS.store(0, Ordering::Relaxed);
    BG_DECODE_NS.store(0, Ordering::Relaxed);
    MEM_HITS.store(0, Ordering::Relaxed);
    DISK_HITS.store(0, Ordering::Relaxed);
    GENERATED.store(0, Ordering::Relaxed);
}

/// Which scraped file the thumb was made from. Box art and screenshots do not share entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThumbKind {
    BoxArt,
    Screenshot,
}

impl ThumbKind {
    pub fn from_media(kind: crate::types::MediaKind) -> Option<Self> {
        match kind {
            crate::types::MediaKind::BoxArt => Some(Self::BoxArt),
            crate::types::MediaKind::Screenshot => Some(Self::Screenshot),
            _ => None,
        }
    }

    fn tag(self) -> u8 {
        match self {
            Self::BoxArt => 1,
            Self::Screenshot => 2,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::BoxArt => "box",
            Self::Screenshot => "shot",
        }
    }
}

/// Identity of one cached bitmap. A new mtime, length, kind, or pixel size is a different file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub source: PathBuf,
    pub modified_ns: u128,
    pub len: u64,
    pub kind: ThumbKind,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SourceStamp {
    modified_ns: u128,
    len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct WorkId {
    path: PathBuf,
    kind: ThumbKind,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug)]
pub struct Work {
    path: PathBuf,
    kind: ThumbKind,
    width: u32,
    height: u32,
    epoch: u64,
}

impl Work {
    fn id(&self) -> WorkId {
        WorkId {
            path: self.path.clone(),
            kind: self.kind,
            width: self.width,
            height: self.height,
        }
    }
}

pub struct JobResult {
    id: WorkId,
    epoch: u64,
    outcome: Option<(ThumbKey, Arc<RenderImage>)>,
}

#[derive(Default)]
struct Cache {
    images: HashMap<ThumbKey, Arc<RenderImage>>,
    disks: HashMap<PathBuf, Vec<ThumbKey>>,
    stamps: HashMap<PathBuf, SourceStamp>,
    epochs: HashMap<PathBuf, u64>,
    failed: HashSet<WorkId>,
    inflight: HashSet<WorkId>,
    wanted: VecDeque<Work>,
    pending: Vec<Work>,
    live_slot: Option<(u32, u32)>,
}

/// Device pixels the card slot occupies. Scale is the window factor.
pub fn slot_px(slot_w: f32, slot_h: f32, scale: f32) -> (u32, u32) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let width = (slot_w.max(1.0) * scale).round().clamp(1.0, 8192.0) as u32;
    let height = (slot_h.max(1.0) * scale).round().clamp(1.0, 8192.0) as u32;
    (width.max(1), height.max(1))
}

pub fn lookup(path: &Path, kind: ThumbKind, width: u32, height: u32) -> Option<Arc<RenderImage>> {
    let image = lock().lookup(path, kind, width, height)?;
    MEM_HITS.fetch_add(1, Ordering::Relaxed);
    Some(image)
}

pub fn begin_visible() {
    lock().pending.clear();
}

pub fn note_visible(path: PathBuf, kind: ThumbKind, width: u32, height: u32) {
    lock().pending.push(Work {
        path,
        kind,
        width,
        height,
        epoch: 0,
    });
}

pub fn end_visible() {
    let mut cache = lock();
    let pending = std::mem::take(&mut cache.pending);
    cache.set_visible(pending);
}

pub fn pop_work() -> Option<Work> {
    lock().pop()
}

pub fn produce(work: Work) -> JobResult {
    produce_in(&thumb_cache_dir(), work)
}

pub fn complete(result: JobResult) -> bool {
    lock().complete(result)
}

/// Drop memory and disk thumbs for `path` so the next view stats the file again.
pub fn invalidate(path: &Path) {
    let mut cache = lock();
    let dir = thumb_cache_dir();
    cache.invalidate_at(&dir, path);
}

/// Drop a memory entry when the file's mtime or length changed. Unchanged files stay put.
pub fn invalidate_stale(path: &Path) {
    let current = read_stamp(path);
    let mut cache = lock();
    if cache.stamps.get(path).copied() == current {
        return;
    }
    let dir = thumb_cache_dir();
    cache.invalidate_at(&dir, path);
}

pub fn device_len(rendered: &RenderImage) -> (DevicePixels, DevicePixels) {
    let size = rendered.size(0);
    (size.width, size.height)
}

fn global() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

fn lock() -> MutexGuard<'static, Cache> {
    global().lock().unwrap_or_else(|err| err.into_inner())
}

impl Cache {
    fn lookup(
        &self,
        path: &Path,
        kind: ThumbKind,
        width: u32,
        height: u32,
    ) -> Option<Arc<RenderImage>> {
        let stamp = self.stamps.get(path).copied()?;
        let key = ThumbKey {
            source: path.to_path_buf(),
            modified_ns: stamp.modified_ns,
