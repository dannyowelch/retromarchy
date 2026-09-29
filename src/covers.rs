//! Box art and screenshots decoded once at the size the grid actually draws.

use gpui_kit::{DevicePixels, RenderImage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

fn cache() -> &'static Mutex<HashMap<(PathBuf, u32, u32), Arc<RenderImage>>> {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u32, u32), Arc<RenderImage>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Decode `path` once and keep a copy that fits inside the slot.
/// Later frames reuse that bitmap instead of the full file.
pub fn grid_art(path: &Path, slot_w: f32, slot_h: f32, scale: f32) -> Option<Arc<RenderImage>> {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let slot_w = slot_w.max(1.0) * scale;
    let slot_h = slot_h.max(1.0) * scale;
    let (src_w, src_h) = image::image_dimensions(path).ok()?;
    if src_w == 0 || src_h == 0 {
        return None;
    }
    let src_aspect = src_w as f32 / src_h as f32;
    let slot_aspect = slot_w / slot_h;
    let (fit_w, fit_h) = if src_aspect > slot_aspect {
        (slot_w, slot_w / src_aspect)
    } else {
        (slot_h * src_aspect, slot_h)
    };
    let width = fit_w.round().clamp(1.0, slot_w) as u32;
    let height = fit_h.round().clamp(1.0, slot_h) as u32;
    let key = (path.to_path_buf(), width, height);
    {
        let cache = cache().lock().unwrap_or_else(|err| err.into_inner());
        if let Some(hit) = cache.get(&key) {
            return Some(hit.clone());
        }
    }
    let source = image::open(path).ok()?.into_rgba8();
    let resized = image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    let mut bgra = resized;
    for pixel in bgra.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let frame = image::Frame::from_parts(bgra, 0, 0, image::Delay::from_numer_denom_ms(0, 1));
    let rendered = Arc::new(RenderImage::new(vec![frame]));
    let mut cache = cache().lock().unwrap_or_else(|err| err.into_inner());
    cache.insert(key, rendered.clone());
    Some(rendered)
}

pub fn device_len(rendered: &RenderImage) -> (DevicePixels, DevicePixels) {
    let size = rendered.size(0);
    (size.width, size.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_art_fits_the_slot_and_is_cached() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("box.png");
        let source = image::RgbaImage::from_pixel(1000, 1400, image::Rgba([10, 20, 30, 255]));
        source.save(&path).unwrap();
        let first = grid_art(&path, 216.0, 288.0, 1.0).unwrap();
        let (width, height) = device_len(&first);
        assert!(width.0 <= 216, "{width:?}");
        assert!(height.0 <= 288, "{height:?}");
        assert!(width.0 >= 200, "{width:?}");
        assert!(height.0 >= 270, "{height:?}");
        let second = grid_art(&path, 216.0, 288.0, 1.0).unwrap();
        assert_eq!(first.id, second.id);
    }
}
