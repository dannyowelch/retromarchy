//! GTK Options → Input. Four millisecond spins, saved to `[input]` as they change.
//!
//! The window title is Options. The group is Input. Each spin steps by 10 and
//! stops at 60_000. Starting pause and the transition may be 0. The repeat
//! intervals may not. [`InputSettings::sanitize`] still pulls a fast repeat
//! down when it would outrun the slow one, and the row shows that result.

use crate::config::InputSettings;
use crate::gamepad::NavDir;

/// GTK spin step. `with_range(..., 10.0)`.
pub const STEP_MS: u32 = 10;
/// GTK spin upper bound, and the cap in [`InputSettings::sanitize`].
pub const MAX_MS: u32 = 60_000;

pub const SECTION: &str = "Input";
pub const INTRO: &str = "Arrow keys, d-pad, and left stick. Saved to config.toml as you edit. Confirm, Back, and Favorite do not repeat.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Starting,
    Slow,
    Fast,
    Ramp,
    Close,
}

const ORDER: [Slot; 5] = [
    Slot::Starting,
    Slot::Slow,
    Slot::Fast,
    Slot::Ramp,
    Slot::Close,
];

struct Field {
    slot: Slot,
    title: &'static str,
    subtitle: &'static str,
    min: u32,
}

const FIELDS: [Field; 4] = [
    Field {
        slot: Slot::Starting,
        title: "Starting pause",
        subtitle: "Milliseconds before the first repeat",
        min: 0,
    },
    Field {
        slot: Slot::Slow,
        title: "Slow repeat",
        subtitle: "Milliseconds between steps at first",
        min: 1,
    },
    Field {
        slot: Slot::Fast,
        title: "Fast repeat",
        subtitle: "Milliseconds between steps after the transition",
        min: 1,
    },
    Field {
        slot: Slot::Ramp,
        title: "Slow-to-fast transition",
        subtitle: "Milliseconds to ease from the slow repeat to the fast one",
        min: 0,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub slot: Slot,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub value: u32,
    pub aimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputOptions {
    input: InputSettings,
    focus: Slot,
}

impl InputOptions {
    pub fn open(input: InputSettings) -> Self {
        Self {
            input: input.sanitized(),
            focus: Slot::Starting,
        }
    }

    pub fn input(&self) -> InputSettings {
        self.input
    }

    pub fn focus(&self) -> Slot {
        self.focus
    }

    pub fn close_aimed(&self) -> bool {
        self.focus == Slot::Close
    }

    pub fn rows(&self) -> [Row; 4] {
        FIELDS.map(|field| Row {
            slot: field.slot,
            title: field.title,
            subtitle: field.subtitle,
            value: value_of(self.input, field.slot),
            aimed: self.focus == field.slot,
        })
    }

    pub fn aim(&mut self, slot: Slot) {
        self.focus = slot;
    }

    /// Up and down move. Left and right step the aimed spin by [`STEP_MS`].
    /// Returns whether `[input]` changed.
    pub fn move_dir(&mut self, dir: NavDir) -> bool {
        match dir {
            NavDir::Up => {
                self.shift(-1, false);
                false
            }
            NavDir::Down => {
                self.shift(1, false);
                false
            }
            NavDir::Left => self.step_focused(-1),
            NavDir::Right => self.step_focused(1),
        }
    }

    pub fn tab(&mut self, backward: bool) {
        self.shift(if backward { -1 } else { 1 }, true);
    }

    /// Aim `slot` and step it. A mouse click on − or + uses this.
    pub fn step(&mut self, slot: Slot, steps: i32) -> bool {
        self.focus = slot;
        self.step_focused(steps)
    }

    fn shift(&mut self, delta: isize, wrap: bool) {
        let pos = ORDER
            .iter()
            .position(|slot| *slot == self.focus)
            .unwrap_or(0);
        let len = ORDER.len() as isize;
        let next = pos as isize + delta;
        let next = if wrap {
            next.rem_euclid(len)
        } else if (0..len).contains(&next) {
            next
        } else {
            return;
        };
        self.focus = ORDER[next as usize];
    }

    fn step_focused(&mut self, steps: i32) -> bool {
        let Some(field) = FIELDS.iter().find(|field| field.slot == self.focus) else {
            return false;
        };
        if steps == 0 {
            return false;
        }
        let before = self.input;
        let next = step_ms(value_of(self.input, field.slot), field.min, steps);
        set_value(&mut self.input, field.slot, next);
        self.input.sanitize();
        self.input != before
    }
}

fn value_of(input: InputSettings, slot: Slot) -> u32 {
    match slot {
        Slot::Starting => input.initial_delay_ms,
        Slot::Slow => input.slow_interval_ms,
        Slot::Fast => input.fast_interval_ms,
        Slot::Ramp => input.ramp_ms,
        Slot::Close => 0,
    }
}

fn set_value(input: &mut InputSettings, slot: Slot, value: u32) {
    match slot {
        Slot::Starting => input.initial_delay_ms = value,
        Slot::Slow => input.slow_interval_ms = value,
        Slot::Fast => input.fast_interval_ms = value,
        Slot::Ramp => input.ramp_ms = value,
        Slot::Close => {}
    }
}

fn step_ms(value: u32, min: u32, steps: i32) -> u32 {
    let magnitude = steps.unsigned_abs().saturating_mul(STEP_MS);
    let stepped = if steps < 0 {
        value.saturating_sub(magnitude)
    } else {
        value.saturating_add(magnitude)
    };
    stepped.clamp(min, MAX_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_on_the_gtk_defaults() {
        let dialog = InputOptions::open(InputSettings::default());
        assert_eq!(dialog.focus(), Slot::Starting);
        let rows = dialog.rows();
        assert_eq!(rows[0].title, "Starting pause");
        assert_eq!(rows[0].value, 400);
        assert!(rows[0].aimed);
        assert_eq!(rows[1].title, "Slow repeat");
        assert_eq!(rows[1].value, 180);
        assert_eq!(rows[2].title, "Fast repeat");
        assert_eq!(rows[2].value, 50);
        assert_eq!(rows[3].title, "Slow-to-fast transition");
        assert_eq!(rows[3].value, 2000);
        assert!(rows.iter().all(|row| row.subtitle.contains("Milliseconds")));
    }

    #[test]
    fn steps_stay_inside_the_gtk_spin_range() {
        let mut dialog = InputOptions::open(InputSettings::default());
        assert!(dialog.move_dir(NavDir::Left));
        assert_eq!(dialog.input().initial_delay_ms, 390);
        dialog.aim(Slot::Starting);
        set_value(&mut dialog.input, Slot::Starting, 0);
        assert!(!dialog.move_dir(NavDir::Left));
        assert_eq!(dialog.input().initial_delay_ms, 0);
        dialog.aim(Slot::Starting);
        set_value(&mut dialog.input, Slot::Starting, MAX_MS);
        assert!(!dialog.move_dir(NavDir::Right));
        assert_eq!(dialog.input().initial_delay_ms, MAX_MS);

        dialog.aim(Slot::Slow);
        set_value(&mut dialog.input, Slot::Slow, 1);
        set_value(&mut dialog.input, Slot::Fast, 1);
        assert!(!dialog.step(Slot::Slow, -1));
        assert_eq!(dialog.input().slow_interval_ms, 1);
    }

    #[test]
    fn fast_repeat_cannot_outrun_slow_repeat() {
        let mut dialog = InputOptions::open(InputSettings {
            initial_delay_ms: 400,
            slow_interval_ms: 50,
            fast_interval_ms: 50,
            ramp_ms: 2000,
        });
        dialog.aim(Slot::Fast);
        assert!(!dialog.move_dir(NavDir::Right));
        assert_eq!(dialog.input().fast_interval_ms, 50);

        dialog.aim(Slot::Slow);
        assert!(dialog.move_dir(NavDir::Left));
        assert_eq!(dialog.input().slow_interval_ms, 40);
        assert_eq!(dialog.input().fast_interval_ms, 40);
    }

    #[test]
    fn arrows_move_and_tab_wraps() {
        let mut dialog = InputOptions::open(InputSettings::default());
        assert!(!dialog.move_dir(NavDir::Up));
        assert_eq!(dialog.focus(), Slot::Starting);
        dialog.move_dir(NavDir::Down);
        dialog.move_dir(NavDir::Down);
        assert_eq!(dialog.focus(), Slot::Fast);
        dialog.tab(false);
        dialog.tab(false);
        assert_eq!(dialog.focus(), Slot::Close);
        assert!(dialog.close_aimed());
        dialog.tab(false);
        assert_eq!(dialog.focus(), Slot::Starting);
        dialog.tab(true);
        assert_eq!(dialog.focus(), Slot::Close);
        assert!(!dialog.move_dir(NavDir::Left));
        assert_eq!(dialog.focus(), Slot::Close);
    }
}
