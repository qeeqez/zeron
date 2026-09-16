//! App lifecycle: background tasks started at workspace creation (the 1s
//! elapsed-time ticker and the project-file scan for the @-mention picker)
//! plus the app-level actions — quit gating, new window, About, dock reopen.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Root, Sizable, WindowExt};
use gpui_kit::*;

use crate::update::UpdateStatus;
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
    cx.windows().into_iter().find(|handle| window_has_running_turn(*handle, cx))
}

/// True while any open workspace window owns a live turn. `Workspace::new`
/// consults this before restoring chats: a `Running` tool on disk belongs to
/// that live turn, so only a cold start (no running turn anywhere) may
/// recover it as failed. The window under construction isn't in
/// `cx.windows()` yet, so it can't false-positive on itself.
pub(crate) fn any_turn_running(cx: &mut App) -> bool {
    cx.windows().into_iter().any(|handle| window_has_running_turn(handle, cx))
}

fn window_has_running_turn(handle: AnyWindowHandle, cx: &mut App) -> bool {
    handle
        .update(cx, |root, _, cx| {
            root.downcast::<Root>()
                .ok()
                .and_then(|root| root.read(cx).view().clone().downcast::<Workspace>().ok())
                .is_some_and(|ws| ws.read(cx).chats.iter().any(|c| c.running))
        })
        .unwrap_or(false)
}

/// Open a fresh workspace window — File > New Window, the dock menu, and
/// dock-icon reopen all land here.
pub fn open_new_window(cx: &mut App) {
    cx.spawn(async move |cx| {
        let _ = open_workspace_window(cx);
    })
    .detach();
}

/// File > Open Project…, the palette row, and the empty-state/switcher
/// buttons: the native folder picker, then `open_project` on the chosen
/// folder. A global action, so the menu item also works with no window open.
pub fn prompt_open_project(cx: &mut App) {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open Project".into()),
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = rx.await else { return };
        let Some(path) = paths.into_iter().next() else { return };
        cx.update(|cx| open_project(&path, cx));
    })
    .detach();
}

/// Open `path` as a project: focus its window when one is already bound to
/// it — two windows on one project would race the same chat files — else
/// open a fresh workspace window on it.
pub fn open_project(path: &std::path::Path, cx: &mut App) {
    let project = crate::project::Project::open(path);
    crate::recent_projects::record(project.root());
    // Deferred: callers are click handlers inside a window update, and a
    // window mid-dispatch can't be re-entered — the scan would miss the
    // dispatching window itself (same reason `request_quit` defers).
    cx.defer(move |cx| {
        if let Some(handle) = project_window(project.root(), cx) {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return;
        }
        cx.spawn(async move |cx| {
            let _ = open_workspace_window_for(project, cx);
        })
        .detach();
    });
}

/// Open a workspace window on the launch project (used at launch, File > New
/// Window, and dock reopen). Returns the handle so callers can follow up —
/// e.g. About opens its dialog in the window it just created.
pub fn open_workspace_window(cx: &mut gpui_kit::AsyncApp) -> gpui_kit::Result<gpui_kit::WindowHandle<Root>> {
    open_workspace_window_for(crate::project::Project::launch(), cx)
}

/// Spawn the launch window — the first workspace window, opened async so
/// `main`'s run callback returns before window setup runs.
pub fn spawn_launch_window(cx: &mut App) {
    cx.spawn(async move |cx| {
        open_workspace_window(cx).expect("failed to open window");
    })
    .detach();
}

