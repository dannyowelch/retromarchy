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
        self.note_disk(&key);
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

    fn invalidate_at(&mut self, dir: &Path, path: &Path) {
        self.forget_disk(dir, path);
        self.invalidate(path);
    }

    fn forget_disk(&mut self, dir: &Path, path: &Path) {
        let Some(keys) = self.disks.remove(path) else {
            return;
        };
        for key in keys {
            let _ = fs::remove_file(thumb_path(dir, &key));
        }
    }

    fn note_disk(&mut self, key: &ThumbKey) {
        let keys = self.disks.entry(key.source.clone()).or_default();
        if !keys.iter().any(|have| have == key) {
            keys.push(key.clone());
        }
    }

    fn invalidate(&mut self, path: &Path) {
        let epoch = self.epochs.entry(path.to_path_buf()).or_insert(0);
        *epoch = epoch.wrapping_add(1);
        self.stamps.remove(path);
        self.disks.remove(path);
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
