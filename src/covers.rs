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

/// Drop memory entries for `path` so the next view stats the file again.
pub fn invalidate(path: &Path) {
    lock().invalidate(path);
}

/// Drop a memory entry when the file's mtime or length changed. Unchanged files stay put.
pub fn invalidate_stale(path: &Path) {
    let current = read_stamp(path);
    let mut cache = lock();
    if cache.stamps.get(path).copied() == current {
        return;
    }
    cache.invalidate(path);
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
            len: stamp.len,
            kind,
            width,
            height,
        };
        self.images.get(&key).cloned()
    }

    fn set_visible(&mut self, works: Vec<Work>) {
        if let Some(first) = works.first() {
            let slot = (first.width, first.height);
            if self.live_slot != Some(slot) {
                self.live_slot = Some(slot);
                self.images
                    .retain(|key, _| key.width == slot.0 && key.height == slot.1);
            }
        }
        self.wanted.clear();
        for mut work in works {
            let id = work.id();
            if self
                .lookup(&work.path, work.kind, work.width, work.height)
                .is_some()
                || self.inflight.contains(&id)
                || self.failed.contains(&id)
            {
                continue;
            }
            if self.wanted.iter().any(|queued| queued.id() == id) {
                continue;
            }
            work.epoch = self.epochs.get(&work.path).copied().unwrap_or(0);
            self.wanted.push_back(work);
            if self.wanted.len() >= MAX_QUEUED {
                break;
            }
        }
    }

    fn pop(&mut self) -> Option<Work> {
        while self.inflight.len() < MAX_IN_FLIGHT {
            let mut work = self.wanted.pop_front()?;
            let id = work.id();
            if self.failed.contains(&id)
                || self
                    .lookup(&work.path, work.kind, work.width, work.height)
                    .is_some()
            {
                continue;
            }
            work.epoch = self.epochs.get(&work.path).copied().unwrap_or(0);
            self.inflight.insert(id);
            return Some(work);
        }
        None
    }

    fn complete(&mut self, result: JobResult) -> bool {
        self.inflight.remove(&result.id);
        let current = self.epochs.get(&result.id.path).copied().unwrap_or(0);
        if current != result.epoch {
            return false;
        }
        let Some((key, image)) = result.outcome else {
            self.failed.insert(result.id);
            return false;
        };
        if let Some((width, height)) = self.live_slot {
            if key.width != width || key.height != height {
                return false;
            }
        }
        self.failed.remove(&result.id);
        self.stamps.insert(
            key.source.clone(),
            SourceStamp {
                modified_ns: key.modified_ns,
                len: key.len,
            },
        );
        self.images.insert(key, image);
        true
    }

    fn invalidate(&mut self, path: &Path) {
        let epoch = self.epochs.entry(path.to_path_buf()).or_insert(0);
        *epoch = epoch.wrapping_add(1);
        self.stamps.remove(path);
        self.images.retain(|key, _| key.source != path);
        self.failed.retain(|id| id.path != path);
        self.wanted.retain(|work| work.path != path);
        self.pending.retain(|work| work.path != path);
    }
}

fn read_stamp(path: &Path) -> Option<SourceStamp> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let modified_ns = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(SourceStamp {
        modified_ns,
        len: meta.len(),
    })
}

pub fn thumb_cache_dir() -> PathBuf {
    xdg::BaseDirectories::with_prefix("retromarchy")
        .map(|dirs| dirs.get_cache_home().join("thumbs"))
        .unwrap_or_else(|_| std::env::temp_dir().join("retromarchy").join("thumbs"))
}

fn produce_in(dir: &Path, work: Work) -> JobResult {
    let id = work.id();
    let epoch = work.epoch;
    let Some(stamp) = read_stamp(&work.path) else {
        return JobResult {
            id,
            epoch,
            outcome: None,
        };
    };
    let key = ThumbKey {
        source: work.path.clone(),
        modified_ns: stamp.modified_ns,
        len: stamp.len,
        kind: work.kind,
        width: work.width,
        height: work.height,
    };
    if let Some(image) = read_thumb(dir, &key) {
        DISK_HITS.fetch_add(1, Ordering::Relaxed);
        return JobResult {
            id,
            epoch,
            outcome: Some((key, image)),
        };
    }
    let started = std::time::Instant::now();
    let Some(rgba) = resize_cover(&work.path, work.width, work.height) else {
        return JobResult {
            id,
            epoch,
            outcome: None,
        };
    };
    BG_DECODE_NS.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    GENERATED.fetch_add(1, Ordering::Relaxed);
    store_thumb(dir, &key, &rgba);
    JobResult {
        id,
        epoch,
        outcome: Some((key, render_image(rgba))),
    }
}