/// Open a workspace window bound to `project` — one window per project, so
/// each keeps its own chats and cwd-scoped work.
pub fn open_workspace_window_for(
    project: crate::project::Project, cx: &mut gpui_kit::AsyncApp,
) -> gpui_kit::Result<gpui_kit::WindowHandle<Root>> {
    crate::recent_projects::record(project.root());
    let trusted = crate::trust::is_trusted(project.root());
    let root_path = project.root().to_path_buf();
    let handle = cx.open_window(
        WindowOptions {
            window_min_size: Some(Size { width: px(800.), height: px(600.) }),
            window_bounds: crate::window::saved_window_bounds(),
            window_background: crate::appearance::window_background_appearance(crate::persist::load_settings().sidebar_frosted),
            // The app draws its own TitleBar and moves the window via
            // start_window_move, so AppKit must not treat the strip as a system
            // window-move region (which would swallow the toggle's clicks).
            app_owns_titlebar_drag: true,
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some("Rixl Code".into()),
                appears_transparent: true,
                traffic_light_position: Some(gpui_kit::point(px(9.), px(9.))),
            }),
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|cx| Workspace::for_project(project.clone(), window, cx));
            let ws = view.clone();
            let frosted = ws.read(cx).sidebar_frosted;
            let handle = window.window_handle();
            window.on_window_should_close(cx, move |window, cx| crate::window::confirm_close(&ws, handle, window, cx));
            cx.new(|cx| {
                let mut root = Root::new(view, window, cx);
                // Frosted sidebar needs the window's blurred background to
                // show through — Root's opaque theme fill would hide it.
                root.style().background = crate::appearance::frosted_root_background(frosted);
                root
            })
        },
    )?;
    // Workspace trust: a folder not in the trusted list opens restricted —
    // `restrict_untrusted` forces read-only/ask access — behind the trust
    // dialog. "Trust" persists the folder; "Don't trust" (or dismissing the
    // dialog) leaves the restriction in place until the banner's Trust…
    // button reopens the prompt.
    if !trusted {
        let _ = handle.update(cx, |root, window, cx| {
            if let Ok(ws) = root.view().clone().downcast::<Workspace>() {
                ws.update(cx, |this, cx| this.restrict_untrusted(cx));
                // `window.open_dialog` would re-borrow Root — we're already
                // inside its update, so feed the builder to it directly.
                root.open_dialog(crate::views::trust::trust_dialog(root_path, ws), window, cx);
            }
        });
    }
    Ok(handle)
}

/// The open workspace window bound to `root`, if any.
fn project_window(root: &std::path::Path, cx: &mut App) -> Option<AnyWindowHandle> {
    cx.windows().into_iter().find(|handle| {
        handle
            .update(cx, |view, _, cx| {
                view.downcast::<Root>()
                    .ok()
                    .and_then(|root| root.read(cx).view().clone().downcast::<Workspace>().ok())
                    .is_some_and(|ws| ws.read(cx).project.root() == root)
            })
            .unwrap_or(false)
    })
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
            let Ok(handle) = open_workspace_window(cx) else { return };
            let _ = handle.update(cx, |_, window, cx| show_about_dialog(window, cx));
        })
        .detach();
    });
}

fn show_about_dialog(window: &mut Window, cx: &mut App) {
    let update = window_workspace(window, cx).map(|ws| ws.read(cx).update.clone());
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let mut body = div()
            .id("about-dialog")
            .test_support()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(crate::app_icon::app_icon("about-app-icon", 64.))
            .child(format!("Version {}", env!("CARGO_PKG_VERSION")))
            .child("A Codex-style agent workspace.");
        if let Some(update) = &update {
            body = body.child(about_update_row(update));
        }
        dialog.title("Rixl Code").overlay_closable(true).child(body)
    });
}

/// The `Workspace` entity behind a window's `Root` view, if it has one.
fn window_workspace(window: &Window, cx: &App) -> Option<Entity<Workspace>> {
    window.root::<Root>().flatten()?.read(cx).view().clone().downcast::<Workspace>().ok()
}

/// The About dialog's update line: a pending release with a Download
/// button, or nothing while checking/up-to-date/unknown — the version line
/// already covers those.
fn about_update_row(update: &crate::update::UpdateState) -> impl IntoElement {
    let mut row = div().flex().items_center().gap_2().text_sm();
    match &update.status {
        UpdateStatus::Available(tag) => {
            let url = update.url.clone();
            row = row.child(format!("{tag} available{}", if update.skipped { " — skipped" } else { "" })).child(
                Button::new("about-update-download")
                    .label("Download")
                    .icon(IconName::Download)
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        },
        UpdateStatus::Checking => {
            row = row.child("Checking for updates…");
        },
        UpdateStatus::Unknown | UpdateStatus::UpToDate => {},
    }
    row
}
