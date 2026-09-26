//! SDL / Xbox layout via gilrs. South confirms, East goes back, North (Y) toggles a favorite.
//! Select (Xbox Back / View) opens the game menu.
//!
//! Directions are holds, not edges. The d-pad and the left stick stay "down" until
//! released or returned near center. [`crate::input_repeat::DirectionRepeat`] decides
//! how many steps that hold emits. Confirm, Back, Favorite, and Menu stay one-shot.

use crate::input_repeat::{AxisHold, AxisSide};
use gilrs::{Axis, Button, EventType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadAction {
    Confirm,
    Back,
    Favorite,
    /// gilrs [`Button::Select`]: Xbox Back / View. Not East (B) and not South (A).
    Menu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickAxis {
    X,
    Y,
}

/// Which way the left stick is held.
///
/// A deflection past [`STICK_ON`] engages that direction and stays engaged until
/// the axis falls inside [`STICK_OFF`] (near center). Values between the two
/// thresholds are hysteresis: they do not start a hold and they do not cancel
/// one. X and Y are independent, so a diagonal can repeat on both axes.
/// This latch does not emit steps; the repeat driver does.
#[derive(Debug, Default)]
pub struct StickLatch {
    x: Option<i8>,
    y: Option<i8>,
}

const STICK_ON: f32 = 0.55;
const STICK_OFF: f32 = 0.35;

impl StickLatch {
    pub fn horizontal(&self) -> Option<NavDir> {
        signed_dir(self.x, NavDir::Left, NavDir::Right)
    }

    pub fn vertical(&self) -> Option<NavDir> {
        // gilrs uses the SDL sign: positive left-stick Y is up.
        signed_dir(self.y, NavDir::Down, NavDir::Up)
    }
}

fn signed_dir(sign: Option<i8>, negative: NavDir, positive: NavDir) -> Option<NavDir> {
    match sign {
        Some(value) if value < 0 => Some(negative),
        Some(value) if value > 0 => Some(positive),
        _ => None,
    }
}

pub fn update_stick(latch: &mut StickLatch, axis: StickAxis, value: f32) {
    let slot = match axis {
        StickAxis::X => &mut latch.x,
        StickAxis::Y => &mut latch.y,
    };
    if !value.is_finite() {
        return;
    }
    if value >= STICK_ON {
        *slot = Some(1);
    } else if value <= -STICK_ON {
        *slot = Some(-1);
    } else if value.abs() < STICK_OFF {
        *slot = None;
    }
}

/// D-pad buttons and the left stick. Face buttons are edges; directions are holds.
/// On an axis, a held d-pad button wins over the stick.
#[derive(Debug, Default)]
pub struct PadHeld {
    stick: StickLatch,
    x: AxisHold,
    y: AxisHold,
}

impl PadHeld {
    /// `down` is a press. Releases clear d-pad holds and return nothing.
    /// South, East, North, and Select return an action only on press.
    pub fn apply_button(&mut self, button: Button, down: bool) -> Option<PadAction> {
        if !down {
            match button {
                Button::DPadLeft => self.x.set(AxisSide::Negative, false),
                Button::DPadRight => self.x.set(AxisSide::Positive, false),
                Button::DPadDown => self.y.set(AxisSide::Negative, false),
                Button::DPadUp => self.y.set(AxisSide::Positive, false),
                _ => {}
            }
            return None;
        }
        match button {
            Button::South => Some(PadAction::Confirm),
            Button::East => Some(PadAction::Back),
            Button::North => Some(PadAction::Favorite),
            Button::Select => Some(PadAction::Menu),
            Button::DPadLeft => {
                self.x.set(AxisSide::Negative, true);
                None
            }
            Button::DPadRight => {
                self.x.set(AxisSide::Positive, true);
                None
            }
            Button::DPadDown => {
                self.y.set(AxisSide::Negative, true);
                None
            }
            Button::DPadUp => {
                self.y.set(AxisSide::Positive, true);
                None
            }
            _ => None,
        }
    }

    /// Updates held directions. Directional events do not return [`PadAction::Move`];
    /// read [`Self::horizontal`] and [`Self::vertical`] and run them through the repeater.
    /// A gilrs `ButtonRepeated` keeps the button down without firing Confirm, Back, Favorite, or Menu.
    pub fn apply(&mut self, event: &EventType) -> Option<PadAction> {
        match event {
            EventType::ButtonPressed(button, _) => self.apply_button(*button, true),
            EventType::ButtonRepeated(button, _) => {
                let _ = self.apply_button(*button, true);
                None
            }
            EventType::ButtonReleased(button, _) => self.apply_button(*button, false),
            EventType::AxisChanged(axis, value, _) => {
                let stick_axis = match axis {
                    Axis::LeftStickX => Some(StickAxis::X),
                    Axis::LeftStickY => Some(StickAxis::Y),
                    _ => None,
                };
                if let Some(stick_axis) = stick_axis {
                    update_stick(&mut self.stick, stick_axis, *value);
                }
                None
            }
            _ => None,
        }
    }

    pub fn horizontal(&self) -> Option<NavDir> {
        match self.x.held() {
            Some(AxisSide::Negative) => Some(NavDir::Left),
            Some(AxisSide::Positive) => Some(NavDir::Right),
            None => self.stick.horizontal(),
        }
    }

    pub fn vertical(&self) -> Option<NavDir> {
        match self.y.held() {
            Some(AxisSide::Negative) => Some(NavDir::Down),
            Some(AxisSide::Positive) => Some(NavDir::Up),
            None => self.stick.vertical(),
        }
    }
}

/// One FlowBox child's allocation, in sibling order.
/// `height <= 0` means the box has not been allocated yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlowTile {
    pub y: i32,
    pub height: i32,
}

/// How many tiles the FlowBox placed on the first line.
/// Children on one line share `y`. A zero-height tile ends the scan so the
/// caller can fall back to `max_children_per_line` before the first layout.
pub fn line_columns(tiles: &[FlowTile], fallback: i32) -> i32 {
    let Some(first) = tiles.first() else {
        return fallback.max(1);
    };
    if first.height <= 0 {
        return fallback.max(1);
    }
    let mut columns = 1i32;
    for tile in tiles.iter().skip(1) {
        if tile.height <= 0 || tile.y != first.y {
            break;
        }
        columns += 1;
    }
    columns.max(1)
}

/// Same jumps as the arrow keys: one tile left or right, one row of `columns` up or down.
/// `columns` is the live line length, not a fixed grid width.
/// With nothing selected, any direction selects the first tile.
pub fn grid_step(index: Option<i32>, dir: NavDir, len: i32, columns: i32) -> Option<i32> {
    if len <= 0 {
        return None;
    }
    let Some(index) = index else {
        return Some(0);
    };
    let next = match dir {
        NavDir::Left => index - 1,
        NavDir::Right => index + 1,
        NavDir::Up => index - columns,
        NavDir::Down => index + columns,
    };
    (next >= 0 && next < len).then_some(next)
}

/// Systems list is vertical. Left and right do nothing; A enters the grid.
pub fn list_step(index: Option<i32>, dir: NavDir, len: i32) -> Option<i32> {
    if len <= 0 {
        return None;
    }
    let delta = match dir {
        NavDir::Up => -1,
        NavDir::Down => 1,
        NavDir::Left | NavDir::Right => return None,
    };
    let Some(index) = index else {
        return Some(0);
    };
    let next = index + delta;
    (next >= 0 && next < len).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stick_stays_held_until_it_returns_near_center() {
        let mut latch = StickLatch::default();
        update_stick(&mut latch, StickAxis::X, 0.9);
        assert_eq!(latch.horizontal(), Some(NavDir::Right));
        update_stick(&mut latch, StickAxis::X, 1.0);
        assert_eq!(latch.horizontal(), Some(NavDir::Right));
        // Between OFF and ON the previous hold remains.
        update_stick(&mut latch, StickAxis::X, 0.4);
        assert_eq!(latch.horizontal(), Some(NavDir::Right));
        update_stick(&mut latch, StickAxis::X, 0.2);
        assert_eq!(latch.horizontal(), None);
        update_stick(&mut latch, StickAxis::X, 0.9);
        assert_eq!(latch.horizontal(), Some(NavDir::Right));
    }

    #[test]
    fn stick_y_positive_is_up() {
        let mut latch = StickLatch::default();
        update_stick(&mut latch, StickAxis::Y, 0.8);
        assert_eq!(latch.vertical(), Some(NavDir::Up));
        update_stick(&mut latch, StickAxis::Y, 0.0);
        assert_eq!(latch.vertical(), None);
        update_stick(&mut latch, StickAxis::Y, -0.8);
        assert_eq!(latch.vertical(), Some(NavDir::Down));
    }

    #[test]
    fn axes_are_independent() {
        let mut latch = StickLatch::default();
        update_stick(&mut latch, StickAxis::X, -0.9);
        update_stick(&mut latch, StickAxis::Y, 0.9);
        assert_eq!(latch.horizontal(), Some(NavDir::Left));
        assert_eq!(latch.vertical(), Some(NavDir::Up));
        update_stick(&mut latch, StickAxis::X, -0.9);
        assert_eq!(latch.horizontal(), Some(NavDir::Left));
    }

    #[test]
    fn dpad_hold_does_not_repeat_confirm() {
        let mut pad = PadHeld::default();
        assert_eq!(pad.apply_button(Button::DPadRight, true), None);
        assert_eq!(pad.horizontal(), Some(NavDir::Right));
        assert_eq!(pad.apply_button(Button::DPadRight, false), None);
        assert_eq!(pad.horizontal(), None);
        assert_eq!(
            pad.apply_button(Button::South, true),
            Some(PadAction::Confirm)
        );
        assert_eq!(pad.apply_button(Button::South, false), None);
        assert_eq!(pad.apply_button(Button::DPadUp, true), None);
        assert_eq!(pad.vertical(), Some(NavDir::Up));
    }

    #[test]
    fn held_stick_repeats_only_through_the_driver() {
        use crate::config::InputSettings;
        use crate::input_repeat::DirectionRepeat;

        let mut latch = StickLatch::default();
        update_stick(&mut latch, StickAxis::X, 0.9);
        let mut repeat = DirectionRepeat::default();
        let settings = InputSettings::default();
        assert_eq!(repeat.poll(&settings, latch.horizontal(), 0), 1);
        assert_eq!(repeat.poll(&settings, latch.horizontal(), 100), 0);
        update_stick(&mut latch, StickAxis::X, 0.95);
        assert_eq!(repeat.poll(&settings, latch.horizontal(), 400), 1);
        update_stick(&mut latch, StickAxis::X, 0.0);
        assert_eq!(repeat.poll(&settings, latch.horizontal(), 500), 0);
        update_stick(&mut latch, StickAxis::X, 0.9);
        assert_eq!(repeat.poll(&settings, latch.horizontal(), 500), 1);
    }

    #[test]
    fn face_and_dpad_mapping() {
        let mut pad = PadHeld::default();
        assert_eq!(
            pad.apply_button(Button::South, true),
            Some(PadAction::Confirm)
        );
        assert_eq!(pad.apply_button(Button::East, true), Some(PadAction::Back));
        assert_eq!(
            pad.apply_button(Button::North, true),
            Some(PadAction::Favorite)
        );
        assert_eq!(pad.apply_button(Button::West, true), None);
        assert_eq!(pad.apply_button(Button::Start, true), None);
        assert_eq!(
            pad.apply_button(Button::Select, true),
            Some(PadAction::Menu)
        );
        assert_eq!(pad.apply_button(Button::DPadLeft, true), None);
        assert_eq!(pad.horizontal(), Some(NavDir::Left));
    }

    #[test]
    fn select_is_the_game_menu_not_confirm_or_back() {
        let mut pad = PadHeld::default();
        assert_eq!(
            pad.apply_button(Button::Select, true),
            Some(PadAction::Menu)
        );
        assert_eq!(pad.apply_button(Button::Select, false), None);
        assert_eq!(pad.apply_button(Button::Start, true), None);
        assert_eq!(
            pad.apply_button(Button::South, true),
            Some(PadAction::Confirm)
        );
        assert_eq!(pad.apply_button(Button::East, true), Some(PadAction::Back));
        assert_eq!(
            pad.apply_button(Button::North, true),
            Some(PadAction::Favorite)
        );
    }

    #[test]
    fn grid_and_list_steps() {
        assert_eq!(grid_step(None, NavDir::Right, 10, 6), Some(0));
        assert_eq!(grid_step(Some(1), NavDir::Right, 10, 6), Some(2));
        assert_eq!(grid_step(Some(0), NavDir::Left, 10, 6), None);
        assert_eq!(grid_step(Some(7), NavDir::Up, 10, 6), Some(1));
        assert_eq!(grid_step(Some(2), NavDir::Down, 10, 6), Some(8));
        assert_eq!(grid_step(Some(8), NavDir::Down, 10, 6), None);
        assert_eq!(grid_step(Some(0), NavDir::Down, 24, 4), Some(4));
        assert_eq!(grid_step(Some(5), NavDir::Up, 24, 4), Some(1));
        assert_eq!(grid_step(Some(0), NavDir::Down, 24, 6), Some(6));
        assert_eq!(grid_step(Some(7), NavDir::Up, 24, 6), Some(1));
        assert_eq!(grid_step(Some(4), NavDir::Left, 24, 4), Some(3));
        assert_eq!(grid_step(Some(4), NavDir::Right, 24, 4), Some(5));
        assert_eq!(list_step(Some(1), NavDir::Down, 3), Some(2));
        assert_eq!(list_step(Some(0), NavDir::Up, 3), None);
        assert_eq!(list_step(Some(1), NavDir::Left, 3), None);
        assert_eq!(list_step(None, NavDir::Down, 3), Some(0));
    }

    #[test]
    fn line_columns_matches_the_laid_out_row() {
        let tile = |y| FlowTile { y, height: 80 };
        let four = [tile(12), tile(12), tile(12), tile(12), tile(104)];
        assert_eq!(line_columns(&four, 6), 4);
        let six = [
            tile(12),
            tile(12),
            tile(12),
            tile(12),
            tile(12),
            tile(12),
            tile(104),
        ];
        assert_eq!(line_columns(&six, 6), 6);
        assert_eq!(line_columns(&[], 6), 6);
        assert_eq!(line_columns(&[FlowTile { y: 0, height: 0 }], 6), 6);
    }
}
