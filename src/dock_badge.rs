//! The macOS dock badge: the count of chats with an unread reply, summed
//! across every workspace window. `update` recomputes it wherever a chat's
//! `unread` flag flips (turn finished off-screen, apply note landed, chat
//! selected or deleted); `clear` drops it on app focus and quit — the
//! badge's job is to pull the user back, so it has nothing to say while
//! they're already looking.
//!
//! The platform write goes through `LAST_BADGE`, which remembers the label
//! currently on the dock tile so an unchanged count never re-sets it.

#[cfg(test)]
mod tests;

use gpui_kit::*;

use crate::model::Chat;
use crate::workspace::Workspace;
use gpui_kit::component::Root;

/// The label currently on the dock tile — `None` means no badge. Lets
/// `apply` skip the AppKit call when the count hasn't moved.
static LAST_BADGE: parking_lot::Mutex<Option<usize>> = parking_lot::Mutex::new(None);

/// What a recomputed count does to the dock tile.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BadgeChange {
    /// The count didn't move — leave the tile alone.
    Unchanged,
    /// Show this count.
    Set(usize),
    /// Remove the badge.
    Clear,
}

/// Fold `count` into `last` and report the tile update. `last` is the label
/// already showing, so a repeat count is `Unchanged` and the platform call
/// is skipped.
pub(crate) fn transition(last: &mut Option<usize>, count: usize) -> BadgeChange {
    let next = (count > 0).then_some(count);
    if *last == next {
        return BadgeChange::Unchanged;
    }
    *last = next;
    match next {
        Some(n) => BadgeChange::Set(n),
        None => BadgeChange::Clear,
    }
}

/// Chats in `chats` with an unread reply — the badge's per-window count.
pub(crate) fn unread_count(chats: &[Chat]) -> usize {
    chats.iter().filter(|c| c.unread).count()
}

/// Unread chats across every open workspace window — the dock badge is
/// app-global, so a reply landing in a background window still counts.
fn total_unread(cx: &App) -> usize {
    cx.windows()
        .iter()
        .map(|handle| {
            handle
                .read(cx, |root: Entity<Root>, cx| {
                    root.read(cx)
                        .view()
                        .clone()
                        .downcast::<Workspace>()
                        .map(|ws| unread_count(&ws.read(cx).chats))
                        .unwrap_or(0)
                })
                .unwrap_or(0)
        })
        .sum()
}

/// Recompute the badge from the live unread flags. Deferred to the end of
/// the effect cycle: callers run inside a window/entity update where
/// walking `cx.windows()` would re-borrow the window that's mid-update.
pub(crate) fn update(cx: &mut App) {
    cx.defer(|cx| apply(total_unread(cx)));
}

/// Drop the badge outright — app focus and quit, where the count is moot
/// because the user is already looking (or the app is gone).
pub(crate) fn clear() {
    apply(0);
}

/// Register the focus-clear observer on a workspace window — called once
/// per window from `lifecycle::open_workspace_window_for`. Gaining focus
/// clears the badge; deactivation does nothing since the badge only moves
/// when a chat's unread flag flips.
pub(crate) fn clear_on_focus(window: &mut Window, cx: &mut Context<Workspace>) {
    cx.observe_window_activation(window, |_, window, _| {
        if window.is_window_active() {
            clear();
        }
    })
    .detach();
}

/// Apply `count` to the dock tile, skipping the platform call when the
/// label is already right.
pub(crate) fn apply(count: usize) {
    let mut last = LAST_BADGE.lock();
    match transition(&mut last, count) {
        BadgeChange::Unchanged => {},
        BadgeChange::Set(n) => set_label(Some(n.to_string())),
        BadgeChange::Clear => set_label(None),
    }
}

/// The label currently applied — the dedupe state tests assert against.
#[cfg(test)]
pub(crate) fn last_badge() -> Option<usize> {
    *LAST_BADGE.lock()
}

/// `NSDockTile.badgeLabel` — a count string, or `nil` to clear. The dock
/// tile is main-thread-only AppKit state; without a main-thread token
/// (headless test, off-main caller) there's no tile to touch.
#[cfg(target_os = "macos")]
fn set_label(label: Option<String>) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let label = label.map(|s| NSString::from_str(&s));
    app.dockTile().setBadgeLabel(label.as_deref());
}

/// Other platforms badge through their own taskbar APIs gpui doesn't
/// expose — nothing to set.
#[cfg(not(target_os = "macos"))]
fn set_label(_label: Option<String>) {}
