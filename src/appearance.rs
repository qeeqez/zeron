//! Appearance application: fonts, contrast and window chrome pushed onto the
//! gpui-component `Theme` global after `Theme::change` (which resets them to
//! the stock config). Split from `workspace.rs` to stay under the SLOC cap.

use gpui_kit::component::Root;
use gpui_kit::component::theme::Theme;
use gpui_kit::*;

use crate::workspace::Workspace;

/// Font-size bounds shared by the interface and code steppers.
pub(crate) const FONT_SIZE_MIN: u8 = 10;
pub(crate) const FONT_SIZE_MAX: u8 = 24;
/// Contrast bounds: percent of the theme's stock chrome intensity.
pub(crate) const CONTRAST_MIN: u16 = 50;
pub(crate) const CONTRAST_MAX: u16 = 200;
/// Minimum foreground/background lightness separation the contrast floor
/// guarantees — below this text stops being legible.
pub(crate) const MIN_LEGIBLE_DELTA: f32 = 0.45;
/// Alpha of the sidebar's translucent fill when frosted glass is on.
/// High enough to read as frosted glass over the blurred window, low
/// enough that the blur still shows through.
pub(crate) const FROSTED_SIDEBAR_ALPHA: f32 = 0.7;

impl Workspace {
    /// Resolve the configured appearance and apply it. "system" maps the OS
    /// window appearance to a concrete mode; anything else is used as-is.
    pub(crate) fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::theme::ThemeMode;
        let mode = match self.theme.as_str() {
            "light" => ThemeMode::Light,
            "dark" => ThemeMode::Dark,
            _ => ThemeMode::from(window.appearance()),
        };
        Theme::change(mode, Some(window), cx);
        // `Theme::change` re-applies the stock config — re-apply the user's
        // fonts, contrast and window chrome on top.
        self.apply_appearance(window, cx);
    }

    /// Push the user's font, contrast and sidebar-translucency choices onto
    /// the live theme. Runs after every `Theme::change` (which resets these
    /// fields to the stock config) and directly from the Appearance controls.
    pub(crate) fn apply_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        {
            let theme = Theme::global_mut(cx);
            // Restore the stock palette for the active mode before scaling —
            // contrast is computed from the unmodified theme each time, so a
            // given percentage always lands on the same colors regardless of
            // slider history.
            let config = if theme.mode.is_dark() { theme.dark_theme.clone() } else { theme.light_theme.clone() };
            theme.apply_config(&config);
            theme.font_family = if self.font_family.is_empty() { ".SystemUIFont".into() } else { self.font_family.clone().into() };
            theme.font_size = px(f32::from(self.font_size));
            if !self.code_font_family.is_empty() {
                theme.mono_font_family = self.code_font_family.clone().into();
            }
            theme.mono_font_size = px(f32::from(self.code_font_size));
            apply_contrast(theme, self.contrast);
        }
        // Push the mutated theme down to the Base layer (scrollbars, text
        // view defaults) — `global_mut` alone leaves them stale.
        Theme::sync_base(cx);
        self.apply_window_chrome(window, cx);
        window.refresh();
        cx.notify();
    }

    /// Window-level chrome for the frosted sidebar: the blurred window
    /// background only shows through where rendered pixels are transparent,
    /// so the `Root` layer's opaque theme fill must come off while frosting
    /// is on (the sidebar paints its own translucent fill, the chat pane its
    /// own opaque one).
    fn apply_window_chrome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let frosted = self.sidebar_frosted;
        window.set_background_appearance(window_background_appearance(frosted));
        if let Some(root) = window.root::<Root>().flatten() {
            root.update(cx, |root, cx| {
                root.style().background = frosted_root_background(frosted);
                cx.notify();
            });
        }
    }

    /// Set the interface font family from a picker confirm (`None` restores
    /// the system default), persist and re-apply.
    pub(crate) fn set_interface_font(&mut self, family: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.font_family = family.unwrap_or_default();
        self.save_settings();
        self.apply_appearance(window, cx);
    }

    /// Set the code (mono) font family from a picker confirm, persist and
    /// re-apply.
    pub(crate) fn set_code_font(&mut self, family: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.code_font_family = family.unwrap_or_default();
        self.save_settings();
        self.apply_appearance(window, cx);
    }

    /// Set contrast from a slider event (clamped to [50, 200]); drags emit
    /// `Change` continuously — only `Release` persists to settings.json.
    pub(crate) fn set_contrast(&mut self, event: &gpui_kit::component::slider::SliderEvent, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::slider::SliderEvent;
        let (value, save) = match event {
            SliderEvent::Change(v) => (v.end(), false),
            SliderEvent::Release(v) => (v.end(), true),
        };
        self.contrast = (value as u16).clamp(CONTRAST_MIN, CONTRAST_MAX);
        if save {
            self.save_settings();
        }
        self.apply_appearance(window, cx);
    }
}

