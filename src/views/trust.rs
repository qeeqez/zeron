//! The workspace-trust surfaces: the first-open dialog ("Do you trust the
//! files in this folder?") and the banner an untrusted workspace shows until
//! the folder is trusted. Trusting persists the folder (see `crate::trust`);
//! dismissing or "Don't trust" leaves the workspace restricted — the banner
//! stays up as the way to trust later.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Icon, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// The trust dialog's content builder, shared by `open_trust_dialog` (the
/// banner's Trust… button) and `open_workspace_window_for`'s post-open gate
/// — the latter already holds the `Root` borrow, so it must feed this to
/// `Root::open_dialog` directly rather than re-enter via `window.open_dialog`.
pub(crate) fn trust_dialog(root: PathBuf, ws: Entity<Workspace>) -> impl Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static {
    move |dialog, _window, cx| {
        let ws_trust = ws.clone();
        dialog
            .title("Do you trust the files in this folder?")
            .overlay_closable(true)
            .w(px(440.))
            .child(
                v_flex()
                    .id("trust-dialog")
                    .test_support()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Icon::new(IconName::FolderOpen).size_4().text_color(cx.theme().muted_foreground))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_sm()
                                    .child(root.display().to_string()),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Rixl Code can read, write and run commands in trusted folders. If you don't trust this folder, it opens in restricted mode: the agent is read-only and every action asks for approval."),
                    ),
            )
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("dont-trust-folder")
                            .label("Don't Trust")
                            .outline()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("trust-folder")
                            .label("Trust")
                            .primary()
                            .on_click(move |_, window, cx| {
                                ws_trust.update(cx, |this, cx| this.trust_project(cx));
                                window.close_dialog(cx);
                            }),
                    ),
            )
    }
}

impl Workspace {
    /// The first-open trust prompt. "Trust" persists the folder and lifts
    /// the restriction; "Don't trust" (or Esc/overlay close) keeps the
    /// workspace in restricted mode — the banner offers the way back.
    pub fn open_trust_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.trusted {
            return;
        }
        let ws = cx.entity();
        window.open_dialog(cx, trust_dialog(self.project.root().to_path_buf(), ws));
    }
}

/// The restricted-mode banner above the transcript — the persistent notice
/// that the folder isn't trusted plus the "Trust…" affordance that reopens
/// the dialog. Rendered only while `!workspace.trusted`.
pub(crate) fn restricted_banner(cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("trust-banner")
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .py_1p5()
        .border_b_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().warning)
        .child(IconName::ShieldAlert)
        .child(
            div()
                .flex_1()
                .child("Restricted mode — this folder isn't trusted. The agent is read-only and asks before every action."),
        )
        .child(
            Button::new("trust-banner-open")
                .label("Trust…")
                .small()
                .outline()
                .on_click(cx.listener(|this, _, window, cx| this.open_trust_dialog(window, cx))),
        )
}
