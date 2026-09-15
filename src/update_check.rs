//! The `Workspace` side of update checking — split from `update.rs` for the
//! SLOC cap: the startup/periodic loop, the manual check, the skip action,
//! and landing a fetched release (state + toast + activity entry).

use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::{Notification, NotificationDelivery};
use gpui_kit::*;

use crate::update::{self, Release, ReleaseOutcome, UpdateStatus};
use crate::workspace::Workspace;

/// Marker for the update toast's notification id — a repeat push replaces
/// the previous toast instead of stacking.
struct UpdateAvailable;

impl Workspace {
    /// Startup + periodic check, spawned from `start_background`. The loop
    /// re-reads `update_last_check` hourly so a long-running window checks
    /// daily; the fetch itself runs on the background executor.
    pub(crate) fn start_update_check(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while update_tick(&this, cx).await {
                cx.background_executor().timer(update::POLL_INTERVAL).await;
            }
        })
        .detach();
    }

    /// The "Check for Updates" menu item and the About row's Check button:
    /// always fetch, always report — the interval gate only applies to the
    /// automatic loop.
    pub(crate) fn check_updates(&mut self, cx: &mut Context<Self>) {
        if matches!(self.update.status, UpdateStatus::Checking) {
            return;
        }
        self.update.status = UpdateStatus::Checking;
        cx.notify();
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| run_update_check(weak, true, cx).await).detach();
    }

    /// The About row's Skip button — dismiss this release so it stops
    /// notifying; a newer tag still lands as a fresh update.
    pub(crate) fn skip_update(&mut self, tag: &str, cx: &mut Context<Self>) {
        self.update.skip(tag);
        cx.notify();
    }

    /// Land a fetch result: update the About state, notify on a new release
    /// (manual checks always report), and record the outcome in the feed.
    fn land_update(&mut self, result: Result<Release, String>, manual: bool, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Ok(release) => match update::apply_release(&mut self.update, &release) {
                ReleaseOutcome::Available => Self::notify_update(&release, window, cx),
                ReleaseOutcome::Skipped => {},
                ReleaseOutcome::UpToDate => {
                    if manual {
                        window.push_notification(
                            Notification::success(format!("You're up to date — {} is the latest release.", env!("CARGO_PKG_VERSION"))),
                            cx,
                        );
                    }
                },
            },
            Err(e) => {
                self.update.status = UpdateStatus::Unknown;
                update::persist_update(|s| s.update_last_check = Some(std::time::SystemTime::now()));
                if manual {
                    window.push_notification(Notification::error(format!("Couldn't check for updates: {e}")), cx);
                }
            },
        }
        cx.notify();
    }

    /// The update-found toast: in-app plus the OS notification center, with
    /// a click-through to the release page.
    fn notify_update(release: &Release, window: &mut Window, cx: &mut Context<Self>) {
        let url = release.url.clone();
        let note = Notification::info(format!("Rixl Code {} is available — you're on {}.", release.tag, env!("CARGO_PKG_VERSION")))
            .title("Update available")
            .id1::<UpdateAvailable>(format!("update-{}", release.tag))
            .delivery(NotificationDelivery::InAppAndSystem)
            .on_click(move |_, _, cx| cx.open_url(&url));
        window.push_notification(note, cx);
    }
}

/// Fetch on the background executor, then land the result on the workspace.
async fn run_update_check(weak: WeakEntity<Workspace>, manual: bool, cx: &mut AsyncApp) {
    let task = cx.background_executor().spawn(async move { update::release_source().latest() });
    let result = task.await;
    let _ = weak.update_in(cx, |this, window, cx| this.land_update(result, manual, window, cx));
}

/// One pass of the periodic loop: check when due. `false` = the workspace
/// is gone and the loop should stop.
async fn update_tick(this: &WeakEntity<Workspace>, cx: &mut AsyncApp) -> bool {
    let Ok(due) = this.update(cx, |_, _| update::update_due()) else { return false };
    if due {
        run_update_check(this.clone(), false, cx).await;
    }
    true
}
