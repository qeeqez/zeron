//! App lifecycle: background tasks started at workspace creation (the 1s
//! elapsed-time ticker and the project-file scan for the @-mention picker)
//! plus the app-level actions — quit gating, new window, About, dock reopen.

use std::time::Duration;

use gpui_kit::component::{Root, WindowExt};
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// Spawn the ticker and the file scan. Called once from `Workspace::new`.
    pub(crate) fn start_background(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let _ = this.update(cx, Self::tick);
            }
        })
        .detach();
        // Scan project files off the UI thread — a large tree would block
        // launch; the @-mention picker just stays empty until it lands.
        let root = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let files = cx.background_executor().spawn(async move { crate::files::scan_project_files(&root) }).await;
            let _ = this.update(cx, |this, cx| {
                this.project_files = files;
                cx.notify();
            });
        })
        .detach();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let mut dirty = self.chats.iter().any(|c| c.running);
        for agent in &mut self.agents {
            if agent.status == crate::model::AgentStatus::Running {
                agent.elapsed_secs += 1;
                dirty = true;
            }
        }
        if dirty {
            cx.notify();
        }
    }
}

/// Cmd+Q / Quit menu item. Mirrors the window close gate: if any workspace
/// window has a reply still generating, prompt in that window and only quit
/// on confirm. `on_app_quit` can't cancel a quit, so the gate lives here —
/// before `cx.quit()` — and drafts are still saved by the quit observer.
pub fn request_quit(cx: &mut App) {
    // A window is briefly absent from `cx.windows()` while its own action
    // dispatch runs — defer so the scan sees every open window.
    cx.defer(|cx| {
        let Some(handle) = running_workspace_window(cx) else {
            cx.quit();
            return;
        };
        let rx = handle
            .update(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Warning,
                    "A reply is still generating",
                    Some("Quitting now will stop it."),
                    &[PromptButton::ok("Quit"), PromptButton::cancel("Cancel")],
                    cx,
                )
            })
            .unwrap_or_else(|_| unreachable!("window came from cx.windows()"));
        cx.spawn(async move |cx| {
            if rx.await == Ok(0) {
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
    });
}

/// The window whose workspace has a running reply, if any. Iterates every
/// open window — not just the active one — so Cmd+Q can't silently kill a
/// turn running in a background window.
fn running_workspace_window(cx: &mut App) -> Option<AnyWindowHandle> {
    cx.windows().into_iter().find(|handle| {
        handle
            .update(cx, |root, _, cx| {
                root.downcast::<Root>()
                    .ok()
                    .and_then(|root| root.read(cx).view().clone().downcast::<Workspace>().ok())
                    .is_some_and(|ws| ws.read(cx).chats.iter().any(|c| c.running))
            })
            .unwrap_or(false)
    })
}

/// Open a fresh workspace window — File > New Window, the dock menu, and
/// dock-icon reopen all land here.
pub fn open_new_window(cx: &mut App) {
    cx.spawn(async move |cx| {
        let _ = crate::root::open_workspace_window(cx);
    })
    .detach();
}

/// The app's About panel, shown in the active window. With no windows open
/// (menu click after the last window closed), open a workspace first so the
/// dialog has somewhere to live. Deferred for the same reason as
/// `request_quit` — the dispatching window is mid-update.
pub fn show_about(cx: &mut App) {
    cx.defer(|cx| {
        if let Some(handle) = cx.active_window() {
            let _ = handle.update(cx, |_, window, cx| show_about_dialog(window, cx));
            return;
        }
        cx.spawn(async move |cx| {
            let Ok(handle) = crate::root::open_workspace_window(cx) else { return };
            let _ = handle.update(cx, |_, window, cx| show_about_dialog(window, cx));
        })
        .detach();
    });
}

fn show_about_dialog(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _window, _cx| {
        dialog.title("Rixl Code").overlay_closable(true).child(
            div()
                .id("about-dialog")
                .test_support()
                .flex()
                .flex_col()
                .gap_2()
                .child(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .child("A Codex-style agent workspace."),
        )
    });
}