fn resize_cover(path: &Path, slot_w: u32, slot_h: u32) -> Option<image::RgbaImage> {
    let source = image::open(path).ok()?.into_rgba8();
    let (width, height) = fit_pixels(source.width(), source.height(), slot_w, slot_h);
    if width == source.width() && height == source.height() {
        return Some(source);
    }
    Some(image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::Triangle,
    ))
}

pub fn fit_pixels(src_w: u32, src_h: u32, slot_w: u32, slot_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (1, 1);
    }
    let slot_w = slot_w.max(1);
    let slot_h = slot_h.max(1);
    let src_aspect = src_w as f32 / src_h as f32;
    let slot_aspect = slot_w as f32 / slot_h as f32;
    let (fit_w, fit_h) = if src_aspect > slot_aspect {
        (slot_w as f32, slot_w as f32 / src_aspect)
    } else {
        (slot_h as f32 * src_aspect, slot_h as f32)
    };
    (
        fit_w.round().clamp(1.0, slot_w as f32) as u32,
        fit_h.round().clamp(1.0, slot_h as f32) as u32,
    )
}

fn render_image(image: image::RgbaImage) -> Arc<RenderImage> {
    let mut bgra = image;
    for pixel in bgra.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let frame = image::Frame::from_parts(bgra, 0, 0, image::Delay::from_numer_denom_ms(0, 1));
    Arc::new(RenderImage::new(vec![frame]))
}

fn thumb_ext() -> &'static str {
    "qoi"
}

fn encode_thumb(image: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    image::codecs::qoi::QoiEncoder::new(&mut buf)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(buf)
}

fn decode_thumb(bytes: &[u8]) -> Option<image::RgbaImage> {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Qoi)
        .ok()
        .map(|image| image.into_rgba8())
}

fn cache_file_name(key: &ThumbKey) -> String {
    format!(
        "{}-{}x{}-{:032x}.{}",
        key.kind.name(),
        key.width,
        key.height,
        hash_key(key),
        thumb_ext()
    )
}

fn hash_key(key: &ThumbKey) -> u128 {
    let mut hash = 0x6c62272e07bb014262b821756295c58d_u128;
    let prime = 0x0000000001000000000000000000013B_u128;
    let mut write = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u128::from(*byte);
            hash = hash.wrapping_mul(prime);
        }
    };
    write(key.source.to_string_lossy().as_bytes());
    write(&[0]);
    write(&key.modified_ns.to_le_bytes());
    write(&key.len.to_le_bytes());
    write(&[key.kind.tag()]);
    write(&key.width.to_le_bytes());
    write(&key.height.to_le_bytes());
    hash
}

fn thumb_path(dir: &Path, key: &ThumbKey) -> PathBuf {
    dir.join(cache_file_name(key))
}

fn store_thumb(dir: &Path, key: &ThumbKey, image: &image::RgbaImage) {
    let Some(bytes) = encode_thumb(image) else {
        return;
    };
    let _ = write_atomic(&thumb_path(dir, key), &bytes);
}

