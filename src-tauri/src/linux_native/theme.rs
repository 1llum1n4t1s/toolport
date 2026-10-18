use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::gio;
use serde::Deserialize;

const FALLBACK_BACKGROUND: &str = "#1d2027";
const FALLBACK_DARK_BACKGROUND: &str = "#20232b";
const FALLBACK_LIGHTER_BACKGROUND: &str = "#252931";
const FALLBACK_FOREGROUND: &str = "#e8e9ec";
const FALLBACK_MUTED: &str = "#a1a9b8";
const FALLBACK_ACCENT: &str = "#7cb0ff";
const FALLBACK_SELECTION: &str = "#334155";
const FALLBACK_RED: &str = "#f38ba8";
const FALLBACK_GREEN: &str = "#7fd0a0";
const FALLBACK_YELLOW: &str = "#f0c674";
const FALLBACK_CYAN: &str = "#38bdf8";

/// WCAG AA for body, secondary and status text (12 to 14px).
const TEXT_CONTRAST: f64 = 4.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeMode {
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OmarchyPalette {
    mode: ThemeMode,
    background: String,
    dark_background: String,
    lighter_background: String,
    foreground: String,
    muted: String,
    accent: String,
    selection: String,
    red: String,
    green: String,
    yellow: String,
    cyan: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawPalette {
    mode: Option<String>,
    background: Option<String>,
    dark_background: Option<String>,
    lighter_background: Option<String>,
    foreground: Option<String>,
    muted: Option<String>,
    accent: Option<String>,
    selection: Option<String>,
    red: Option<String>,
    green: Option<String>,
    yellow: Option<String>,
    cyan: Option<String>,
}

impl Default for OmarchyPalette {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Dark,
            background: FALLBACK_BACKGROUND.into(),
            dark_background: FALLBACK_DARK_BACKGROUND.into(),
            lighter_background: FALLBACK_LIGHTER_BACKGROUND.into(),
            foreground: FALLBACK_FOREGROUND.into(),
            muted: FALLBACK_MUTED.into(),
            accent: FALLBACK_ACCENT.into(),
            selection: FALLBACK_SELECTION.into(),
            red: FALLBACK_RED.into(),
            green: FALLBACK_GREEN.into(),
            yellow: FALLBACK_YELLOW.into(),
            cyan: FALLBACK_CYAN.into(),
        }
    }
}

