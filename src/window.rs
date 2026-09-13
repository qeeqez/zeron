use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::base::InteractiveElementExt;
use gpui_kit::*;

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
