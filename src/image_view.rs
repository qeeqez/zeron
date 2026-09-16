//! The image lightbox: clicking an image thumbnail — a composer attachment
//! chip or an image attached to a sent message — opens a centered overlay
//! with the image scaled to fit the window over a dimmed backdrop. Esc (via
//! the `EscapeKey` handler in `root`), a backdrop click, or the ✕ button
//! closes it. `Workspace::image_view` holds the open path; `None` is closed.

use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// Open the lightbox on `path` — the image renders at full size, scaled
    /// to fit the window.
    pub fn open_image_view(&mut self, path: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.image_view = Some(path.into());
        cx.notify();
    }

    /// Close the lightbox.
    pub fn close_image_view(&mut self, cx: &mut Context<Self>) {
        self.image_view = None;
        cx.notify();
    }
}

/// The overlay root: a full-window dimmed backdrop under a centered stage.
/// The backdrop's hitbox covers the window, so a press anywhere the image
/// doesn't occlude lands on it and closes the lightbox.
pub fn image_view_overlay(path: &SharedString, window: &Window, cx: &mut Context<Workspace>) -> impl IntoElement {
    let file = std::path::PathBuf::from(path.as_str());
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
    let backdrop = div()
        .id("image-view-backdrop")
        .test_support()
        .absolute()
        .inset_0()
        .bg(hsla(0.0, 0.0, 0.0, 0.6))
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_image_view(cx)));
    div()
        .id("image-view-overlay")
        .test_support()
        .absolute()
        .inset_0()
        .child(backdrop)
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(stage(&file, &name, window, cx)),
        )
        .child(close_button(cx))
        .child(caption(&name))
}

/// The centered image, capped at 85% of the window in both axes — `Contain`
/// scales it down to fit. A missing file shows a placeholder instead of an
/// empty frame; a file that exists but fails to decode falls back to the
/// same placeholder via `with_fallback`.
fn stage(file: &std::path::Path, name: &str, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let viewport = window.viewport_size();
    let max_w = px(f32::from(viewport.width) * 0.85);
    let max_h = px(f32::from(viewport.height) * 0.85);
    if !file.is_file() {
        return placeholder(name, cx).into_any_element();
    }
    let muted = cx.theme().muted_foreground;
    div()
        .id("image-view-img")
        .test_support()
        .aria_label(file.to_string_lossy().to_string())
        .occlude()
        .child(
            img(file.to_path_buf())
                .max_w(max_w)
                .max_h(max_h)
                .object_fit(ObjectFit::Contain)
                .rounded_md()
                .with_fallback(move || {
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .text_color(muted)
                        .child(IconName::ImageOff)
                        .child("Couldn't load image")
                        .into_any_element()
                }),
        )
        .into_any_element()
}

/// The missing-file stand-in: an icon plus the file name so the user can
/// tell which attachment went away.
fn placeholder(name: &str, cx: &mut Context<Workspace>) -> ObservedElement<Stateful<Div>> {
    div()
        .id("image-view-missing")
        .test_support()
        .occlude()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .p_6()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .text_color(cx.theme().muted_foreground)
        .child(IconName::ImageOff)
        .child(format!("{name} — file not found"))
}

/// The ✕ button, pinned to the window's top-right corner.
fn close_button(cx: &mut Context<Workspace>) -> ObservedElement<Stateful<Div>> {
    div()
        .id("image-view-close")
        .test_support()
        .absolute()
        .top_4()
        .right_4()
        .flex()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded_md()
        .cursor_pointer()
        .bg(hsla(0.0, 0.0, 0.0, 0.5))
        .text_color(hsla(0.0, 0.0, 1.0, 0.9))
        .child(IconName::X)
        .on_click(cx.listener(|this, _, _, cx| this.close_image_view(cx)))
}

/// The file name, centered along the window's bottom edge.
fn caption(name: &str) -> Div {
    div().absolute().bottom_4().left_0().right_0().flex().justify_center().child(
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .text_sm()
            .bg(hsla(0.0, 0.0, 0.0, 0.5))
            .text_color(hsla(0.0, 0.0, 1.0, 0.9))
            .child(name.to_string()),
    )
}