/// The window's background appearance: `Blurred` while the frosted sidebar
/// is on (the sidebar's translucent fill sits over the blur), `Opaque`
/// otherwise. `open_workspace_window` uses this for `WindowOptions` so a
/// window opened with frosting off never starts blurred.
pub(crate) fn window_background_appearance(frosted: bool) -> WindowBackgroundAppearance {
    if frosted { WindowBackgroundAppearance::Blurred } else { WindowBackgroundAppearance::Opaque }
}

/// The `Root` layer's background while the frosted sidebar is on: transparent,
/// so the window's `Blurred` background shows through the sidebar region.
/// `None` restores the stock theme fill.
pub(crate) fn frosted_root_background(frosted: bool) -> Option<Fill> {
    frosted.then(|| Fill::from(transparent_black()))
}

/// The sidebar column's fill: translucent over the blurred window when
/// frosted (real vibrancy), the stock opaque color otherwise.
pub(crate) fn sidebar_fill(theme: &Theme, frosted: bool) -> Hsla {
    if frosted { theme.sidebar.opacity(FROSTED_SIDEBAR_ALPHA) } else { theme.sidebar }
}

/// Scale every theme color's lightness distance from the stock
/// foreground/background midpoint by `contrast` percent — 100 reproduces the
/// stock theme exactly, below softens, above sharpens. The scale is floored
/// so fg/bg separation never drops under `MIN_LEGIBLE_DELTA`, even on themes
/// that start nearly flat. Callers must restore stock colors first; this
/// scales whatever palette is currently on the theme.
fn apply_contrast(theme: &mut Theme, contrast: u16) {
    let contrast = contrast.clamp(CONTRAST_MIN, CONTRAST_MAX);
    let mid = (theme.foreground.l + theme.background.l) / 2.;
    let stock_delta = (theme.foreground.l - theme.background.l).abs();
    let mut scale = f32::from(contrast) / 100.;
    if stock_delta > 0. {
        scale = scale.max(MIN_LEGIBLE_DELTA / stock_delta);
    }
    let colors = &mut theme.colors;
    let tokens = &mut theme.tokens;
    macro_rules! scale_color {
        ($($field:ident),+ $(,)?) => {
            $(
                colors.$field = scale_lightness(colors.$field, mid, scale);
                tokens.$field.color = colors.$field;
                if tokens.$field.background.as_solid().is_some() {
                    tokens.$field.background = colors.$field.into();
                }
            )+
        };
    }
    scale_color!(
        accent, accent_foreground, accordion, background, border,
        button, button_active, button_foreground, button_hover,
        button_danger, button_danger_active, button_danger_foreground, button_danger_hover,
        button_info, button_info_active, button_info_foreground, button_info_hover,
        button_primary, button_primary_active, button_primary_foreground, button_primary_hover,
        button_secondary, button_secondary_active, button_secondary_foreground, button_secondary_hover,
        button_success, button_success_active, button_success_foreground, button_success_hover,
        button_warning, button_warning_active, button_warning_foreground, button_warning_hover,
        group_box, group_box_foreground, caret,
        chart_1, chart_2, chart_3, chart_4, chart_5, chart_bullish, chart_bearish,
        danger, danger_active, danger_foreground, danger_hover,
        description_list_label, description_list_label_foreground,
        drag_border, drop_target, foreground,
        info, info_active, info_foreground, info_hover, input,
        link, link_active, link_hover,
        list, list_active, list_active_border, list_even, list_head, list_hover,
        muted, muted_foreground, popover, popover_foreground,
        primary, primary_active, primary_foreground, primary_hover, progress_bar, ring,
        scrollbar, scrollbar_thumb, scrollbar_thumb_hover,
        secondary, secondary_active, secondary_foreground, secondary_hover, selection,
        sidebar, sidebar_accent, sidebar_accent_foreground, sidebar_border, sidebar_foreground,
        sidebar_primary, sidebar_primary_foreground, skeleton, slider_bar, slider_thumb,
        success, success_foreground, success_hover, success_active, switch, switch_thumb,
        tab, tab_active, tab_active_foreground, tab_bar, tab_bar_segmented, tab_foreground,
        table, table_active, table_active_border, table_even, table_head, table_head_foreground,
        table_foot, table_foot_foreground, table_hover, table_row_border,
        title_bar, title_bar_border, status_bar, status_bar_border, tiles,
        warning, warning_active, warning_hover, warning_foreground,
        overlay, window_border,
        red, red_light, green, green_light, blue, blue_light,
        yellow, yellow_light, magenta, magenta_light, cyan, cyan_light,
    );
}

/// Move `color`'s lightness toward or away from `mid` by `scale`, clamped to
/// [0, 1]. Hue, saturation and alpha are preserved.
fn scale_lightness(color: Hsla, mid: f32, scale: f32) -> Hsla {
    Hsla { l: (mid + (color.l - mid) * scale).clamp(0., 1.), ..color }
}
