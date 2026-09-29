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
    fn overwritten_file_reloads_thumb_pixels() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("box.png");
        image::RgbaImage::from_pixel(8, 10, image::Rgba([200, 10, 10, 255]))
            .save(&src)
            .unwrap();
        let file = fs::File::options().write(true).open(&src).unwrap();
        let early = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        file.set_modified(early).unwrap();
        drop(file);

        let thumbs = root.path().join("thumbs");
        let first = produce_in(&thumbs, work(&src, ThumbKind::BoxArt, 8, 10));
        let old_key = first.outcome.as_ref().unwrap().0.clone();
        let mut cache = Cache::default();
        assert!(cache.complete(first));
        assert_eq!(
            pixel(
                cache
                    .lookup(&src, ThumbKind::BoxArt, 8, 10)
                    .unwrap()
                    .as_ref()
            ),
            [10, 10, 200, 255]
        );

        image::RgbaImage::from_pixel(8, 10, image::Rgba([10, 180, 20, 255]))
            .save(&src)
            .unwrap();
        let file = fs::File::options().write(true).open(&src).unwrap();
        file.set_modified(early + Duration::from_secs(90)).unwrap();
        drop(file);
        cache.invalidate_at(&thumbs, &src);
        assert!(cache.lookup(&src, ThumbKind::BoxArt, 8, 10).is_none());

        let second = produce_in(&thumbs, work_at(&src, &cache));
        let new_key = second.outcome.as_ref().unwrap().0.clone();
        assert_ne!(cache_file_name(&new_key), cache_file_name(&old_key));
        assert!(cache.complete(second));
        assert_eq!(
            pixel(
                cache
                    .lookup(&src, ThumbKind::BoxArt, 8, 10)
                    .unwrap()
                    .as_ref()
            ),
            [20, 180, 10, 255]
        );
    }

