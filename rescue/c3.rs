    #[test]
    fn same_filename_and_stamp_drops_the_disk_thumb() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("box.png");
        image::RgbaImage::from_pixel(8, 10, image::Rgba([200, 10, 10, 255]))
            .save(&src)
            .unwrap();
        let file = fs::File::options().write(true).open(&src).unwrap();
        let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        file.set_modified(stamp).unwrap();
        drop(file);
        let len = fs::metadata(&src).unwrap().len();

        let thumbs = root.path().join("thumbs");
        let first = produce_in(&thumbs, work(&src, ThumbKind::BoxArt, 8, 10));
        let key = first.outcome.as_ref().unwrap().0.clone();
        let mut cache = Cache::default();
        assert!(cache.complete(first));

        image::RgbaImage::from_pixel(8, 10, image::Rgba([10, 180, 20, 255]))
            .save(&src)
            .unwrap();
        let file = fs::File::options().write(true).open(&src).unwrap();
        file.set_modified(stamp).unwrap();
        drop(file);
        assert_eq!(fs::metadata(&src).unwrap().len(), len);
        assert_eq!(
            read_stamp(&src).unwrap(),
            SourceStamp {
                modified_ns: 10_000 * 1_000_000_000,
                len,
            }
        );

        cache.invalidate_at(&thumbs, &src);
        assert!(!thumb_path(&thumbs, &key).is_file());
        let second = produce_in(&thumbs, work_at(&src, &cache));
        assert_eq!(
            cache_file_name(&second.outcome.as_ref().unwrap().0),
            cache_file_name(&key)
        );
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

    fn pixel(image: &RenderImage) -> [u8; 4] {
        image.as_bytes(0).unwrap()[0..4].try_into().unwrap()
    }

    fn work_at(path: &Path, cache: &Cache) -> Work {
        let mut job = work(path, ThumbKind::BoxArt, 8, 10);
        job.epoch = cache.epochs.get(path).copied().unwrap_or(0);
        job
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