impl OmarchyPalette {
    fn parse(input: &str) -> Result<Self, String> {
        let raw: RawPalette = toml::from_str(input)
            .map_err(|error| format!("could not parse Omarchy colors.toml: {error}"))?;
        let fallback = Self::default();
        Ok(Self {
            mode: match raw.mode.as_deref() {
                Some("light") => ThemeMode::Light,
                Some("dark") => ThemeMode::Dark,
                _ => fallback.mode,
            },
            background: safe_color(raw.background, &fallback.background),
            dark_background: safe_color(raw.dark_background, &fallback.dark_background),
            lighter_background: safe_color(raw.lighter_background, &fallback.lighter_background),
            foreground: safe_color(raw.foreground, &fallback.foreground),
            muted: safe_color(raw.muted, &fallback.muted),
            accent: safe_color(raw.accent, &fallback.accent),
            selection: safe_color(raw.selection, &fallback.selection),
            red: safe_color(raw.red, &fallback.red),
            green: safe_color(raw.green, &fallback.green),
            yellow: safe_color(raw.yellow, &fallback.yellow),
            cyan: safe_color(raw.cyan, &fallback.cyan),
        })
    }

    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))
            .and_then(|contents| Self::parse(&contents))
            .unwrap_or_else(|error| {
                eprintln!("toolport-gtk: {error}; using the fallback palette");
                Self::default()
            })
    }

    /// Colors the stylesheet derives from the palette. Every surface that holds
    /// text is opaque. Text colors start from the theme's own color and move
    /// toward black (light themes) or white (dark themes) only as far as WCAG AA
    /// needs on every surface they sit on. Stock themes whose `muted` is a border
    /// color, or whose status colors are pale on a light background, stay
    /// readable and keep their hue.
    fn tokens(&self) -> Tokens {
        let background = rgb(&self.background);
        let card = mix(background, rgb(&self.lighter_background), 0.5).map(f64::round);
        let surfaces = [background, rgb(&self.dark_background), card];
        let (black, white) = ([0.0; 3], [255.0; 3]);
        let toward = if contrast(black, background) > contrast(white, background) {
            black
        } else {
            white
        };
        let secondary = readable(rgb(&self.muted), toward, &surfaces, false);
        let status = |color: &str| hex(readable(rgb(color), toward, &surfaces, true));
        Tokens {
            card: hex(card),
            text_secondary: hex(secondary),
            success: status(&self.green),
            warning: status(&self.yellow),
            danger: status(&self.red),
            partial: status(&self.cyan),
        }
    }

    fn css(&self) -> String {
        let tokens = self.tokens();
        format!(
            r#"
/* Palette from Omarchy's colors.toml. */
@define-color toolport_bg {background};
@define-color toolport_bg_dark {dark_background};
@define-color toolport_surface {lighter_background};
@define-color toolport_fg {foreground};
@define-color toolport_muted {muted};
@define-color toolport_accent {accent};
@define-color toolport_selection {selection};

/* Semantic tokens (design spec: bg, sidebar, surface, raised, border, text,
   text-secondary, text-disabled, accent and the status colors). Surfaces are
   opaque; alpha is only for tints, hover, selection and borders on top of them. */
@define-color toolport_sidebar @toolport_bg_dark;
@define-color toolport_card {card};
@define-color toolport_raised @toolport_bg_dark;
@define-color toolport_border alpha(@toolport_fg, 0.10);
@define-color toolport_text_secondary {text_secondary};
@define-color toolport_text_disabled alpha(@toolport_text_secondary, 0.7);
@define-color toolport_success {success};
@define-color toolport_warning {warning};
@define-color toolport_error {danger};
@define-color toolport_partial {partial};
@define-color toolport_info @toolport_text_secondary;
@define-color toolport_info_tint alpha(@toolport_fg, 0.06);
@define-color toolport_off @toolport_text_secondary;

:root {{
  --accent-bg-color: @toolport_accent;
  --accent-fg-color: @toolport_bg_dark;
  --accent-color: @toolport_accent;
  --destructive-bg-color: @toolport_error;
  --destructive-fg-color: @toolport_bg_dark;
  --destructive-color: @toolport_error;
  --success-bg-color: @toolport_success;
  --success-color: @toolport_success;
  --warning-bg-color: @toolport_warning;
  --warning-fg-color: @toolport_bg_dark;
  --warning-color: @toolport_warning;
  --error-bg-color: @toolport_error;
  --error-color: @toolport_error;
  --window-bg-color: @toolport_bg;
  --window-fg-color: @toolport_fg;
  --view-bg-color: @toolport_bg_dark;
  --view-fg-color: @toolport_fg;
  --headerbar-bg-color: @toolport_sidebar;
  --headerbar-fg-color: @toolport_fg;
  --headerbar-border-color: @toolport_border;
  --sidebar-bg-color: @toolport_sidebar;
  --sidebar-fg-color: @toolport_fg;
  --card-bg-color: @toolport_card;
  --card-fg-color: @toolport_fg;
  --dialog-bg-color: @toolport_raised;
  --dialog-fg-color: @toolport_fg;
  --popover-bg-color: @toolport_raised;
  --popover-fg-color: @toolport_fg;
  --shade-color: alpha(black, 0.28);
}}

/* libadwaita fades secondary labels with opacity, which drops them below AA
   on most themes. Use the opaque secondary color instead. Placeholders keep
   the disabled color. */
.toolport-native .dim-label,
.toolport-native .dimmed,
.toolport-native row label.subtitle,
.toolport-native headerbar .subtitle,
.toolport-native windowtitle .subtitle {{
  opacity: 1;
  color: @toolport_text_secondary;
}}

tooltip.background {{
  background-color: @toolport_raised;
  color: @toolport_fg;
  border: 1px solid @toolport_border;
  border-radius: 8px;
}}

window {{
  background-color: @toolport_bg;
  color: @toolport_fg;
  font-family: "Inter", system-ui, sans-serif;
  font-size: 14px;
}}

window.messagedialog,
window.dialog-window.alert,
dialog-host > dialog.alert sheet {{
  background-color: @toolport_raised;
  color: @toolport_fg;
}}

window.messagedialog .message-area > .body,
window.dialog-window.alert .message-area > .body,
dialog-host > dialog.alert .message-area > .body {{
  color: @toolport_text_secondary;
}}

window.messagedialog .response-area > button,
window.dialog-window.alert .response-area > button,
dialog-host > dialog.alert .response-area > button {{
  border: 1px solid @toolport_border;
  color: @toolport_fg;
  background-color: @toolport_card;
  box-shadow: none;
}}

window.messagedialog .response-area > button:hover,
window.dialog-window.alert .response-area > button:hover,
dialog-host > dialog.alert .response-area > button:hover {{
  background-color: alpha(@toolport_accent, 0.14);
  border-color: alpha(@toolport_accent, 0.30);
}}

window.messagedialog .response-area > button.suggested-action,
window.dialog-window.alert .response-area > button.suggested-action,
dialog-host > dialog.alert .response-area > button.suggested-action {{
  color: @toolport_bg_dark;
  background-color: @toolport_accent;
  border-color: @toolport_accent;
}}

window.messagedialog .response-area > button.destructive-action,
window.dialog-window.alert .response-area > button.destructive-action,
dialog-host > dialog.alert .response-area > button.destructive-action {{
  color: @toolport_bg_dark;
  background-color: @toolport_error;
  border-color: @toolport_error;
}}

.toolport-native {{
  background-color: @toolport_bg;
  color: @toolport_fg;
  /* Omarchy's 0.9091 text scale resolves Adwaita Sans 11 to 13.33px. At that
     fractional raster size Pango drops the top stroke of capitals such as E,
     F, P, and T. Keep the chosen family and weights, but use the nearest whole
     CSS pixel so glyph hinting remains intact. */
  font-family: "Inter", system-ui, sans-serif;
  font-size: 14px;
}}

.toolport-native .caption {{
  font-size: 13px;
}}

.toolport-shell {{ background-color: @toolport_bg; }}

.toolport-content,
.toolport-content scrolledwindow,
.toolport-content viewport {{
  background-color: @toolport_bg;
  color: @toolport_fg;
}}

.toolport-sidebar {{
  background-color: @toolport_sidebar;
  color: @toolport_fg;
  border-right: 1px solid @toolport_border;
}}

.toolport-header {{
  background-color: @toolport_sidebar;
  border-bottom: 1px solid @toolport_border;
  box-shadow: none;
}}

.toolport-header button.toolport-header-add {{
  min-width: 32px;
  min-height: 32px;
  padding: 0;
  border-radius: 8px;
  color: @toolport_accent;
  background-color: alpha(@toolport_accent, 0.10);
}}

.toolport-header button.toolport-header-add:hover {{
  background-color: alpha(@toolport_accent, 0.18);
}}

popover.toolport-main-menu > contents {{
  min-width: 220px;
  padding: 8px;
  border: 1px solid @toolport_border;
  border-radius: 12px;
  color: @toolport_fg;
  background-color: @toolport_raised;
  box-shadow: 0 12px 30px alpha(black, 0.32);
}}

popover.toolport-main-menu button.toolport-main-menu-item {{
  min-height: 36px;
  padding: 0 10px;
  border: none;
  border-radius: 8px;
  color: @toolport_fg;
  background-color: transparent;
  box-shadow: none;
}}

popover.toolport-main-menu button.toolport-main-menu-item:hover {{
  color: @toolport_fg;
  background-color: alpha(@toolport_accent, 0.12);
}}

popover.toolport-main-menu button.toolport-main-menu-quit {{
  color: @toolport_error;
}}

popover.toolport-main-menu .toolport-menu-shortcut {{
  color: @toolport_text_secondary;
  font-size: 12px;
}}

popover.toolport-main-menu separator {{
  min-height: 1px;
  margin: 5px 8px;
  background-color: @toolport_border;
}}

popover.toolport-menu > contents {{
  min-width: 220px;
  padding: 8px;
  border: 1px solid @toolport_border;
  border-radius: 12px;
  color: @toolport_fg;
  background-color: @toolport_raised;
  box-shadow: 0 12px 30px alpha(black, 0.32);
}}

popover.toolport-menu button.toolport-menu-item {{
  min-height: 36px;
  padding: 0 10px;
  border: none;
  border-radius: 8px;
  color: @toolport_fg;
  background-color: transparent;
  box-shadow: none;
}}

popover.toolport-menu button.toolport-menu-item:hover {{
  color: @toolport_fg;
  background-color: alpha(@toolport_accent, 0.12);
}}

popover.toolport-menu button.toolport-menu-item.destructive-action {{
  color: @toolport_error;
}}

popover.toolport-menu .toolport-menu-heading {{
  margin: 5px 10px 2px;
  color: @toolport_text_secondary;
  font-size: 12px;
  font-weight: 600;
}}

popover.toolport-menu .toolport-menu-check {{
  color: @toolport_accent;
}}

popover.toolport-menu separator {{
  min-height: 1px;
  margin: 5px 8px;
  background-color: @toolport_border;
}}

popover.toolport-catalog-suggestions > contents {{
  padding: 8px;
  border: 1px solid @toolport_border;
  border-radius: 12px;
  color: @toolport_fg;
  background-color: @toolport_raised;
  box-shadow: 0 12px 30px alpha(black, 0.34);
}}

.toolport-suggestion-heading {{
  padding: 4px 7px 5px;
}}

.toolport-suggestion-row {{
  min-height: 42px;
  padding: 5px 7px;
  border-radius: 8px;
  background-color: transparent;
}}

.toolport-suggestion-row:hover {{
  background-color: alpha(@toolport_accent, 0.08);
}}

.toolport-suggestion-message {{
  padding: 12px;
}}

.toolport-suggestion-message.error {{
  color: @toolport_error;
}}

button.toolport-suggestion-more {{
  min-height: 30px;
  margin-top: 3px;
  border-radius: 8px;
  color: @toolport_accent;
  background-color: alpha(@toolport_accent, 0.08);
}}

popover.background > contents,
popover.menu > contents,
dropdown popover > contents {{
  border: 1px solid @toolport_border;
  border-radius: 12px;
  color: @toolport_fg;
  background-color: @toolport_raised;
  box-shadow: 0 12px 30px alpha(black, 0.32);
}}

popover.menu modelbutton:hover,
dropdown popover row:hover {{
  background-color: alpha(@toolport_accent, 0.12);
}}

dropdown popover listview {{
  color: @toolport_fg;
  background-color: transparent;
}}

dropdown popover row:selected {{
  color: @toolport_fg;
  background-color: alpha(@toolport_accent, 0.18);
}}

.toolport-brand-mark {{
  min-width: 28px;
  min-height: 28px;
}}

.toolport-client-logo,
.toolport-server-logo {{
  min-width: 32px;
  min-height: 32px;
  margin: 1px;
}}

.toolport-server-logo {{
  border-radius: 8px;
  background-color: alpha(white, 0.88);
}}

/* Single-color client marks are drawn in black. Dark themes turn them white;
   the hue rotation keeps an accent such as Amazon Q's orange close to its own. */
.toolport-mono-logo {{
  filter: {mono_logo_filter};
}}

.toolport-nav-item {{
  min-height: 34px;
  padding: 0 12px;
  border-radius: 8px;
  /* Omarchy's `muted` is a border colour and falls below 3:1 on most stock
     themes; navigation is primary text, as in Omarchy's own menus. */
  color: @toolport_fg;
  transition: 120ms ease;
}}

.toolport-nav-item:hover {{
  background-color: alpha(@toolport_fg, 0.06);
  color: @toolport_fg;
}}

.toolport-nav-item.selected {{
  background-color: alpha(@toolport_accent, 0.075);
  color: @toolport_fg;
  box-shadow: inset 3px 0 @toolport_accent;
}}

.toolport-nav-item.selected image {{
  color: @toolport_accent;
}}



.toolport-mode-badge {{
  margin: 7px 2px;
  padding: 4px 9px;
  border: 1px solid @toolport_border;
  border-radius: 999px;
  background-color: @toolport_info_tint;
  color: @toolport_text_secondary;
  font-size: 12px;
}}

.toolport-feedback {{
  padding: 10px 14px;
  border-radius: 0 12px 12px 0;
  background-color: alpha(@toolport_accent, 0.08);
  border: none;
  border-left: 3px solid @toolport_accent;
  color: @toolport_fg;
  font-size: 13px;
}}

/* Holds its place so rows and open menus do not jump, but shows nothing. */
.toolport-feedback.idle {{
  background-color: transparent;
  border-color: transparent;
}}

.toolport-feedback.success {{
  color: @toolport_fg;
  border-color: @toolport_success;
  background-color: alpha(@toolport_success, 0.08);
}}

.toolport-feedback.error {{
  color: @toolport_fg;
  border-color: @toolport_error;
  background-color: alpha(@toolport_error, 0.08);
}}

.toolport-editor {{
  background-color: @toolport_bg;
  color: @toolport_fg;
}}

.toolport-editor headerbar {{
  min-height: 48px;
  padding: 4px 8px;
  background-color: @toolport_sidebar;
  border-bottom: 1px solid @toolport_border;
  box-shadow: none;
}}

.toolport-editor headerbar button {{
  min-height: 32px;
  min-width: 32px;
  padding: 0 12px;
  border-radius: 8px;
  background-color: @toolport_card;
  border: 1px solid @toolport_border;
}}

.toolport-editor headerbar button:hover {{
  background-color: alpha(@toolport_fg, 0.10);
}}

.toolport-editor headerbar button.suggested-action {{
  color: @toolport_bg_dark;
  background-color: @toolport_accent;
  border-color: alpha(@toolport_accent, 0.72);
}}

.toolport-editor headerbar button.suggested-action:disabled {{
  color: alpha(@toolport_fg, 0.58);
  background-color: alpha(@toolport_accent, 0.16);
  border-color: alpha(@toolport_accent, 0.20);
  opacity: 1;
}}

.toolport-editor-body {{
  padding: 20px;
}}

.toolport-import-scroller {{
  border: 1px solid @toolport_border;
  border-radius: 12px;
  background-color: @toolport_card;
}}

.toolport-import-scroller viewport,
.toolport-import-list {{
  background-color: transparent;
}}

.toolport-import-list {{
  padding: 8px;
}}

.toolport-import-row {{
  min-height: 48px;
  padding: 9px 11px;
  border-radius: 8px;
  background-color: @toolport_card;
}}

.toolport-import-row:hover {{
  background-color: alpha(@toolport_accent, 0.08);
}}

.toolport-editor-lede {{
  margin: 2px 2px 4px;
  color: @toolport_text_secondary;
}}

.toolport-editor-intro {{
  margin-bottom: 4px;
}}

.toolport-editor-icon {{
  min-width: 42px;
  min-height: 42px;
  border-radius: 12px;
  background-color: alpha(@toolport_accent, 0.12);
  color: @toolport_accent;
}}

.toolport-form-section {{
  padding: 14px 4px;
  border-radius: 0;
  background-color: transparent;
  border: none;
  border-top: 1px solid @toolport_border;
}}

.toolport-paste-expander {{
  padding: 0 14px;
  border-radius: 12px;
  color: @toolport_fg;
  background-color: @toolport_card;
  border: 1px solid @toolport_border;
}}

.toolport-paste-expander > title {{
  min-height: 46px;
  font-weight: 600;
}}

.toolport-paste-expander > title > expander {{
  color: @toolport_accent;
}}

.toolport-field-label {{
  color: @toolport_text_secondary;
  font-size: 12px;
  font-weight: 600;
}}

.toolport-input,
.toolport-input > text,
.toolport-input > button,
.toolport-text-area,
.toolport-text-area textview,
.toolport-text-area text {{
  background-color: @toolport_card;
  color: @toolport_fg;
}}

.toolport-input,
.toolport-input > button,
.toolport-text-area {{
  min-height: 40px;
  border-radius: 8px;
  border: 1px solid alpha(@toolport_fg, 0.15);
  box-shadow: none;
}}

.toolport-input:focus-within,
.toolport-text-area:focus-within {{
  border-color: alpha(@toolport_accent, 0.72);
  box-shadow: 0 0 0 2px alpha(@toolport_accent, 0.12);
}}

.toolport-compact-select,
.toolport-compact-select > button {{
  min-height: 32px;
  padding-top: 0;
  padding-bottom: 0;
}}

.toolport-editor-note {{
  padding: 11px 13px;
  border-radius: 12px;
  background-color: @toolport_info_tint;
  border: 1px solid @toolport_border;
  color: @toolport_text_secondary;
}}

.toolport-editor .toolport-muted {{
  color: @toolport_text_secondary;
}}

.toolport-editor button.toolport-secondary-action {{
  background-color: @toolport_raised;
  border: 1px solid @toolport_border;
}}

.toolport-editor button.toolport-destructive-icon {{
  color: @toolport_error;
  background-color: alpha(@toolport_error, 0.07);
  border: 1px solid alpha(@toolport_error, 0.15);
}}

.toolport-credential-row {{
  padding: 13px 4px;
  border-radius: 0;
  border: none;
  border-bottom: 1px solid @toolport_border;
  background-color: transparent;
}}

.toolport-approvals {{
  padding: 12px;
}}

.toolport-global-alert {{
  color: @toolport_fg;
  background-color: @toolport_bg;
  border: none;
  border-left: 3px solid @toolport_accent;
  border-radius: 0 12px 12px 0;
  box-shadow: 0 12px 34px alpha(@toolport_bg_dark, 0.58);
}}

.toolport-global-alert.security {{
  padding: 12px 14px;
  border-color: alpha(@toolport_accent, 0.34);
  background-color: mix(@toolport_bg, @toolport_accent, 0.035);
}}

.toolport-global-alert.security > image {{
  color: @toolport_accent;
}}

.toolport-global-alert.security-event {{
  padding: 11px 14px;
  border-color: alpha(@toolport_error, 0.30);
  background-color: mix(@toolport_bg, @toolport_error, 0.035);
}}

.toolport-global-alert.security-event > image {{
  color: @toolport_error;
}}

.toolport-approval-card {{
  padding: 14px 16px;
  border: none;
  border-left: 3px solid @toolport_warning;
  border-radius: 0 12px 12px 0;
  background-color: alpha(@toolport_warning, 0.08);
}}

.toolport-approval-deadline {{
  padding: 3px 8px;
  border-radius: 999px;
  color: @toolport_accent;
  background-color: alpha(@toolport_accent, 0.10);
}}

.toolport-sensitive-review {{
  padding: 10px 12px;
  border-radius: 0 12px 12px 0;
  color: @toolport_fg;
  background-color: alpha(@toolport_error, 0.08);
  border: none;
  border-left: 3px solid @toolport_error;
}}

.toolport-arguments {{
  margin-top: 8px;
  padding: 10px 12px;
  border-radius: 8px;
  background-color: @toolport_bg_dark;
  font-family: monospace;
}}

.toolport-page {{
  background-color: @toolport_bg;
}}

.toolport-dialog-content {{
  padding: 18px;
}}

.toolport-summary {{
  margin-top: 8px;
  margin-bottom: 8px;
}}

.toolport-summary-item {{
  background-color: transparent;
  border: none;
  border-bottom: 1px solid @toolport_border;
  border-radius: 0;
  padding: 10px 13px;
}}

.toolport-security-status {{
  min-height: 48px;
  padding: 8px 12px;
  color: @toolport_fg;
  background-color: @toolport_card;
  border: none;
  border-left: 3px solid @toolport_success;
  border-radius: 0 12px 12px 0;
  box-shadow: none;
}}

.toolport-security-status:hover {{
  background-color: mix(@toolport_card, @toolport_fg, 0.04);
  border-color: alpha(@toolport_accent, 0.28);
}}

.toolport-security-status.attention {{
  background-color: alpha(@toolport_error, 0.055);
  border-color: alpha(@toolport_error, 0.24);
}}

.toolport-security-status.review {{
  border-color: @toolport_warning;
}}

.toolport-security-status-icon.review {{
  color: @toolport_warning;
}}

.toolport-security-status-icon {{
  color: @toolport_success;
}}

.toolport-security-status-icon.attention {{
  color: @toolport_error;
}}

.toolport-activity-filter,
.toolport-activity-filter > button {{
  min-height: 32px;
  padding-top: 0;
  padding-bottom: 0;
}}

button.toolport-activity-filter {{
  padding-left: 11px;
  padding-right: 11px;
}}

.toolport-search {{
  min-height: 36px;
  border-radius: 8px;
  color: @toolport_fg;
  background-color: @toolport_card;
  border: 1px solid @toolport_border;
  box-shadow: none;
}}

.toolport-search:focus-within {{
  border-color: alpha(@toolport_accent, 0.62);
  background-color: @toolport_card;
  box-shadow: 0 0 0 2px alpha(@toolport_accent, 0.10);
}}

.toolport-card {{
  background-color: transparent;
  border: none;
  border-bottom: 1px solid @toolport_border;
  border-radius: 0;
  padding: 12px 4px;
  transition: 120ms ease;
}}

.toolport-card:hover {{
  background-color: alpha(@toolport_accent, 0.04);
}}

/* The name is a flat button for opening details; drop button padding so it
   lines up with the origin and status lines under it. */
.toolport-card button.toolport-card-title {{
  padding: 0;
  margin: 0;
  min-height: 0;
  background: none;
  box-shadow: none;
}}

.toolport-card button.toolport-card-title:hover label {{
  text-decoration: underline;
}}

.toolport-details-expander {{
  padding: 11px 14px;
  border: 1px solid @toolport_border;
  border-radius: 12px;
  color: @toolport_fg;
  background-color: @toolport_card;
}}

.toolport-details-expander:hover {{
  border-color: alpha(@toolport_accent, 0.24);
  background-color: mix(@toolport_card, @toolport_fg, 0.04);
}}

.toolport-details-expander > title {{
  min-height: 30px;
  font-weight: 600;
}}

.toolport-stat-row {{
  min-height: 28px;
  padding: 2px 4px;
}}

.toolport-stat-metrics {{
  font-variant-numeric: tabular-nums;
}}

.toolport-settings-parent {{ font-weight: 600; }}
.toolport-safety-card {{ border: 1px solid @toolport_border; border-radius: 12px; padding: 16px; background-color: @toolport_card; }}
.toolport-safety-card:checked {{ border-color: @toolport_accent; background: mix(@toolport_card, @toolport_accent, 0.08); }}
.toolport-settings-group.toolport-switch-group {{
  border: 1px solid @toolport_border;
  border-radius: 12px;
  background-color: @toolport_card;
}}
.toolport-settings-group {{
  background-color: transparent;
  border: none;
  border-top: 1px solid @toolport_border;
  border-radius: 0;
}}

/* Groups whose children are plain form fields rather than `toolport-setting-row`,
   which is where the other groups get their inner padding from. */
.toolport-settings-group.toolport-padded-group {{
  padding: 16px 18px;
}}

/* Text-colour equivalents of the badge variants, for rows that state their
   status in the subtitle instead of repeating it in a chip. */
.toolport-state-success {{
  color: @toolport_success;
}}
.toolport-state-review {{
  color: @toolport_warning;
}}
.toolport-state-muted {{
  color: @toolport_off;
}}

/* Wider than `toolport-summary-item`, which is sized for a number over a
   one-word label rather than a heading over a wrapped sentence. */
.toolport-value-card {{
  background-color: transparent;
  border: none;
  border-bottom: 1px solid @toolport_border;
  border-radius: 0;
  padding: 15px 17px;
}}

.toolport-setting-row {{
  padding: 11px 14px;
  border-bottom: 1px solid alpha(@toolport_fg, 0.075);
}}

.toolport-setting-row:last-child {{
  border-bottom: none;
}}

.toolport-project-files {{
  background-color: @toolport_card;
  border: 1px solid @toolport_border;
  border-radius: 12px;
}}

.toolport-project-file-row {{
  padding: 11px 13px;
  border-bottom: 1px solid alpha(@toolport_fg, 0.07);
}}

.toolport-project-file-row:last-child {{
  border-bottom: none;
}}

.toolport-card button.toolport-secondary-action {{
  background-color: @toolport_raised;
  border: 1px solid @toolport_border;
}}

.toolport-card-icon {{
  min-width: 32px;
  min-height: 32px;
  margin: 1px;
  border-radius: 8px;
  background-color: alpha(@toolport_accent, 0.12);
  color: @toolport_accent;
}}

.toolport-muted {{ color: @toolport_text_secondary; }}

/* One status chip: a pill with the status color at 12% and full-color text,
   no border. The React Badge variants use the same colors. `review` and
   `approval-warning` are warning; `disabled` is off. */
.toolport-badge {{
  background-color: @toolport_info_tint;
  color: @toolport_text_secondary;
  border: none;
  border-radius: 999px;
  padding: 2px 8px;
}}

.toolport-badge.success {{
  background-color: alpha(@toolport_success, 0.12);
  color: @toolport_success;
}}

.toolport-badge.off,
.toolport-badge.info,
.toolport-badge.disabled {{
  background-color: @toolport_info_tint;
  color: @toolport_off;
}}

.toolport-badge.warning,
.toolport-badge.review,
.toolport-badge.approval-warning {{
  color: @toolport_warning;
  background-color: alpha(@toolport_warning, 0.12);
}}

.toolport-badge.partial {{
  color: @toolport_partial;
  background-color: alpha(@toolport_partial, 0.12);
}}

.toolport-badge.error,
.toolport-badge.approval-denied {{
  color: @toolport_error;
  background-color: alpha(@toolport_error, 0.12);
}}
image.approval-denied {{ color: @toolport_error; }}
image.approval-warning {{ color: @toolport_warning; }}
image.success {{
  color: @toolport_success;
  -gtk-icon-palette: error @toolport_success, warning @toolport_success, success @toolport_success;
}}
image.disabled {{ color: @toolport_off; }}

.toolport-catalog-added {{
  min-height: 30px;
  padding: 0 8px;
  border: none;
  border-radius: 8px;
  background-color: alpha(@toolport_success, 0.12);
  color: @toolport_success;
  font-size: 12px;
  font-weight: 600;
}}

button.toolport-catalog-action {{
  min-height: 32px;
  padding: 0 8px;
  border-radius: 8px;
}}

.toolport-server-state {{
  color: @toolport_text_secondary;
  font-size: 13px;
}}

.toolport-card switch {{
  margin-left: 2px;
}}

/* Track: accent when on, the foreground at 18% when off. The knob is white in
   both states and in both modes, as in React. */
.toolport-native switch {{
  color: @toolport_fg;
  background-color: alpha(@toolport_fg, 0.18);
  border: none;
  box-shadow: none;
}}

.toolport-native switch:checked {{
  color: @toolport_bg_dark;
  background-color: @toolport_accent;
}}

.toolport-native switch slider,
.toolport-native switch:checked slider {{
  background-color: white;
  box-shadow: 0 1px 2px alpha(black, 0.30);
}}

.toolport-action-item {{
  min-width: 150px;
  min-height: 36px;
  padding: 0 10px;
  border-radius: 8px;
  background-color: transparent;
  border: none;
}}

.toolport-action-item:hover {{
  background-color: alpha(@toolport_fg, 0.07);
}}

.toolport-action-item.destructive-action {{
  color: @toolport_error;
}}

.toolport-state-card {{
  min-height: 180px;
  padding: 28px;
  border: 1px dashed alpha(@toolport_fg, 0.15);
  border-radius: 12px;
  background-color: @toolport_card;
}}

.toolport-state-icon {{ color: @toolport_accent; }}
.toolport-state-card.error {{
  border-color: alpha(@toolport_error, 0.28);
}}
.toolport-state-card.error .toolport-state-icon {{ color: @toolport_error; }}

.toolport-native .title-2 {{ font-size: 22px; font-weight: 600; }}
.toolport-native .title-3 {{ font-size: 17px; font-weight: 600; }}
.toolport-native .heading {{ font-weight: 600; }}
.toolport-section-label {{
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.6px;
  text-transform: uppercase;
  color: @toolport_muted;
}}

/* Clients page: one line per client, controls on the right. */
.toolport-card.toolport-client-row {{ padding: 9px 4px; }}
.toolport-client-row .toolport-client-logo {{ min-width: 28px; min-height: 28px; }}
.toolport-card.toolport-client-profile {{ padding-left: 44px; }}
.toolport-client-tile {{
  border: 1px solid @toolport_border;
  border-radius: 12px;
  padding: 10px 12px;
  background-color: @toolport_card;
}}
.toolport-client-tile:hover {{ background-color: mix(@toolport_card, @toolport_accent, 0.05); }}
button.toolport-client-group-toggle {{
  padding: 0;
  margin: 0;
  min-height: 0;
  background: none;
  box-shadow: none;
}}
.toolport-client-row .toolport-badge.caption {{ padding: 1px 8px; }}

/* Skills page: the React view's list-and-detail layout with one status column per agent. */
.toolport-skill-tabbar {{
  border-bottom: 1px solid @toolport_border;
}}
button.toolport-skill-tab {{
  border-radius: 0;
  padding: 4px 2px 9px 2px;
  min-height: 0;
  background: none;
  box-shadow: none;
  border-bottom: 2px solid transparent;
  color: @toolport_text_secondary;
}}
button.toolport-skill-tab:hover {{ color: @toolport_fg; background: none; }}
button.toolport-skill-tab:checked {{
  color: @toolport_fg;
  font-weight: 600;
  background: none;
  border-bottom-color: @toolport_accent;
}}
.toolport-skill-count {{
  background-color: @toolport_info_tint;
  color: @toolport_text_secondary;
  border-radius: 999px;
  padding: 0 6px;
  font-weight: 500;
}}
.toolport-skill-count.warning {{
  background-color: alpha(@toolport_warning, 0.12);
  color: @toolport_warning;
}}
.toolport-skill-banner {{
  border: 1px solid @toolport_border;
  border-left: 3px solid @toolport_warning;
  border-radius: 12px;
  background-color: @toolport_card;
  padding: 12px 14px;
  margin-bottom: 14px;
}}
.toolport-skill-tile {{
  border-radius: 8px;
  background-color: alpha(@toolport_accent, 0.12);
  color: @toolport_accent;
}}
.toolport-skill-tile.warning {{ background-color: alpha(@toolport_warning, 0.12); color: @toolport_warning; }}
.toolport-skill-tile.error {{ background-color: alpha(@toolport_error, 0.12); color: @toolport_error; }}
/* Info is neutral in both shells. */
.toolport-skill-tile.info {{ background-color: @toolport_info_tint; color: @toolport_info; }}
.toolport-skill-head {{
  border-bottom: 1px solid @toolport_border;
  padding: 0 8px 6px 8px;
}}
.toolport-skill-head button.toolport-skill-col {{
  padding: 2px 0;
  margin: 0;
  min-width: 0;
  min-height: 0;
  border-radius: 8px;
}}
.toolport-skill-head button.toolport-skill-col.selected {{
  background-color: alpha(@toolport_accent, 0.16);
  box-shadow: inset 0 0 0 1px alpha(@toolport_accent, 0.60);
}}
.toolport-skill-head .toolport-client-logo {{ min-width: 18px; min-height: 18px; margin: 0; }}
.toolport-skill-logo .toolport-client-logo {{ min-width: 20px; min-height: 20px; margin: 0; }}
.toolport-skill-chip .toolport-client-logo {{ min-width: 14px; min-height: 14px; margin: 0; }}
.toolport-skill-pip {{
  margin: 1px 3px;
  min-width: 6px;
  min-height: 6px;
  border-radius: 999px;
  background-color: @toolport_accent;
}}
list.toolport-skill-list {{ background: transparent; }}
list.toolport-skill-list > row {{ border-radius: 8px; padding: 0; }}
list.toolport-skill-list > row:hover {{ background-color: alpha(@toolport_fg, 0.04); }}
list.toolport-skill-list > row:selected {{ background-color: alpha(@toolport_accent, 0.14); }}
list.toolport-skill-list > row.toolport-skill-group-row:hover {{ background: none; }}
.toolport-skill-row {{ padding: 7px 8px; }}
.toolport-skill-name {{ font-weight: 600; }}
.toolport-skill-group {{ padding: 14px 8px 5px 8px; }}
.toolport-skill-dot {{
  min-width: 12px;
  min-height: 12px;
  border-radius: 999px;
}}
.toolport-skill-dot.on {{ background-color: @toolport_success; }}
.toolport-skill-dot.name-only {{
  min-width: 8px;
  min-height: 8px;
  border: 2px solid @toolport_partial;
  background-image: linear-gradient(90deg, @toolport_partial 50%, transparent 50%);
}}
.toolport-skill-dot.shadowed {{
  min-width: 8px;
  min-height: 8px;
  border: 2px solid @toolport_warning;
}}
.toolport-skill-dot.broken {{
  min-width: 10px;
  min-height: 10px;
  border: 2px solid @toolport_error;
}}
.toolport-skill-dot-core {{
  min-width: 4px;
  min-height: 4px;
  border-radius: 999px;
  background-color: @toolport_error;
}}
.toolport-skill-dot.none {{
  min-width: 4px;
  min-height: 4px;
  background-color: alpha(@toolport_fg, 0.28);
}}
.toolport-skill-off {{ color: @toolport_off; }}
.toolport-skill-legend {{
  border-top: 1px solid @toolport_border;
  padding-top: 8px;
  margin-top: 6px;
}}
.toolport-skill-agents {{
  border: 1px solid @toolport_border;
  border-radius: 12px;
  background-color: @toolport_card;
}}
.toolport-skill-agent {{
  padding: 10px 12px;
  border-bottom: 1px solid @toolport_border;
}}
.toolport-skill-agent:last-child {{ border-bottom: none; }}
.toolport-skill-logo {{
  min-width: 32px;
  min-height: 32px;
  border-radius: 8px;
  background-color: alpha(@toolport_fg, 0.06);
}}
levelbar.toolport-skill-meter trough {{
  min-height: 5px;
  border-radius: 999px;
  background-color: alpha(@toolport_fg, 0.10);
  border: none;
  box-shadow: none;
  padding: 0;
}}
levelbar.toolport-skill-meter block {{ min-height: 5px; border-radius: 999px; border: none; }}
levelbar.toolport-skill-meter block.filled {{ background-color: @toolport_success; }}
levelbar.toolport-skill-meter block.empty {{ background: none; }}
levelbar.toolport-skill-meter.over block.filled {{ background-color: @toolport_warning; }}
.toolport-skill-issue {{
  border: 1px solid @toolport_border;
  border-radius: 12px;
  background-color: @toolport_card;
  padding: 16px;
}}
.toolport-skill-chip {{
  background-color: alpha(@toolport_fg, 0.07);
  border-radius: 8px;
  padding: 3px 7px;
}}
.toolport-skill-chip.logo {{ padding: 0; }}
button.toolport-skill-chip {{ min-height: 0; min-width: 0; padding: 3px 8px; }}
button.toolport-skill-chip:hover {{ background-color: alpha(@toolport_fg, 0.13); }}
.toolport-skill-segments button {{ padding: 2px 9px; min-height: 24px; }}

/* Sync page: one amber block for everything that needs a decision. */
.toolport-attention {{
  border-left: 3px solid @toolport_warning;
  border-radius: 0 12px 12px 0;
  background-color: alpha(@toolport_warning, 0.08);
}}

.toolport-attention-item {{
  padding: 10px 14px;
  border-bottom: 1px solid alpha(@toolport_warning, 0.14);
}}

.toolport-attention-item:last-child {{
  border-bottom: none;
}}

.toolport-sync-row {{
  padding: 9px 2px;
  border-bottom: 1px solid @toolport_border;
}}

.toolport-sync-row:last-child {{
  border-bottom: none;
}}

.toolport-sync-choice {{
  padding: 2px;
  border-radius: 8px;
  background-color: @toolport_card;
  border: 1px solid @toolport_border;
}}

.toolport-sync-choice > button {{
  min-height: 24px;
  padding: 2px 10px;
  border-radius: 8px;
  border: none;
  background: none;
  box-shadow: none;
  font-size: 12px;
  font-weight: 500;
  color: @toolport_text_secondary;
}}

.toolport-sync-choice > button:checked {{
  background-color: alpha(@toolport_fg, 0.14);
  color: @toolport_fg;
  font-weight: 600;
}}

.toolport-sync-tag {{
  color: @toolport_warning;
}}

.toolport-sync-status {{
  color: @toolport_text_secondary;
}}

.toolport-sync-status.healthy {{
  color: @toolport_success;
}}
.toolport-muted {{ font-size: 13px; }}
.toolport-native button {{ border-radius: 8px; font-weight: 600; }}
.toolport-feedback.warning {{
  border-color: @toolport_warning;
  background-color: alpha(@toolport_warning, 0.08);
  color: @toolport_fg;
}}
.toolport-feedback .heading {{ color: @toolport_fg; }}
.toolport-feedback + .toolport-feedback {{
  border-top: 1px solid @toolport_border;
}}

.toolport-native .toolport-badge {{
  font-size: 12px;
  font-weight: 500;
}}

selection {{
  background: @toolport_selection;
  color: @toolport_bg_dark;
}}
button.suggested-action {{
  background-color: @toolport_accent;
  color: @toolport_bg_dark;
}}

window:backdrop .toolport-card,
window:backdrop .toolport-summary-item {{
  border-color: alpha(@toolport_fg, 0.06);
}}
"#,
            card = tokens.card,
            text_secondary = tokens.text_secondary,
            success = tokens.success,
            warning = tokens.warning,
            danger = tokens.danger,
            partial = tokens.partial,
            background = self.background,
            dark_background = self.dark_background,
            lighter_background = self.lighter_background,
            foreground = self.foreground,
            muted = self.muted,
            accent = self.accent,
            selection = self.selection,
            mono_logo_filter = match self.mode {
                ThemeMode::Dark => "invert(1) hue-rotate(180deg)",
                ThemeMode::Light => "none",
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tokens {
    card: String,
    text_secondary: String,
    success: String,
    warning: String,
    danger: String,
    partial: String,
}

type Rgb = [f64; 3];

/// Parses a color that `safe_color` has already validated as `#rrggbb`.
fn rgb(hex: &str) -> Rgb {
    let channel = |at: usize| f64::from(u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0));
    [channel(1), channel(3), channel(5)]
}

fn hex(color: Rgb) -> String {
    let channel = |value: f64| value.round().clamp(0.0, 255.0) as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color[0]),
        channel(color[1]),
        channel(color[2])
    )
}

