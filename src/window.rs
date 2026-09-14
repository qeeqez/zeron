use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::base::InteractiveElementExt;
use gpui_kit::component::sidebar::SidebarToggleButton;
use gpui_kit::*;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSView;
#[cfg(target_os = "macos")]
use objc2_foundation::NSRect;

use crate::workspace::Workspace;

/// Make `el` a window-drag strip: press-and-move calls `start_window_move`,
/// double-click runs the native titlebar action. Needed because the window
/// sets `app_owns_titlebar_drag` — AppKit no longer drags it, so the app does.
/// Interactive children (buttons) stop mousedown propagation, so they still
/// click instead of starting a drag.
pub(crate) fn titlebar_drag(el: Stateful<Div>) -> Stateful<Div> {
    let moving = Rc::new(Cell::new(false));
    let down = moving.clone();
    let up = moving.clone();
    let up_out = moving.clone();
    let mv = moving;
    el.on_mouse_down(MouseButton::Left, move |_, _, _| down.set(true))
        .on_mouse_up(MouseButton::Left, move |_, _, _| up.set(false))
        .on_mouse_up_out(MouseButton::Left, move |_, _, _| up_out.set(false))
        .on_mouse_move(move |_, window, _| {
            if mv.replace(false) {
                window.start_window_move();
            }
        })
        .on_double_click(|_, window, _| window.titlebar_double_click())
}

/// Height of the per-pane top drag strip. Matches the traffic-light
/// container (`button_height + 2·traffic_light_position.y` ≈ 14 + 2·9) so
/// strip content centers on the same line as the lights. The sidebar's strip
/// and the content's titlebar both use it → one continuous titlebar row.
pub(crate) const TOP_BAR_H: f32 = 32.;

/// Sidebar width clamp while dragging the resize handle. `workspace.rs`
/// clamps the persisted value to the same range.
pub(crate) const SIDEBAR_WIDTH_MIN: f32 = 180.;
pub(crate) const SIDEBAR_WIDTH_MAX: f32 = 480.;

/// Whether the sidebar's native vibrancy view should be installed: only
/// while the frosted sidebar is on AND the sidebar is actually mounted —
/// collapsed unmounts it, and a leftover vibrancy strip would glow behind
/// the chat pane's leading edge.
pub(crate) fn sidebar_vibrancy_active(frosted: bool, collapsed: bool) -> bool {
    frosted && !collapsed
}

/// Tag identifying the app's own `NSVisualEffectView` behind the sidebar, so
/// `sync_sidebar_vibrancy` can find, resize or remove it without touching
/// gpui's blurred background view. ("RIXL" in hex.)
#[cfg(target_os = "macos")]
const SIDEBAR_VIBRANCY_TAG: isize = 0x5249_584C;

/// Install, resize or remove the `NSVisualEffectView` that makes the frosted
/// sidebar read as real glass. gpui's `Blurred` window background uses
/// `NSVisualEffectMaterial::Selection` — the weakest material, which reads
/// as "transparent and faded" rather than frosted — and gpui-pre exposes no
/// material choice, so the app layers its own effect view with the `Sidebar`
/// material over the sidebar region, directly below the Metal content view
/// (and above gpui's own blur view, which stays as a fallback).
///
/// Called from `Workspace::render` so the view tracks `sidebar_width` during
/// resize drags and disappears on collapse/unfrost. Headless test windows
/// have no native handle — this no-ops there.
#[cfg(target_os = "macos")]
pub(crate) fn sync_sidebar_vibrancy(window: &Window, sidebar_width: f32, active: bool) {
    use objc2_foundation::{NSPoint, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // `Window::window_handle` returns gpui's AnyWindowHandle — the raw
    // AppKit handle comes from the HasWindowHandle trait method.
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(kit) = handle.as_raw() else { return };
    // SAFETY: the AppKit handle exposes gpui's live Metal view; render runs
    // on the main thread, so the view and its superview are valid to touch.
    let native_view = unsafe { &*kit.ns_view.as_ptr().cast::<NSView>() };
    let Some(content_view) = (unsafe { native_view.superview() }) else { return };
    let existing = content_view.viewWithTag(SIDEBAR_VIBRANCY_TAG);
    if !active {
        if let Some(view) = existing {
            view.removeFromSuperview();
        }
        return;
    }
    // Full content-view height, sidebar_width wide, pinned to the leading
    // edge. Height also tracks via autoresizing; width is set every render
    // so the view follows resize drags.
    let bounds = content_view.bounds();
    let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(f64::from(sidebar_width), bounds.size.height));
    match existing {
        Some(view) => view.setFrame(frame),
        None => install_sidebar_vibrancy(&content_view, native_view, frame),
    }
}

