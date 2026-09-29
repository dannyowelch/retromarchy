//! Scroll-cost probe. Enabled with `RETROMARCHY_PROFILE=1`.
//! Prints CPU time from `/proc/self/stat` for a stationary redraw and for
//! scripted fast and slow grid scrolls, then quits.

use gpui_kit::{point, px, App, ScrollHandle, Window};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

static TILES: AtomicUsize = AtomicUsize::new(0);
static SHOWN: AtomicUsize = AtomicUsize::new(0);
static PASSES: AtomicUsize = AtomicUsize::new(0);

pub fn enabled() -> bool {
    std::env::var_os("RETROMARCHY_PROFILE").is_some()
}

pub fn begin_frame() {
    if enabled() {
        // Tiles are built during prepaint, after the previous frame was counted.
        SHOWN.store(TILES.swap(0, Ordering::Relaxed), Ordering::Relaxed);
        PASSES.store(0, Ordering::Relaxed);
    }
}

pub fn shown_tiles() -> usize {
    SHOWN.load(Ordering::Relaxed)
}

/// The grid measures one row before it paints the visible rows. Only the later
/// passes are on-screen tiles.
pub fn count_pass() -> bool {
    if !enabled() {
        return false;
    }
    PASSES.fetch_add(1, Ordering::Relaxed) > 0
}

pub fn count_tile() {
    if enabled() {
        TILES.fetch_add(1, Ordering::Relaxed);
    }
}

struct Probe {
    stage: Stage,
    stage_frames: u32,
    stage_started: Instant,
    cpu_started: u64,
    last_frame: Instant,
    frame_ms: f64,
}

enum Stage {
    Warmup,
    Still,
    Fast,
    Slow,
}

pub fn after_frame(scroll: &ScrollHandle, window: &mut Window, cx: &mut App) {
    if !enabled() {
        return;
    }
    static PROBE: std::sync::Mutex<Option<Probe>> = std::sync::Mutex::new(None);
    let mut guard = PROBE.lock().unwrap_or_else(|err| err.into_inner());
    let probe = guard.get_or_insert_with(|| Probe {
        stage: Stage::Warmup,
        stage_frames: 0,
        stage_started: Instant::now(),
        cpu_started: cpu_ms(),
        last_frame: Instant::now(),
        frame_ms: 0.0,
    });
    let now = Instant::now();
    if probe.stage_frames > 0 {
        probe.frame_ms += now.duration_since(probe.last_frame).as_secs_f64() * 1000.0;
    }
    probe.last_frame = now;
    probe.stage_frames += 1;
    let tiles = shown_tiles();

    let advance = match probe.stage {
        Stage::Warmup => probe.stage_started.elapsed().as_secs_f32() >= warmup_secs(),
        Stage::Still => probe.stage_frames > 60,
        Stage::Fast | Stage::Slow => probe.stage_frames > 120,
    };
    if advance {
        report(probe, tiles, scroll);
        probe.stage = match probe.stage {
            Stage::Warmup => Stage::Still,
            Stage::Still => Stage::Fast,
            Stage::Fast => Stage::Slow,
            Stage::Slow => {
                eprintln!("PROFILE done");
                cx.quit();
                return;
            }
        };
        probe.stage_frames = 0;
        probe.stage_started = Instant::now();
        probe.cpu_started = cpu_ms();
        probe.frame_ms = 0.0;
    }

    if matches!(probe.stage, Stage::Fast | Stage::Slow) {
        let step = match probe.stage {
            Stage::Fast => 48.0,
            Stage::Slow => 2.0,
            _ => 0.0,
        };
        nudge(scroll, step);
    }
    window.request_animation_frame();
}

fn warmup_secs() -> f32 {
    std::env::var("RETROMARCHY_PROFILE_WARMUP_SECS")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(12.0)
}

fn report(probe: &Probe, tiles: usize, scroll: &ScrollHandle) {
    let elapsed = probe.stage_started.elapsed().as_secs_f64() * 1000.0;
    let cpu = cpu_ms().saturating_sub(probe.cpu_started);
    let frames = probe.stage_frames.max(1);
    let name = match probe.stage {
        Stage::Warmup => "warmup",
        Stage::Still => "still",
        Stage::Fast => "scroll_fast",
        Stage::Slow => "scroll_slow",
    };
    eprintln!(
        "PROFILE phase={name} frames={frames} elapsed_ms={elapsed:.0} cpu_ms={cpu} cpu_per_frame_ms={:.2} avg_frame_ms={:.2} tiles={tiles} rss_kb={} offset={:.0} max_scroll={:.0}",
        cpu as f64 / frames as f64,
        probe.frame_ms / frames.saturating_sub(1).max(1) as f64,
        rss_kb(),
        -scroll.offset().y.as_f32(),
        scroll.max_offset().y.as_f32(),
    );
    let _ = std::io::Write::flush(&mut std::io::stderr());
}

fn nudge(scroll: &ScrollHandle, step: f32) {
    let max = scroll.max_offset().y.as_f32();
    if max <= 1.0 {
        return;
    }
    let current = -scroll.offset().y.as_f32();
    let mut next = current + step;
    if next >= max {
        next = 0.0;
    }
    let y = if next <= 0.0 { px(0.) } else { px(-next) };
    scroll.set_offset(point(px(0.), y));
}

fn cpu_ms() -> u64 {
    let text = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let Some((_, rest)) = text.rsplit_once(')') else {
        return 0;
    };
    let mut fields = rest.split_whitespace();
    let utime = fields
        .nth(11)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let stime = fields
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    // Linux user HZ is 100 on this platform.
    (utime + stime).saturating_mul(10)
}

fn rss_kb() -> u64 {
    let text = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        }
    }
    0
}
