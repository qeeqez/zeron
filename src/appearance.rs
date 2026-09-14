//! Appearance application: fonts, contrast and window chrome pushed onto the
//! gpui-component `Theme` global after `Theme::change` (which resets them to
//! the stock config). Split from `workspace.rs` to stay under the SLOC cap.

use gpui_kit::component::theme::Theme;
use gpui_kit::*;

use crate::workspace::Workspace;

/// Font-size bounds shared by the interface and code steppers.
pub(crate) const FONT_SIZE_MIN: u8 = 10;
pub(crate) const FONT_SIZE_MAX: u8 = 24;
/// Contrast bounds: percent of the theme's stock chrome intensity.
pub(crate) const CONTRAST_MIN: u16 = 50;
pub(crate) const CONTRAST_MAX: u16 = 200;

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
        // Frosted sidebar needs the window's blurred background to show
        // through; an opaque sidebar lets the window stay opaque (and keeps
        // subpixel text rendering).
        window.set_background_appearance(if self.sidebar_frosted {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Opaque
        });
        window.refresh();
        cx.notify();
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

/// Scale the theme's chrome colors by `contrast` percent: below 100 they mix
/// toward the background (softer), above 100 toward the foreground (harder).
/// `foreground` and `background` themselves stay fixed as the endpoints.
fn apply_contrast(theme: &mut Theme, contrast: u16) {
    let contrast = contrast.clamp(CONTRAST_MIN, CONTRAST_MAX);
    if contrast == 100 {
        return;
    }
    let (target, t) = if contrast < 100 {
        (theme.background, f32::from(100 - contrast) / 100.)
    } else {
        (theme.foreground, f32::from(contrast - 100) / 100.)
    };
    // `theme.colors` once — field access through `DerefMut` would re-borrow
    // `theme` per field and trip the borrow checker.
    let colors = &mut theme.colors;
    for color in [
        &mut colors.muted_foreground,
        &mut colors.border,
        &mut colors.sidebar_border,
        &mut colors.input,
        &mut colors.list_active_border,
        &mut colors.drag_border,
    ] {
        *color = mix(*color, target, t);
    }
}

/// Linear RGBA lerp from `a` toward `b` by `t` — `Hsla::blend` composites
/// rather than interpolates, so this does the mix by hand.
fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    Hsla::from(Rgba {
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
        a: lerp(a.a, b.a),
    })
}