/// Create the sidebar's `NSVisualEffectView` (`Sidebar` material, blending
/// behind the window) and insert it directly below the Metal content view —
/// above gpui's own blur view, which stays as a fallback.
#[cfg(target_os = "macos")]
fn install_sidebar_vibrancy(content_view: &NSView, native_view: &NSView, frame: NSRect) {
    use objc2::{MainThreadMarker, msg_send, rc::Retained};
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
        NSWindowOrderingMode,
    };
    let Some(mtm) = MainThreadMarker::new() else { return };
    // SAFETY: initWithFrame: is NSView's designated initializer; the view is
    // configured before being added to the hierarchy on the main thread.
    let view: Retained<NSVisualEffectView> = unsafe { msg_send![mtm.alloc::<NSVisualEffectView>(), initWithFrame: frame] };
    view.setMaterial(NSVisualEffectMaterial::Sidebar);
    view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    view.setState(NSVisualEffectState::Active);
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
    // SAFETY: setTag: is a plain NSView setter the bindings don't expose.
    unsafe { msg_send![&view, setTag: SIDEBAR_VIBRANCY_TAG] }
    content_view.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(native_view));
}

/// Non-macOS builds have no native vibrancy view — the translucent fill over
/// gpui's blurred window background is all there is.
#[cfg(not(target_os = "macos"))]
pub(crate) fn sync_sidebar_vibrancy(_window: &Window, _sidebar_width: f32, _active: bool) {}

/// The sidebar toggle for the unified top bar. It sits inline in the
/// `titlebar_drag` strip in `Workspace::render`, right of the traffic
/// lights — the same spot whether the sidebar is open or collapsed. The
/// mousedown must stop here: the strip's own listener would otherwise arm a
/// window move under the press and swallow the drag.
pub(crate) fn sidebar_toggle(collapsed: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("sidebar-toggle")
        .test_support()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            SidebarToggleButton::new()
                .collapsed(collapsed)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
        )
}

/// Prompt before closing while a reply is running; persist bounds first.
/// Also stashes the composer text into the active chat's draft so unsent
/// input survives the close.
///
/// `remove_window` re-fires `on_window_should_close`, so the confirmed
/// path must bypass the prompt — a `Cell` flag on the workspace does that.
pub fn confirm_close(ws: &Entity<Workspace>, handle: AnyWindowHandle, window: &mut Window, cx: &mut App) -> bool {
    save_window_bounds(window);
    ws.update(cx, |this, cx| {
        this.chats[this.active].draft = this.composer.read(cx).value().to_string();
        this.save();
    });
    if ws.read(cx).close_confirmed.replace(false) {
        return true;
    }
    if !ws.read(cx).chats.iter().any(|c| c.running) {
        return true;
    }
    let rx = window.prompt(
        gpui_kit::PromptLevel::Warning,
        "A reply is still generating",
        Some("Closing now will stop it."),
        &[gpui_kit::PromptButton::ok("Close"), gpui_kit::PromptButton::cancel("Cancel")],
        cx,
    );
    let ws2 = ws.clone();
    cx.spawn(async move |cx| {
        if rx.await == Ok(0) {
            ws2.update(cx, |this, _| this.close_confirmed.set(true));
            let _ = handle.update(cx, |_, window, _cx| window.remove_window());
        }
    })
    .detach();
    false
}

/// Persist the current window bounds into settings.
fn save_window_bounds(window: &Window) {
    if let gpui_kit::WindowBounds::Windowed(b) = window.window_bounds() {
        let mut s = crate::persist::load_settings();
        s.window_bounds = Some([b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into()]);
        crate::persist::save_settings(&s);
    }
}

/// Restore window bounds from settings, if saved.
pub fn saved_window_bounds() -> Option<gpui_kit::WindowBounds> {
    crate::persist::load_settings().window_bounds.map(|[x, y, w, h]| {
        gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
            origin: gpui_kit::point(px(x), px(y)),
            size: gpui_kit::size(px(w), px(h)),
        })
    })
}
