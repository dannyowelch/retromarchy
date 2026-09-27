//! GTK theme toggle. `theme = "system"` follows Omarchy. `theme = "launchbox"`
//! is the built-in dark palette from the GTK stylesheet. Anything else toggles
//! back to `system`, same as the GTK `t` key and header button.

use gpui_kit::base::ThemeAppearance;
use gpui_kit::rgb;
use gpui_omarchy::Theme;

pub const SYSTEM: &str = "system";
pub const LAUNCHBOX: &str = "launchbox";

/// `t` with no modifiers. GTK matches the `t` keysym, so Shift+T does not fire.
pub fn theme_key(key: &str, key_char: Option<&str>, shift: bool, modified: bool) -> bool {
    if shift || modified {
        return false;
    }
    key == "t" || key_char == Some("t")
}

pub fn next_theme(theme: &str) -> &'static str {
    if theme == SYSTEM {
        LAUNCHBOX
    } else {
        SYSTEM
    }
}

pub fn is_launchbox(theme: &str) -> bool {
    theme == LAUNCHBOX
}

/// GTK LaunchBox CSS, on the palette the shell already reads through `cx.omarchy()`.
pub fn launchbox_theme() -> Theme {
    Theme {
        name: "LaunchBox".into(),
        appearance: ThemeAppearance::Dark,
        background: rgb(0x0a0a0a).into(),
        surface: rgb(0x1a1a1a).into(),
        inset: rgb(0x1a1a1a).into(),
        foreground: rgb(0xe8e8e8).into(),
        secondary: rgb(0x999999).into(),
        bright: rgb(0xe8e8e8).into(),
        accent: rgb(0x6ab0ff).into(),
        on_accent: rgb(0x0a0a0a).into(),
        selection: rgb(0x2a2a2a).into(),
        border: rgb(0x3a3a3a).into(),
        danger: rgb(0xe01b24).into(),
        warning: rgb(0xe0af68).into(),
        success: rgb(0x9ece6a).into(),
        font: ".SystemUIFont".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_matches_gtk() {
        assert_eq!(next_theme("system"), "launchbox");
        assert_eq!(next_theme("launchbox"), "system");
        assert_eq!(next_theme("custom"), "system");
        assert!(is_launchbox("launchbox"));
        assert!(!is_launchbox("system"));
    }

    #[test]
    fn theme_key_is_unshifted_t() {
        assert!(theme_key("t", None, false, false));
        assert!(theme_key("key", Some("t"), false, false));
        assert!(!theme_key("t", Some("t"), true, false));
        assert!(!theme_key("t", None, false, true));
        assert!(!theme_key("T", Some("T"), false, false));
    }

    #[test]
    fn launchbox_palette_uses_the_gtk_css_colors() {
        let theme = launchbox_theme();
        assert_eq!(theme.name.as_ref(), "LaunchBox");
        assert_eq!(theme.appearance, ThemeAppearance::Dark);
        assert_eq!(theme.background, rgb(0x0a0a0a).into());
        assert_eq!(theme.surface, rgb(0x1a1a1a).into());
        assert_eq!(theme.foreground, rgb(0xe8e8e8).into());
        assert_eq!(theme.secondary, rgb(0x999999).into());
        assert_eq!(theme.accent, rgb(0x6ab0ff).into());
        assert_eq!(theme.selection, rgb(0x2a2a2a).into());
        assert_eq!(theme.danger, rgb(0xe01b24).into());
    }
}
