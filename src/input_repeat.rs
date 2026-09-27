//! Hold-to-repeat for one direction.
//!
//! The first poll while a direction is held emits one step. The next step waits
//! [`InputSettings::initial_delay_ms`]. Later gaps start at
//! [`InputSettings::slow_interval_ms`] and reach
//! [`InputSettings::fast_interval_ms`] after [`InputSettings::ramp_ms`] from
//! that first repeat. Releasing, or switching direction, resets the clock.
//! Each poll returns at most one step so a late tick cannot skip ahead.
//!
//! [`HoldRepeat`] is the two-axis clock the shell ticks. Arrow keys and the
//! gamepad share it. A fresh arrow steps immediately; later key-repeat events
//! only keep that arrow down. A held arrow wins over the pad on the same axis.

use crate::config::InputSettings;
use crate::gamepad::NavDir;

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

/// Arrow keys plus one [`DirectionRepeat`] per axis.
///
/// Keyboard Y is independent of the d-pad's sign: negative is Up, positive is Down.
/// The pad directions passed to [`HoldRepeat::poll`] are already [`NavDir`]s.
#[derive(Debug, Default)]
pub struct HoldRepeat {
    key_x: AxisHold,
    key_y: AxisHold,
    repeat_x: DirectionRepeat<NavDir>,
    repeat_y: DirectionRepeat<NavDir>,
}

impl HoldRepeat {
    /// Returns how many steps to apply now. A key-repeat event for a key that
    /// is already down returns 0 so the clock, not the OS repeat, emits the rest.
    pub fn press(&mut self, settings: &InputSettings, dir: NavDir, now_ms: u64) -> u32 {
        let (axis, repeat, side) = self.axis_mut(dir);
        let fresh = !axis.is_down(side);
        axis.set(side, true);
        if !fresh {
            return 0;
        }
        repeat.poll(settings, Some(dir), now_ms)
    }

    pub fn release(&mut self, settings: &InputSettings, dir: NavDir, now_ms: u64) {
        let (axis, repeat, side) = self.axis_mut(dir);
        let was_held = axis.held().is_some();
        axis.set(side, false);
        if was_held && axis.held().is_none() {
            repeat.poll(settings, None, now_ms);
        }
    }

    /// One tick. Each returned direction is a single step to apply.
    /// `None` means that axis does not move on this tick.
    pub fn poll(
        &mut self,
        settings: &InputSettings,
        pad_x: Option<NavDir>,
        pad_y: Option<NavDir>,
        now_ms: u64,
    ) -> (Option<NavDir>, Option<NavDir>) {
        let held_x = match self.key_x.held() {
            Some(AxisSide::Negative) => Some(NavDir::Left),
            Some(AxisSide::Positive) => Some(NavDir::Right),
            None => pad_x,
        };
        let held_y = match self.key_y.held() {
            Some(AxisSide::Negative) => Some(NavDir::Up),
            Some(AxisSide::Positive) => Some(NavDir::Down),
            None => pad_y,
        };
        let step_x = self.repeat_x.poll(settings, held_x, now_ms);
        let step_y = self.repeat_y.poll(settings, held_y, now_ms);
        (held_x.filter(|_| step_x > 0), held_y.filter(|_| step_y > 0))
    }

    fn axis_mut(&mut self, dir: NavDir) -> (&mut AxisHold, &mut DirectionRepeat<NavDir>, AxisSide) {
        match dir {
            NavDir::Left => (&mut self.key_x, &mut self.repeat_x, AxisSide::Negative),
            NavDir::Right => (&mut self.key_x, &mut self.repeat_x, AxisSide::Positive),
            NavDir::Up => (&mut self.key_y, &mut self.repeat_y, AxisSide::Negative),
            NavDir::Down => (&mut self.key_y, &mut self.repeat_y, AxisSide::Positive),
        }
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
    fn hold_repeat_steps_once_on_press_and_then_on_the_clock() {
        let settings = settings();
        let mut hold = HoldRepeat::default();
        assert_eq!(hold.press(&settings, NavDir::Right, 0), 1);
        assert_eq!(hold.press(&settings, NavDir::Right, 32), 0);
        assert_eq!(hold.poll(&settings, None, None, 32), (None, None));
        assert_eq!(
            hold.poll(&settings, None, None, 400),
            (Some(NavDir::Right), None)
        );
        hold.release(&settings, NavDir::Right, 450);
        assert_eq!(hold.poll(&settings, None, None, 450), (None, None));
        assert_eq!(hold.press(&settings, NavDir::Right, 450), 1);
    }

    #[test]
    fn a_held_arrow_wins_over_the_pad_on_that_axis() {
        let settings = settings();
        let mut hold = HoldRepeat::default();
        assert_eq!(hold.press(&settings, NavDir::Up, 0), 1);
        assert_eq!(
            hold.poll(&settings, Some(NavDir::Right), Some(NavDir::Down), 0),
            (Some(NavDir::Right), None)
        );
        assert_eq!(
            hold.poll(&settings, Some(NavDir::Right), Some(NavDir::Down), 16),
            (None, None)
        );
    }

    #[test]
    fn releasing_an_unheld_arrow_leaves_a_pad_hold_on_its_clock() {
        let settings = settings();
        let mut hold = HoldRepeat::default();
        assert_eq!(
            hold.poll(&settings, Some(NavDir::Right), None, 0).0,
            Some(NavDir::Right)
        );
        hold.release(&settings, NavDir::Left, 50);
        assert_eq!(hold.poll(&settings, Some(NavDir::Right), None, 50).0, None);
        assert_eq!(
            hold.poll(&settings, Some(NavDir::Right), None, 400).0,
            Some(NavDir::Right)
        );
    }

    #[test]
    fn the_pause_follows_input_settings() {
        let settings = InputSettings {
            initial_delay_ms: 100,
            slow_interval_ms: 80,
            fast_interval_ms: 40,
            ramp_ms: 0,
        };
        let mut hold = HoldRepeat::default();
        assert_eq!(
            hold.poll(&settings, Some(NavDir::Down), None, 0).0,
            Some(NavDir::Down)
        );
        assert_eq!(hold.poll(&settings, Some(NavDir::Down), None, 99).0, None);
        assert_eq!(
            hold.poll(&settings, Some(NavDir::Down), None, 100).0,
            Some(NavDir::Down)
        );
        assert_eq!(hold.poll(&settings, Some(NavDir::Down), None, 139).0, None);
        assert_eq!(
            hold.poll(&settings, Some(NavDir::Down), None, 140).0,
            Some(NavDir::Down)
        );
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
