//! SDL / Xbox layout via gilrs. South confirms, East goes back, North (Y) toggles a favorite.
//! The left stick fires one step per deflection and must return near center before the next.

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
    Move(NavDir),
    Confirm,
    Back,
    Favorite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickAxis {
    X,
    Y,
}

#[derive(Debug, Default)]
pub struct StickLatch {
    x: Option<i8>,
    y: Option<i8>,
}

const STICK_ON: f32 = 0.55;
const STICK_OFF: f32 = 0.35;

pub fn stick_move(latch: &mut StickLatch, axis: StickAxis, value: f32) -> Option<NavDir> {
    let (slot, negative, positive) = match axis {
        StickAxis::X => (&mut latch.x, NavDir::Left, NavDir::Right),
        // gilrs uses the SDL sign: positive left-stick Y is up.
        StickAxis::Y => (&mut latch.y, NavDir::Down, NavDir::Up),
    };
    axis_step(slot, value, negative, positive)
}

fn axis_step(
    fired: &mut Option<i8>,
    value: f32,
    negative: NavDir,
    positive: NavDir,
) -> Option<NavDir> {
    if !value.is_finite() {
        return None;
    }
    let dir = if value >= STICK_ON {
        1
    } else if value <= -STICK_ON {
        -1
    } else {
        0
    };
    if dir == 0 {
        if value.abs() < STICK_OFF {
            *fired = None;
        }
        return None;
    }
    if *fired == Some(dir) {
        return None;
    }
    *fired = Some(dir);
    Some(if dir < 0 { negative } else { positive })
}

pub fn button_action(button: Button) -> Option<PadAction> {
    match button {
        Button::South => Some(PadAction::Confirm),
        Button::East => Some(PadAction::Back),
        Button::North => Some(PadAction::Favorite),
        Button::DPadLeft => Some(PadAction::Move(NavDir::Left)),
        Button::DPadRight => Some(PadAction::Move(NavDir::Right)),
        Button::DPadUp => Some(PadAction::Move(NavDir::Up)),
        Button::DPadDown => Some(PadAction::Move(NavDir::Down)),
        _ => None,
    }
}

pub fn event_action(event: &EventType, latch: &mut StickLatch) -> Option<PadAction> {
    match event {
        EventType::ButtonPressed(button, _) => button_action(*button),
        EventType::AxisChanged(axis, value, _) => {
            let stick_axis = match axis {
                Axis::LeftStickX => StickAxis::X,
                Axis::LeftStickY => StickAxis::Y,
                _ => return None,
            };
            stick_move(latch, stick_axis, *value).map(PadAction::Move)
        }
        _ => None,
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
    fn stick_fires_once_until_it_returns() {
        let mut latch = StickLatch::default();
        assert_eq!(stick_move(&mut latch, StickAxis::X, 0.9), Some(NavDir::Right));
        assert_eq!(stick_move(&mut latch, StickAxis::X, 1.0), None);
        assert_eq!(stick_move(&mut latch, StickAxis::X, 0.4), None);
        assert_eq!(stick_move(&mut latch, StickAxis::X, 0.2), None);
        assert_eq!(stick_move(&mut latch, StickAxis::X, 0.9), Some(NavDir::Right));
    }

    #[test]
    fn stick_y_positive_is_up() {
        let mut latch = StickLatch::default();
        assert_eq!(stick_move(&mut latch, StickAxis::Y, 0.8), Some(NavDir::Up));
        assert_eq!(stick_move(&mut latch, StickAxis::Y, 0.0), None);
        assert_eq!(stick_move(&mut latch, StickAxis::Y, -0.8), Some(NavDir::Down));
    }

    #[test]
    fn axes_are_independent() {
        let mut latch = StickLatch::default();
        assert_eq!(stick_move(&mut latch, StickAxis::X, -0.9), Some(NavDir::Left));
        assert_eq!(stick_move(&mut latch, StickAxis::Y, 0.9), Some(NavDir::Up));
        assert_eq!(stick_move(&mut latch, StickAxis::X, -0.9), None);
    }

    #[test]
    fn face_and_dpad_mapping() {
        assert_eq!(button_action(Button::South), Some(PadAction::Confirm));
        assert_eq!(button_action(Button::East), Some(PadAction::Back));
        assert_eq!(button_action(Button::North), Some(PadAction::Favorite));
        assert_eq!(button_action(Button::West), None);
        assert_eq!(button_action(Button::Start), None);
        assert_eq!(button_action(Button::Select), None);
        assert_eq!(
            button_action(Button::DPadLeft),
            Some(PadAction::Move(NavDir::Left))
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
        let six = [tile(12), tile(12), tile(12), tile(12), tile(12), tile(12), tile(104)];
        assert_eq!(line_columns(&six, 6), 6);
        assert_eq!(line_columns(&[], 6), 6);
        assert_eq!(line_columns(&[FlowTile { y: 0, height: 0 }], 6), 6);
    }
}