fn read_thumb(dir: &Path, key: &ThumbKey) -> Option<Arc<RenderImage>> {
    let bytes = fs::read(thumb_path(dir, key)).ok()?;
    Some(render_image(decode_thumb(&bytes)?))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{n}.tmp", std::process::id()));
    let tmp = path.with_file_name(name);
    let write = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if write.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn work(path: &Path, kind: ThumbKind, width: u32, height: u32) -> Work {
        Work {
            path: path.to_path_buf(),
            kind,
            width,
            height,
            epoch: 0,
        }
    }

    fn key_for(
        path: &Path,
        stamp: SourceStamp,
        kind: ThumbKind,
        width: u32,
        height: u32,
    ) -> ThumbKey {
        ThumbKey {
            source: path.to_path_buf(),
            modified_ns: stamp.modified_ns,
            len: stamp.len,
            kind,
            width,
            height,
        }
    }

    #[test]
    fn key_changes_with_path_mtime_size_kind_and_pixels() {
        let path = PathBuf::from("/games/box/chrono.png");
        let stamp = SourceStamp {
            modified_ns: 1_700_000_000_000_000_000,
            len: 120_000,
        };
        let base = key_for(&path, stamp, ThumbKind::BoxArt, 216, 288);
        let name = cache_file_name(&base);
        assert_eq!(name, cache_file_name(&base));
        assert!(name.starts_with("box-216x288-"));
        assert!(name.ends_with(&format!(".{}", thumb_ext())));

        let mut mtime = base.clone();
        mtime.modified_ns += 1;
        assert_ne!(mtime, base);
        assert_ne!(cache_file_name(&mtime), name);

        let mut len = base.clone();
        len.len += 1;
        assert_ne!(len, base);
        assert_ne!(cache_file_name(&len), name);

        let mut kind = base.clone();
        kind.kind = ThumbKind::Screenshot;
        assert_ne!(kind, base);
        assert_ne!(cache_file_name(&kind), name);
        assert!(cache_file_name(&kind).starts_with("shot-216x288-"));

        let mut width = base.clone();
        width.width += 10;
        assert_ne!(width, base);
        assert_ne!(cache_file_name(&width), name);

        let mut height = base.clone();
        height.height += 10;
        assert_ne!(height, base);
        assert_ne!(cache_file_name(&height), name);

        let mut other = base.clone();
        other.source = PathBuf::from("/games/shot/chrono.png");
        assert_ne!(other, base);
        assert_ne!(cache_file_name(&other), name);
    }

    #[test]
    fn rewritten_source_misses_the_old_thumb_and_keeps_the_file() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("box.png");
        image::RgbaImage::from_pixel(8, 10, image::Rgba([1, 2, 3, 255]))
            .save(&src)
            .unwrap();
        let file = fs::File::options().write(true).open(&src).unwrap();
        let early = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        file.set_modified(early).unwrap();
        drop(file);
        let stamp = read_stamp(&src).unwrap();
        assert_eq!(stamp.modified_ns, 10_000 * 1_000_000_000);

        let thumbs = root.path().join("thumbs");
        let key = key_for(&src, stamp, ThumbKind::BoxArt, 40, 50);
        let rgba = image::RgbaImage::from_pixel(4, 5, image::Rgba([9, 8, 7, 255]));
        store_thumb(&thumbs, &key, &rgba);
        let saved = decode_thumb(&fs::read(thumb_path(&thumbs, &key)).unwrap()).unwrap();
        assert_eq!(saved.as_raw(), rgba.as_raw());

        fs::write(&src, b"replaced-art-bytes-that-are-longer").unwrap();
        let file = fs::File::options().write(true).open(&src).unwrap();
        file.set_modified(early + Duration::from_secs(90)).unwrap();
        drop(file);
        let next = read_stamp(&src).unwrap();
        assert_ne!(next, stamp);
        let fresh = key_for(&src, next, ThumbKind::BoxArt, 40, 50);
        assert_ne!(cache_file_name(&fresh), cache_file_name(&key));
        assert!(fs::read(thumb_path(&thumbs, &fresh)).is_err());
        assert!(thumb_path(&thumbs, &key).is_file());
        let left: Vec<_> = fs::read_dir(&thumbs)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left.len(), 1);
        assert!(left
            .iter()
            .all(|name| !name.to_string_lossy().contains(".tmp")));
    }

    #[test]
    fn unchanged_stamp_keeps_the_memory_entry() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("box.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([3, 3, 3, 255]))
            .save(&src)
            .unwrap();
        let file = fs::File::options().write(true).open(&src).unwrap();
        let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(50_000);
        file.set_modified(stamp).unwrap();
        drop(file);
        let result = produce_in(
            &root.path().join("thumbs"),
            work(&src, ThumbKind::BoxArt, 8, 8),
        );
        assert!(complete(result));
        assert!(lookup(&src, ThumbKind::BoxArt, 8, 8).is_some());
        invalidate_stale(&src);
        assert!(lookup(&src, ThumbKind::BoxArt, 8, 8).is_some());

        fs::write(&src, b"rewritten-cover-bytes").unwrap();
        let file = fs::File::options().write(true).open(&src).unwrap();
        file.set_modified(stamp + Duration::from_secs(10)).unwrap();
        drop(file);
        invalidate_stale(&src);
        assert!(lookup(&src, ThumbKind::BoxArt, 8, 8).is_none());
    }

    #[test]
    fn fitted_pixels_stay_inside_the_slot() {
        assert_eq!(fit_pixels(1000, 1400, 216, 288), (206, 288));
        assert_eq!(fit_pixels(1280, 960, 216, 162), (216, 162));
        assert_eq!(fit_pixels(0, 10, 20, 20), (1, 1));
    }

    #[test]
    fn memory_hit_does_not_need_the_disk_file() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("shot.png");
        image::RgbaImage::from_pixel(30, 20, image::Rgba([4, 5, 6, 255]))
            .save(&src)
            .unwrap();
        let thumbs = root.path().join("thumbs");
        let job = work(&src, ThumbKind::Screenshot, 24, 16);
        let result = produce_in(&thumbs, job);
        assert!(thumb_path(&thumbs, &result.outcome.as_ref().unwrap().0).is_file());
        let mut cache = Cache::default();
        cache.live_slot = Some((24, 16));
        assert!(cache.complete(result));
        fs::remove_dir_all(&thumbs).unwrap();
        let hit = cache.lookup(&src, ThumbKind::Screenshot, 24, 16).unwrap();
        let again = cache.lookup(&src, ThumbKind::Screenshot, 24, 16).unwrap();
        assert_eq!(hit.id, again.id);
        assert!(cache.lookup(&src, ThumbKind::BoxArt, 24, 16).is_none());
        assert!(cache.lookup(&src, ThumbKind::Screenshot, 40, 16).is_none());
    }

    #[test]
    fn produce_fits_a_tall_cover_and_reloads_it() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("box.png");
        image::RgbaImage::from_pixel(100, 140, image::Rgba([10, 20, 30, 255]))
            .save(&src)
            .unwrap();
        let thumbs = root.path().join("thumbs");
        let first = produce_in(&thumbs, work(&src, ThumbKind::BoxArt, 50, 70));
        let (key, image) = first.outcome.as_ref().unwrap();
        let (width, height) = device_len(image);
        assert!(width.0 <= 50 && height.0 <= 70, "{width:?} {height:?}");
        assert!(width.0 >= 45 && height.0 >= 65, "{width:?} {height:?}");
        let second = produce_in(&thumbs, work(&src, ThumbKind::BoxArt, 50, 70));
        let (again, _) = second.outcome.unwrap();
        assert_eq!(again, *key);
        assert_eq!(
            fs::read_dir(&thumbs).unwrap().count(),
            1,
            "disk hit must not write a second file"
        );
    }

    #[test]
    fn visible_queue_drops_rows_that_scrolled_away() {
        let mut cache = Cache::default();
        let far = PathBuf::from("/roms/a.png");
        let near = PathBuf::from("/roms/b.png");
        cache.set_visible(vec![
            work(&far, ThumbKind::BoxArt, 10, 12),
            work(&near, ThumbKind::BoxArt, 10, 12),
        ]);
        cache.set_visible(vec![work(&near, ThumbKind::BoxArt, 10, 12)]);
        let next = cache.pop().unwrap();
        assert_eq!(next.path, near);
        assert!(cache.pop().is_none());
    }

    #[test]
    fn queue_keeps_visible_order_and_caps_concurrency() {
        let mut cache = Cache::default();
        let paths: Vec<_> = (0..6)
            .map(|n| PathBuf::from(format!("/roms/{n}.png")))
            .collect();
        cache.set_visible(
            paths
                .iter()
                .map(|path| work(path, ThumbKind::BoxArt, 8, 8))
                .collect(),
        );
        let mut started = Vec::new();
        while let Some(job) = cache.pop() {
            started.push(job.path.clone());
        }
        assert_eq!(started.len(), MAX_IN_FLIGHT);
        assert_eq!(started, paths[..MAX_IN_FLIGHT]);
        let done = JobResult {
            id: Work {
                path: started[0].clone(),
                kind: ThumbKind::BoxArt,
                width: 8,
                height: 8,
                epoch: 0,
            }
            .id(),
            epoch: 0,
            outcome: None,
        };
        assert!(!cache.complete(done));
        let next = cache.pop().unwrap();
        assert_eq!(next.path, paths[MAX_IN_FLIGHT]);
    }

    #[test]
    fn failed_and_inflight_cards_are_not_queued_again() {
        let mut cache = Cache::default();
        let path = PathBuf::from("/roms/bad.png");
        cache.set_visible(vec![work(&path, ThumbKind::Screenshot, 12, 9)]);
        let job = cache.pop().unwrap();
        cache.set_visible(vec![work(&path, ThumbKind::Screenshot, 12, 9)]);
        assert!(cache.pop().is_none());
        assert!(!cache.complete(JobResult {
            id: job.id(),
            epoch: job.epoch,
            outcome: None,
        }));
        cache.set_visible(vec![work(&path, ThumbKind::Screenshot, 12, 9)]);
        assert!(cache.pop().is_none());
    }

    #[test]
    fn invalidate_drops_an_in_flight_result() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("box.png");
        image::RgbaImage::from_pixel(6, 8, image::Rgba([1, 1, 1, 255]))
            .save(&src)
            .unwrap();
        let job = work(&src, ThumbKind::BoxArt, 6, 8);
        let mut cache = Cache::default();
        cache.live_slot = Some((6, 8));
        cache.set_visible(vec![job.clone()]);
        let started = cache.pop().unwrap();
        let result = produce_in(root.path(), started);
        cache.invalidate(&src);
        assert!(!cache.complete(result));
        assert!(cache.lookup(&src, ThumbKind::BoxArt, 6, 8).is_none());
        cache.set_visible(vec![work(&src, ThumbKind::BoxArt, 6, 8)]);
        assert!(cache.pop().is_some());
    }

    #[test]
    fn thumb_cache_dir_uses_xdg_cache_home() {
        let root = tempfile::tempdir().unwrap();
        let _env = crate::config::XdgEnv::sandbox(root.path());
        assert_eq!(
            thumb_cache_dir(),
            root.path().join("cache").join("retromarchy").join("thumbs")
        );
    }
}
