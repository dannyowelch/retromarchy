//! Hold-to-repeat for one direction.
//!
//! The first poll while a direction is held emits one step. The next step waits
//! [`InputSettings::initial_delay_ms`]. Later gaps start at
//! [`InputSettings::slow_interval_ms`] and reach
//! [`InputSettings::fast_interval_ms`] after [`InputSettings::ramp_ms`] from
//! that first repeat. Releasing, or switching direction, resets the clock.
//! Each poll returns at most one step so a late tick cannot skip ahead.

use crate::config::InputSettings;

/// Which side of an axis is down. The caller maps this to left/right or up/down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSide {
    Negative,
    Positive,
}

/// Buttons held on one axis. The most recently pressed side wins while both are down.
#[derive(Debug, Default, Clone, Copy)]
pub struct AxisHold {
    negative: bool,
    positive: bool,
    last: Option<AxisSide>,
}

impl AxisHold {
    pub fn set(&mut self, side: AxisSide, down: bool) {
        match side {
            AxisSide::Negative => self.negative = down,
            AxisSide::Positive => self.positive = down,
        }
        if down {
            self.last = Some(side);
            return;
        }
        if self.last != Some(side) {
            return;
        }
        self.last = match side {
            AxisSide::Negative if self.positive => Some(AxisSide::Positive),
            AxisSide::Positive if self.negative => Some(AxisSide::Negative),
            _ => None,
        };
    }

    pub fn held(self) -> Option<AxisSide> {
        match self.last {
            Some(AxisSide::Negative) if self.negative => Some(AxisSide::Negative),
            Some(AxisSide::Positive) if self.positive => Some(AxisSide::Positive),
            _ => None,
        }
    }