/// The same blend as GTK's `mix(a, b, amount)`.
fn mix(a: Rgb, b: Rgb, amount: f64) -> Rgb {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * amount)
}

fn luminance(color: Rgb) -> f64 {
    let linear = |value: f64| {
        let value = value / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The first blend of `color` toward `toward` that reaches AA on every surface,
/// and on the 12% status tint over each surface when `tinted`.
fn readable(color: Rgb, toward: Rgb, surfaces: &[Rgb], tinted: bool) -> Rgb {
    (0..=20)
        // Rounded first, so the check holds for the hex value the CSS gets.
        .map(|step| mix(color, toward, f64::from(step) / 20.0).map(f64::round))
        .find(|candidate| {
            surfaces.iter().all(|surface| {
                contrast(*candidate, *surface) >= TEXT_CONTRAST
                    && (!tinted
                        || contrast(*candidate, mix(*surface, *candidate, 0.12)) >= TEXT_CONTRAST)
            })
        })
        .unwrap_or(toward)
}

fn safe_color(candidate: Option<String>, fallback: &str) -> String {
    candidate
        .filter(|value| is_hex_color(value))
        .unwrap_or_else(|| fallback.to_string())
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn palette_path() -> PathBuf {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(dirs::state_dir)
        .unwrap_or_else(|| PathBuf::from(".local/state"));
    state_home.join("omarchy/current/theme/colors.toml")
}

pub(super) struct ThemeController {
    path: PathBuf,
    provider: gtk::CssProvider,
    monitors: RefCell<Vec<gio::FileMonitor>>,
}

impl ThemeController {
    pub(super) fn new() -> Rc<Self> {
        Rc::new(Self {
            path: palette_path(),
            provider: gtk::CssProvider::new(),
            monitors: RefCell::new(Vec::new()),
        })
    }

    pub(super) fn attach(self: &Rc<Self>, window: &adw::ApplicationWindow) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        gtk::style_context_add_provider_for_display(
            &display,
            &self.provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        self.reload();
        self.start_monitors();

        let controller = Rc::clone(self);
        window.connect_destroy(move |_| {
            controller.monitors.borrow_mut().clear();
        });
    }

    fn reload(&self) {
        let palette = OmarchyPalette::load(&self.path);
        self.provider.load_from_data(&palette.css());
        let scheme = match palette.mode {
            ThemeMode::Light => adw::ColorScheme::ForceLight,
            ThemeMode::Dark => adw::ColorScheme::ForceDark,
        };
        adw::StyleManager::default().set_color_scheme(scheme);
    }

    fn start_monitors(self: &Rc<Self>) {
        if !self.monitors.borrow().is_empty() {
            return;
        }

        let mut monitors = Vec::new();
        if let Ok(monitor) = gio::File::for_path(&self.path)
            .monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        {
            connect_reload(&monitor, Rc::downgrade(self));
            monitors.push(monitor);
        }

        if let Some(current_dir) = self.path.parent().and_then(Path::parent) {
            if let Ok(monitor) = gio::File::for_path(current_dir)
                .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            {
                connect_reload(&monitor, Rc::downgrade(self));
                monitors.push(monitor);
            }
        }
        *self.monitors.borrow_mut() = monitors;
    }
}

fn connect_reload(monitor: &gio::FileMonitor, controller: Weak<ThemeController>) {
    monitor.connect_changed(move |_, _, _, _| {
        if let Some(controller) = controller.upgrade() {
            controller.reload();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_omarchy_foundational_palette() {
        let palette = OmarchyPalette::parse(
            r##"
mode = "light"
background = "#f0f1f2"
dark_background = "#d0d1d2"
lighter_background = "#ffffff"
foreground = "#101112"
muted = "#606162"
accent = "#3366ff"
selection = "#ccddee"
red = "#ff3344"
green = "#22aa66"
"##,
        )
        .unwrap();

        assert_eq!(palette.mode, ThemeMode::Light);
        assert_eq!(palette.background, "#f0f1f2");
        assert_eq!(palette.accent, "#3366ff");
        assert_eq!(palette.green, "#22aa66");
    }

    #[test]
    fn invalid_colors_cannot_enter_css() {
        let palette = OmarchyPalette::parse(
            r##"
background = "#123456; } window { color: red;"
accent = "not-a-color"
foreground = "#abcdef"
"##,
        )
        .unwrap();

        assert_eq!(palette.background, FALLBACK_BACKGROUND);
        assert_eq!(palette.accent, FALLBACK_ACCENT);
        assert_eq!(palette.foreground, "#abcdef");
        assert!(!palette.css().contains("window { color"));
    }

    #[test]
    fn incomplete_palette_uses_role_specific_fallbacks() {
        let palette = OmarchyPalette::parse("mode = \"dark\"\naccent = \"#123abc\"\n").unwrap();

        assert_eq!(palette.mode, ThemeMode::Dark);
        assert_eq!(palette.accent, "#123abc");
        assert_eq!(palette.background, FALLBACK_BACKGROUND);
        assert_eq!(palette.red, FALLBACK_RED);
    }

    #[test]
    fn malformed_toml_fails_closed_to_the_caller() {
        assert!(OmarchyPalette::parse("background = [").is_err());
    }

    #[test]
    fn all_generated_color_tokens_are_validated_hex_values() {
        let palette = OmarchyPalette::default();
        for value in [
            &palette.background,
            &palette.dark_background,
            &palette.lighter_background,
            &palette.foreground,
            &palette.muted,
            &palette.accent,
            &palette.selection,
            &palette.red,
            &palette.green,
        ] {
            assert!(is_hex_color(value));
        }
    }

    #[test]
    fn generated_css_draws_opaque_surfaces() {
        let css = OmarchyPalette::default().css();

        assert!(css.contains("font-size: 14px"));
        assert!(css.contains(".toolport-native .caption"));
        assert!(!css.contains("font-size: 0.88em"));
        assert!(css.contains(".toolport-shell { background-color: @toolport_bg; }"));
        assert!(css.contains("--card-bg-color: @toolport_card"));
        assert!(css.contains("--dialog-bg-color: @toolport_raised"));
        assert!(css.contains("--popover-bg-color: @toolport_raised"));
        assert!(css.contains("window.messagedialog"));
        assert!(css.contains("dialog-host > dialog.alert sheet"));
        // Only tints, hover, selection and borders may use alpha; a surface
        // color with alpha would let the wallpaper or the rows behind a menu
        // show through.
        for surface in [
            "@toolport_bg",
            "@toolport_bg_dark",
            "@toolport_surface",
            "@toolport_card",
        ] {
            assert!(
                !css.contains(&format!("background-color: alpha({surface},")),
                "{surface}"
            );
        }
        for selector in [
            ".toolport-native {",
            ".toolport-sidebar {",
            ".toolport-header {",
            "popover.toolport-menu > contents {",
            "popover.toolport-main-menu > contents {",
            "popover.toolport-catalog-suggestions > contents {",
            "dropdown popover > contents {",
            "tooltip.background {",
            ".toolport-search {",
            ".toolport-skill-issue {",
            ".toolport-settings-group.toolport-switch-group {",
        ] {
            let start = css.find(selector).expect("the selector is generated");
            let rule = &css[start..start + css[start..].find('}').unwrap()];
            let background = rule
                .lines()
                .find(|line| line.trim_start().starts_with("background-color:"))
                .unwrap_or_else(|| panic!("{selector} sets a background"));
            assert!(!background.contains("alpha("), "{selector}: {background}");
            assert!(
                !background.contains("transparent"),
                "{selector}: {background}"
            );
        }
    }

    #[test]
    fn switch_knob_is_white_in_both_states() {
        let css = OmarchyPalette::default().css();
        let start = css.find(".toolport-native switch slider,").unwrap();
        let rule = &css[start..start + css[start..].find('}').unwrap()];
        assert!(rule.contains(".toolport-native switch:checked slider"));
        assert!(rule.contains("background-color: white;"));
    }

    #[test]
    fn one_warning_and_partial_color_replace_the_hardcoded_ones() {
        let css = OmarchyPalette::default().css();
        for old in [
            "#f0c674",
            "#956016",
            "#d9a441",
            "#38bdf8",
            "@warning_color",
            "@purple_2",
        ] {
            assert!(
                !css.replace("@define-color toolport_warning #f0c674;", "")
                    .replace("@define-color toolport_partial #38bdf8;", "")
                    .contains(old),
                "{old}"
            );
        }
        let palette =
            OmarchyPalette::parse("mode = \"dark\"\nyellow = \"#e8c66a\"\ncyan = \"#7dd7dc\"\n")
                .unwrap();
        let css = palette.css();
        assert!(css.contains("@define-color toolport_warning #e8c66a;"));
        assert!(css.contains("@define-color toolport_partial #7dd7dc;"));
    }

    /// Stock Omarchy palettes that stress the tokens: a dark theme whose `muted`
    /// is a border color, and light themes with pale status colors.
    const STOCK_PALETTES: [&str; 4] = [
        // catppuccin
        r##"mode = "dark"
background = "#1e1e2e"
dark_background = "#161622"
lighter_background = "#313244"
foreground = "#cdd6f4"
muted = "#585b70"
accent = "#89b4fa"
red = "#f38ba8"
yellow = "#f9e2af"
green = "#a6e3a1"
cyan = "#94e2d5"
"##,
        // catppuccin-latte
        r##"mode = "light"
background = "#eff1f5"
dark_background = "#e3e4e8"
lighter_background = "#dce0e8"
foreground = "#4c4f69"
muted = "#acb0be"
accent = "#1e66f5"
red = "#d20f39"
yellow = "#df8e1d"
green = "#40a02b"
cyan = "#179299"
"##,
        // flexoki-light
        r##"mode = "light"
background = "#FFFCF0"
dark_background = "#f2efe4"
lighter_background = "#E6E4D9"
foreground = "#100F0F"
muted = "#B7B5AC"
accent = "#205EA6"
red = "#D14D41"
yellow = "#D0A215"
green = "#879A39"
cyan = "#3AA99F"
"##,
        // tokyo-night
        r##"mode = "dark"
background = "#1a1b26"
dark_background = "#13141c"
lighter_background = "#24283b"
foreground = "#a9b1d6"
muted = "#414868"
accent = "#7aa2f7"
red = "#f7768e"
yellow = "#e0af68"
green = "#9ece6a"
cyan = "#449dab"
"##,
    ];

    #[test]
    fn secondary_and_status_text_meet_aa_on_every_surface() {
        for input in STOCK_PALETTES.iter().copied().chain(std::iter::once("")) {
            let palette = OmarchyPalette::parse(input).unwrap();
            let tokens = palette.tokens();
            let surfaces = [
                rgb(&palette.background),
                rgb(&palette.dark_background),
                rgb(&tokens.card),
            ];
            for (name, color) in [
                ("text_secondary", &tokens.text_secondary),
                ("success", &tokens.success),
                ("warning", &tokens.warning),
                ("danger", &tokens.danger),
                ("partial", &tokens.partial),
            ] {
                for surface in surfaces {
                    let ratio = contrast(rgb(color), surface);
                    assert!(
                        ratio >= TEXT_CONTRAST,
                        "{name} {color} is {ratio:.2}:1 in {input}"
                    );
                }
            }
        }
    }

    #[test]
    fn readable_colors_keep_the_theme_color_when_it_already_passes() {
        let palette = OmarchyPalette::default();
        let tokens = palette.tokens();
        assert_eq!(tokens.text_secondary, FALLBACK_MUTED);
        assert_eq!(tokens.success, FALLBACK_GREEN);
        assert_eq!(tokens.warning, FALLBACK_YELLOW);
    }

    #[test]
    fn navigation_and_status_text_never_use_the_muted_border_colour() {
        let css = OmarchyPalette::default().css();
        for selector in [
            ".toolport-nav-item {",
            ".toolport-mode-badge {",
            ".toolport-state-muted {",
        ] {
            let start = css.find(selector).expect("the selector is generated");
            let rule = &css[start..start + css[start..].find('}').unwrap()];
            assert!(!rule.contains("@toolport_muted"), "{selector}");
        }
    }
}
