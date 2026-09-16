//! The `Workspace` side of update checking — split from `update.rs` for the
//! SLOC cap: the "Check for Updates" menu entry, the startup/periodic loop,
//! the skip action, and landing a fetched release (state + dialog or toast +
//! activity entry). Manual checks report in a dialog; the automatic daily
//! check stays a toast so it never interrupts.

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::component::notification::{Notification, NotificationDelivery};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::v_flex;
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

    /// Land a fetch result: update the About state, then report — a manual
    /// check always opens the result dialog; the automatic loop toasts a new
    /// release and stays quiet otherwise.
    fn land_update(&mut self, result: Result<Option<Release>, String>, manual: bool, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = match &result {
            Ok(release) => {
                let outcome = update::apply_release(&mut self.update, release.as_ref());
                match (outcome, release) {
                    (ReleaseOutcome::Available, Some(release)) if manual => UpdateDialog::Available(release.clone(), false),
                    (ReleaseOutcome::Skipped, Some(release)) if manual => UpdateDialog::Available(release.clone(), true),
                    (ReleaseOutcome::Available, Some(release)) => {
                        Self::notify_update(release, window, cx);
                        return cx.notify();
                    },
                    (ReleaseOutcome::Skipped, _) => return cx.notify(),
                    (_, None) if manual => UpdateDialog::UpToDate { no_releases: true },
                    _ if manual => UpdateDialog::UpToDate { no_releases: false },
                    _ => return cx.notify(),
                }
            },
            Err(e) => {
                self.update.status = UpdateStatus::Unknown;
                update::persist_update(|s| s.update_last_check = Some(std::time::SystemTime::now()));
                if !manual {
                    return cx.notify();
                }
                UpdateDialog::Failed(e.clone())
            },
        };
        window.open_dialog(cx, update_dialog(dialog));
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

/// The "Check for Updates" menu item: run a manual check in the active
/// window's workspace — or open a window first when none is open, the same
/// fallback `show_about` uses. Deferred because the dispatching window is
/// mid-update.
pub fn check_for_updates(cx: &mut App) {
    cx.defer(|cx| {
        if let Some(handle) = cx.active_window() {
            check_in_window(handle, cx);
            return;
        }
        cx.spawn(async move |cx| {
            let Ok(handle) = crate::lifecycle::open_workspace_window(cx) else { return };
            check_in_window(*handle, cx);
        })
        .detach();
    });
}

/// Run a manual check in `handle`'s workspace — a no-op for a window whose
/// root isn't a `Workspace` (there are none today, but the downcast keeps
/// the menu item safe if that changes).
fn check_in_window<C: AppContext>(handle: AnyWindowHandle, cx: &mut C) {
    let _ = handle.update(cx, |view, _window, cx| {
        // The window's root view is `Root`; the workspace sits inside it.
        let Ok(root) = view.downcast::<gpui_kit::component::Root>() else { return };
        let Some(ws) = root.read(cx).view().clone().downcast::<Workspace>().ok() else { return };
        ws.update(cx, |ws, cx| ws.check_updates(cx));
    });
}

/// What the manual check's result dialog shows.
enum UpdateDialog {
    /// Nothing newer — `no_releases` means the repo has no releases at all
    /// (the `latest` endpoint 404s), which the copy notes.
    UpToDate { no_releases: bool },
    /// A newer release — name, notes excerpt, and a View Release button.
    /// `skipped` marks a release the user already dismissed.
    Available(Release, bool),
    /// The fetch or parse failed — the error renders as a muted line.
    Failed(String),
}

/// The result dialog's content builder for `window.open_dialog` — up-to-date
/// and failure get a single Dismiss button; a newer release adds View
/// Release, which opens the release page and closes.
fn update_dialog(result: UpdateDialog) -> impl Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static {
    move |dialog, _window, cx| {
        let mut body = v_flex().id("update-dialog").test_support().gap_2().text_sm();
        let mut footer = DialogFooter::new().child(
            Button::new("update-dialog-dismiss")
                .label("Dismiss")
                .outline()
                .on_click(|_, window, cx| window.close_dialog(cx)),
        );
        let (title, label) = match &result {
            UpdateDialog::UpToDate { no_releases } => {
                let detail = if *no_releases {
                    "The repository has no published releases yet."
                } else {
                    "No newer release is available."
                };
                body = body
                    .child(format!("You're on the latest version (v{}).", env!("CARGO_PKG_VERSION")))
                    .child(div().text_color(cx.theme().muted_foreground).child(detail));
                ("Check for Updates", format!("You're on the latest version (v{}). {detail}", env!("CARGO_PKG_VERSION")))
            },
            UpdateDialog::Available(release, skipped) => {
                body = body.child(format!("You're on v{}.", env!("CARGO_PKG_VERSION")));
                if let Some(name) = &release.name {
                    body = body.child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()));
                }
                if let Some(notes) = &release.notes {
                    body = body.child(div().text_xs().text_color(cx.theme().muted_foreground).child(update::release_notes_excerpt(notes)));
                }
                if *skipped {
                    body = body.child(div().text_xs().text_color(cx.theme().muted_foreground).child("You skipped this release."));
                }
                let url = release.url.clone();
                footer = footer.child(
                    Button::new("update-dialog-view-release")
                        .label("View Release")
                        .primary()
                        .icon(IconName::ExternalLink)
                        .on_click(move |_, window, cx| {
                            cx.open_url(&url);
                            window.close_dialog(cx);
                        }),
                );
                ("Update Available", format!("{} available{}", release.tag, if *skipped { " — skipped" } else { "" }))
            },
            UpdateDialog::Failed(e) => {
                body = body
                    .child("Couldn't check for updates.")
                    .child(div().text_color(cx.theme().muted_foreground).child(e.clone()));
                ("Check for Updates", format!("Couldn't check for updates. {e}"))
            },
        };
        dialog.title(title).overlay_closable(true).w(px(440.)).child(body.aria_label(label)).footer(footer)
    }
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

// Declared here, not in `main.rs` — that file is at the SLOC cap.
#[cfg(test)]
#[path = "update_dialog_tests.rs"]
mod update_dialog_tests;