    pub fn is_down(self, side: AxisSide) -> bool {
        match side {
            AxisSide::Negative => self.negative,
            AxisSide::Positive => self.positive,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Armed<T> {
    dir: T,
    pressed_at_ms: u64,
    next_at_ms: u64,
}

#[derive(Debug)]
pub struct DirectionRepeat<T> {
    armed: Option<Armed<T>>,
}

impl<T> Default for DirectionRepeat<T> {
    fn default() -> Self {
        Self { armed: None }
    }
}

impl<T: Copy + PartialEq> DirectionRepeat<T> {
    /// `held` is the direction still down, or `None` when it has been released.
    /// `now_ms` is milliseconds on a monotonic clock.
    pub fn poll(&mut self, settings: &InputSettings, held: Option<T>, now_ms: u64) -> u32 {
        let Some(dir) = held else {
            self.armed = None;
            return 0;
        };
        let settings = settings.sanitized();
        if let Some(armed) = self.armed {
            if armed.dir == dir {
                if now_ms < armed.next_at_ms {
                    return 0;
                }
                let since_first = now_ms.saturating_sub(
                    armed
                        .pressed_at_ms
                        .saturating_add(u64::from(settings.initial_delay_ms)),
                );
                let interval = repeat_interval_ms(&settings, since_first);
                self.armed = Some(Armed {
                    dir,
                    pressed_at_ms: armed.pressed_at_ms,
                    next_at_ms: now_ms.saturating_add(interval),
                });
                return 1;
            }
        }
        self.armed = Some(Armed {
            dir,
            pressed_at_ms: now_ms,
            next_at_ms: now_ms.saturating_add(u64::from(settings.initial_delay_ms)),
        });
        1
    }
}

/// Gap after a repeat, given milliseconds since the first repeat (not the initial press).
pub fn repeat_interval_ms(settings: &InputSettings, since_first_repeat_ms: u64) -> u64 {
    let settings = settings.sanitized();
    let slow = u64::from(settings.slow_interval_ms);
    let fast = u64::from(settings.fast_interval_ms);
    let ramp = u64::from(settings.ramp_ms);
    if ramp == 0 || since_first_repeat_ms >= ramp {
        return fast;
    }
    let span = slow.saturating_sub(fast);
    slow - span.saturating_mul(since_first_repeat_ms) / ramp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamepad::NavDir;

    fn settings() -> InputSettings {
        InputSettings::default()
    }

    #[test]
    fn press_fires_once_and_waits_out_the_pause() {
        let mut repeat = DirectionRepeat::default();
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Right), 0), 1);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Right), 0), 0);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Right), 399), 0);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Right), 400), 1);
    }

    #[test]
    fn slow_gap_then_fast_after_the_ramp() {
        let settings = settings();
        assert_eq!(repeat_interval_ms(&settings, 0), 180);
        assert_eq!(repeat_interval_ms(&settings, 1000), 115);
        assert_eq!(repeat_interval_ms(&settings, 2000), 50);
        assert_eq!(repeat_interval_ms(&settings, 5000), 50);

        let mut repeat = DirectionRepeat::default();
        assert_eq!(repeat.poll(&settings, Some(NavDir::Left), 0), 1);
        assert_eq!(repeat.poll(&settings, Some(NavDir::Left), 400), 1);
        assert_eq!(repeat.poll(&settings, Some(NavDir::Left), 579), 0);
        assert_eq!(repeat.poll(&settings, Some(NavDir::Left), 580), 1);

        let mut repeat = DirectionRepeat::default();
        let mut last = None;
        let mut gaps_after_ramp = Vec::new();
        for now in 0..4500 {
            if repeat.poll(&settings, Some(NavDir::Right), now) == 1 {
                if let Some(prev) = last {
                    if prev >= 400 + 2000 {
                        gaps_after_ramp.push(now - prev);
                    }
                }
                last = Some(now);
            }
        }
        assert!(!gaps_after_ramp.is_empty());
        assert!(
            gaps_after_ramp.iter().all(|gap| *gap == 50),
            "{gaps_after_ramp:?}"
        );
    }

    #[test]
    fn release_resets_and_a_new_press_fires_immediately() {
        let mut repeat = DirectionRepeat::default();
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Up), 0), 1);
        assert_eq!(repeat.poll(&settings(), None, 1000), 0);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Up), 1000), 1);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Up), 1399), 0);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Up), 1400), 1);
    }

    #[test]
    fn changing_direction_restarts_the_pause() {
        let mut repeat = DirectionRepeat::default();
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Left), 0), 1);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Right), 50), 1);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Right), 449), 0);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Right), 450), 1);
    }

    #[test]
    fn a_late_tick_does_not_burst() {
        let mut repeat = DirectionRepeat::default();
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Down), 0), 1);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Down), 10_000), 1);
        assert_eq!(repeat.poll(&settings(), Some(NavDir::Down), 10_000), 0);
    }

    #[test]
    fn zero_ramp_uses_the_fast_interval_immediately() {
        let settings = InputSettings {
            ramp_ms: 0,
            ..InputSettings::default()
        };
        assert_eq!(repeat_interval_ms(&settings, 0), 50);
    }

    #[test]
    fn opposite_sides_keep_the_latest_press() {
        let mut axis = AxisHold::default();
        axis.set(AxisSide::Negative, true);
        assert_eq!(axis.held(), Some(AxisSide::Negative));
        axis.set(AxisSide::Positive, true);
        assert_eq!(axis.held(), Some(AxisSide::Positive));
        axis.set(AxisSide::Positive, false);
        assert_eq!(axis.held(), Some(AxisSide::Negative));
        axis.set(AxisSide::Negative, false);
        assert_eq!(axis.held(), None);
    }

    #[test]
    fn a_repeat_event_for_a_key_already_down_is_not_a_fresh_press() {
        let mut axis = AxisHold::default();
        assert!(!axis.is_down(AxisSide::Negative));
        axis.set(AxisSide::Negative, true);
        axis.set(AxisSide::Positive, true);
        assert!(axis.is_down(AxisSide::Negative));
        assert_eq!(axis.held(), Some(AxisSide::Positive));
    }
}
