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
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .flex()
        .flex_col()
        .bg(theme.background)
        .occlude()
        .on_mouse_down(
            gpui_kit::MouseButton::Left,
            |_: &gpui_kit::MouseDownEvent, _, cx| {
                cx.stop_propagation();
            },
        )
        .on_mouse_down(
            gpui_kit::MouseButton::Right,
            |_: &gpui_kit::MouseDownEvent, _, cx| {
                cx.stop_propagation();
            },
        )
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
                            div()
                                .id(list_id)
                                .flex_1()
                                .min_h(px(0.))
                                .flex()
                                .flex_col()
                                .overflow_y_scroll()
                                .track_scroll(list_scroll)
                                .p(px(8.))
                                .gap(px(4.))
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
