use crate::browse::SIDEBAR_WIDTH;
use gpui_kit::{
    div, px, App, InteractiveElement, IntoElement, ParentElement, ScrollHandle,
    StatefulInteractiveElement, Styled,
};
use gpui_omarchy::ActiveTheme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    List,
    Panel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split {
    pub index: usize,
    pub side: Side,
}

impl Split {
    pub fn list() -> Self {
        Self {
            index: 0,
            side: Side::List,
        }
    }

    pub fn at(index: usize) -> Self {
        Self {
            index,
            side: Side::List,
        }
    }

    pub fn move_list(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.index = 0;
            return;
        }
        let next = self.index as isize + delta;
        if (0..len as isize).contains(&next) {
            self.index = next as usize;
        }
    }

    pub fn enter(&mut self) {
        self.side = Side::Panel;
    }

    pub fn leave(&mut self) {
        self.side = Side::List;
    }

    pub fn select(&mut self, index: usize) {
        self.index = index;
        self.side = Side::List;
    }
}

pub const HINT: &str = "Up and down move the list. Right, Enter, or Tab edits. Left at the edge, or Shift-Tab, returns. Esc saves and closes.";

/// Shared by the library sidebar and the settings-screen lists.
pub const LIST_PAD: f32 = 8.;
pub const LIST_GAP: f32 = 4.;
pub const ROW_PAD_X: f32 = 8.;
pub const ROW_PAD_Y: f32 = 6.;
pub const ROW_GAP: f32 = 1.;
pub const TITLE_SIZE: f32 = 14.;
pub const TITLE_LINE: f32 = 18.;
pub const DETAIL_SIZE: f32 = 12.;
pub const DETAIL_LINE: f32 = 15.;

pub fn list_metrics<E: Styled>(el: E) -> E {
    el.p(px(LIST_PAD)).gap(px(LIST_GAP))
}

pub fn row_metrics<E: Styled>(el: E) -> E {
    el.flex_shrink_0()
        .flex()
        .flex_col()
        .px(px(ROW_PAD_X))
        .py(px(ROW_PAD_Y))
        .gap(px(ROW_GAP))
}

pub fn title_metrics<E: Styled>(el: E) -> E {
    el.w_full()
        .min_w(px(0.))
        .text_size(px(TITLE_SIZE))
        .line_height(px(TITLE_LINE))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
}

pub fn detail_metrics<E: Styled>(el: E) -> E {
    el.text_size(px(DETAIL_SIZE)).line_height(px(DETAIL_LINE))
}

/// List and panel for one settings screen. The shell places this in the slot
/// between the menu bar and the status bar, in place of the game grid.
pub fn screen<R, E>(
    list_id: &'static str,
    title: &'static str,
    list_focused: bool,
    list_scroll: &ScrollHandle,
    rows: R,
    panel_focused: bool,
    panel: impl IntoElement,
    cx: &App,
) -> impl IntoElement
where
    R: IntoIterator<Item = E>,
    E: IntoElement,
{
    let theme = cx.omarchy();
    div()
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .flex_col()
        .bg(theme.background)
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(16.))
                .px(px(16.))
                .h(px(48.))
                .bg(theme.surface)
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex_none()
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_size(px(16.))
                        .child(title),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.))
                        .text_color(theme.secondary)
                        .whitespace_normal()
                        .child(HINT),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_row()
                .child(
                    div()
                        .w(px(SIDEBAR_WIDTH))
                        .h_full()
                        .min_h(px(0.))
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .bg(theme.inset)
                        .border_r_1()
                        .border_color(if list_focused {
                            theme.accent
                        } else {
                            theme.border
                        })
                        .child(
                            list_metrics(
                                div()
                                    .id(list_id)
                                    .flex_1()
                                    .min_h(px(0.))
                                    .flex()
                                    .flex_col()
                                    .overflow_y_scroll()
                                    .track_scroll(list_scroll),
                            )
                            .children(rows),
                        ),
                )
                .child(
                    div()
                        .id("settings-panel")
                        .flex_1()
                        .h_full()
                        .min_w(px(0.))
                        .min_h(px(0.))
                        .flex()
                        .flex_col()
                        .overflow_y_scroll()
                        .p(px(16.))
                        .gap(px(12.))
                        .border_1()
                        .border_color(if panel_focused {
                            theme.accent
                        } else {
                            theme.background
                        })
                        .child(panel),
                ),
        )
}

/// Last index scrolled into view. The same index does not scroll again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RevealedList {
    index: Option<usize>,
}

/// Whether this frame should move a settings list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListReveal {
    Scroll(usize),
    Wait,
    Keep,
}

impl RevealedList {
    /// `ready` means the scrollport already has a height.
    /// The stored index wins over `ready`, so a later zero-height frame does not wait again.
    pub fn reveal(&mut self, index: usize, ready: bool) -> ListReveal {
        if self.index == Some(index) {
            return ListReveal::Keep;
        }
        if !ready {
            return ListReveal::Wait;
        }
        self.index = Some(index);
        ListReveal::Scroll(index)
    }

    pub fn clear(&mut self) {
        self.index = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_keeps_the_offset_while_the_selection_stays() {
        let mut revealed = RevealedList::default();
        assert_eq!(revealed.reveal(0, false), ListReveal::Wait);
        assert_eq!(revealed.reveal(0, true), ListReveal::Scroll(0));
        assert_eq!(revealed.reveal(0, true), ListReveal::Keep);
        assert_eq!(revealed.reveal(0, true), ListReveal::Keep);
        assert_eq!(revealed.reveal(0, false), ListReveal::Keep);
    }

    #[test]
    fn selection_change_scrolls_once_after_the_list_is_ready() {
        let mut revealed = RevealedList::default();
        assert_eq!(revealed.reveal(0, true), ListReveal::Scroll(0));
        assert_eq!(revealed.reveal(4, false), ListReveal::Wait);
        assert_eq!(revealed.reveal(4, false), ListReveal::Wait);
        assert_eq!(revealed.reveal(4, true), ListReveal::Scroll(4));
        assert_eq!(revealed.reveal(4, true), ListReveal::Keep);
    }

    #[test]
    fn closing_the_list_reveals_that_index_again() {
        let mut revealed = RevealedList::default();
        assert_eq!(revealed.reveal(2, true), ListReveal::Scroll(2));
        revealed.clear();
        assert_eq!(revealed.reveal(2, false), ListReveal::Wait);
        assert_eq!(revealed.reveal(2, true), ListReveal::Scroll(2));
    }
}
