use gpui_kit::{
    div, px, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, ScrollHandle, StatefulInteractiveElement, Styled, Window,
};
use gpui_omarchy::ActiveTheme;
use std::rc::Rc;

pub const PAGE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picker {
    Closed,
    Open { cursor: usize, query: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jump {
    PageUp,
    PageDown,
    Home,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub cursor: usize,
    pub items: Vec<Item>,
}

impl Picker {
    pub fn closed() -> Self {
        Self::Closed
    }

    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open { .. })
    }

    pub fn cursor(&self) -> Option<usize> {
        match self {
            Self::Open { cursor, .. } => Some(*cursor),
            Self::Closed => None,
        }
    }

    pub fn open_at(&mut self, index: usize) {
        *self = Self::Open {
            cursor: index,
            query: String::new(),
        };
    }

    pub fn close(&mut self) {
        *self = Self::Closed;
    }

    pub fn move_by(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }
        let Self::Open { cursor, query } = self else {
            return;
        };
        let last = len as isize - 1;
        *cursor = (*cursor as isize + delta).clamp(0, last) as usize;
        query.clear();
    }

    pub fn jump(&mut self, jump: Jump, len: usize) {
        if len == 0 {
            return;
        }
        match jump {
            Jump::Home => self.move_to(0),
            Jump::End => self.move_to(len - 1),
            Jump::PageUp => self.move_by(-(PAGE as isize), len),
            Jump::PageDown => self.move_by(PAGE as isize, len),
        }
    }

    pub fn type_ahead(&mut self, text: &str, labels: &[String]) {
        let Some(extra) = letters(text) else {
            return;
        };
        let Self::Open { cursor, query } = self else {
            return;
        };
        let extended = format!("{query}{extra}");
        if let Some(index) = first_prefix(labels, &extended) {
            *cursor = index;
            *query = extended;
            return;
        }
        if let Some(index) = first_prefix(labels, &extra) {
            *cursor = index;
            *query = extra;
        }
    }

    fn move_to(&mut self, index: usize) {
        let Self::Open { cursor, query } = self else {
            return;
        };
        *cursor = index;
        query.clear();
    }
}

fn letters(text: &str) -> Option<String> {
    let out: String = text
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn first_prefix(labels: &[String], prefix: &str) -> Option<usize> {
    if prefix.is_empty() {
        return None;
    }
    labels
        .iter()
        .position(|label| label.to_ascii_lowercase().starts_with(prefix))
}

pub fn menu<V, F>(
    menu: &Menu,
    scroll: &ScrollHandle,
    cx: &Context<V>,
    on_row: F,
) -> impl IntoElement
where
    V: 'static,
    F: Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
{
    let theme = cx.omarchy();
    let on_row = Rc::new(on_row);
    let mut list = div()
        .id("picker-menu")
        .w_full()
        .max_h(px(240.))
        .min_h(px(0.))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .border_1()
        .border_color(theme.accent)
        .bg(theme.background)
        .occlude()
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
        });
    for (index, item) in menu.items.iter().enumerate() {
        let chosen = index == menu.cursor;
        let on_row = Rc::clone(&on_row);
        let mut row = div()
            .id(("picker-row", index))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(8.))
            .py(px(6.))
            .cursor_pointer()
            .on_click(
                cx.listener(move |this: &mut V, _: &ClickEvent, window, cx| {
                    on_row(this, index, window, cx);
                    cx.stop_propagation();
                }),
            );
        if chosen {
            row = row.bg(theme.selected_fill()).text_color(theme.accent);
        } else {
            row = row.hover(|style| style.bg(theme.hover_fill()));
        }
        row = row.child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(item.label.clone()),
        );
        if !item.detail.is_empty() {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(theme.secondary)
                    .child(item.detail.clone()),
            );
        }
        list = list.child(row);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Vec<String> {
        vec![
            "None".into(),
            "Alpha".into(),
            "Arcade".into(),
            "Stella".into(),
        ]
    }

    #[test]
    fn arrows_stop_at_the_ends_and_page_home_end_jump() {
        let mut picker = Picker::closed();
        picker.open_at(0);
        picker.move_by(-1, 10);
        assert_eq!(picker.cursor(), Some(0));
        picker.jump(Jump::PageDown, 10);
        assert_eq!(picker.cursor(), Some(PAGE));
        picker.jump(Jump::End, 10);
        assert_eq!(picker.cursor(), Some(9));
        picker.jump(Jump::PageDown, 10);
        assert_eq!(picker.cursor(), Some(9));
        picker.jump(Jump::Home, 10);
        assert_eq!(picker.cursor(), Some(0));
        picker.close();
        assert_eq!(picker.cursor(), None);
    }

    #[test]
    fn typing_jumps_to_the_first_prefix_and_extends_it() {
        let labels = labels();
        let mut picker = Picker::closed();
        picker.type_ahead("a", &labels);
        assert_eq!(picker.cursor(), None);
        picker.open_at(0);
        picker.type_ahead("a", &labels);
        assert_eq!(picker.cursor(), Some(1));
        picker.type_ahead("r", &labels);
        assert_eq!(picker.cursor(), Some(2));
        picker.type_ahead("z", &labels);
        assert_eq!(picker.cursor(), Some(2));
        picker.type_ahead("s", &labels);
        assert_eq!(picker.cursor(), Some(3));
    }
}
