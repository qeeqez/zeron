//! The Project settings section: the per-project setup script — a shell
//! command run (`sh -c`) inside each new thread worktree, the desktop
//! counterpart of Codex cloud's environment setup. Persisted to the
//! project's `state.json` (`ProjectState.setup_script`); see
//! `crate::setup_script` for the runner. Below it, the live per-thread
//! worktrees under `.worktrees/` with reveal/delete affordances.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::settings_sections::{SettingsView, group_label};
use crate::worktree::WorktreeInfo;

/// The Project content pane: the setup-script field with its dirty/saved
/// state, the approval allowlist, then the live thread-worktree list.
pub(crate) fn project_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(setup_script_block(s, cx))
        .child(rules_block(s, cx))
        .child(worktrees_block(s, cx))
}

/// The "Always allow" rules block — one row per durable grant recorded by
/// an approval card's Always-allow button, with per-rule Delete. Rules
/// live in the project's `state.json`; deleting one makes matching
/// requests prompt again.
fn rules_block(s: &SettingsView, cx: &App) -> impl IntoElement {
    let rules = s.ws.read(cx).approval_rules.clone();
    let mut list = div().id("approval-rule-list").test_support().flex().flex_col().gap_1();
    if rules.is_empty() {
        list = list.child(
            div()
                .id("approval-rule-empty")
                .test_support()
                .p_3()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No rules — an approval card's Always allow adds one"),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(group_label("Always allow", &s.search, cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Approval requests matching a rule auto-approve instead of prompting. Rules are stored per project."),
        )
        .child(list.children(rules.iter().enumerate().map(|(ix, rule)| rule_row(ix, rule, s, cx))))
}

/// One rule row: the gated action's kind and detail, then Delete.
fn rule_row(ix: usize, rule: &crate::backend::ApprovalRule, s: &SettingsView, cx: &App) -> impl IntoElement {
    let ws = s.ws.clone();
    div()
        .id(("approval-rule", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .p_2()
        .rounded_md()
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_xs().font_semibold().overflow_hidden().child(rule.kind.label()))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(rule.detail.clone()),
                ),
        )
        .child(div().flex_1())
        .child(
            Button::new(SharedString::from(format!("approval-rule-delete-{ix}")))
                .label("Delete")
                .icon(IconName::Trash)
                .small()
                .danger()
                .on_click(move |_, _, cx| {
                    ws.update(cx, |this, cx| this.remove_approval_rule(ix, cx));
                }),
        )
}

/// The setup-script field: a note on when it runs, the textarea, and the
/// Save button with its dirty/saved status.
fn setup_script_block(s: &SettingsView, cx: &App) -> impl IntoElement {
    let (saved, draft) = {
        let ws = s.ws.read(cx);
        (ws.setup_script.clone(), ws.setup_script_input.read(cx).value().to_string())
    };
    let dirty = draft.trim() != saved.trim();
    let ws = s.ws.clone();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Setup script", &s.search, cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Runs inside each new thread worktree (sh -c) — install dependencies, symlink env files. Failures land as a note; they never block the thread."),
        )
        .child(s.search.wrap(
            "Setup script",
            div()
                .id("setup-script-field")
                .test_support()
                .child(Textarea::new(&s.setup_script_input).aria_label("Setup script")),
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("setup-script-save")
                        .label("Save")
                        .small()
                        .primary()
                        .disabled(!dirty)
                        .on_click(move |_, _, cx| {
                            ws.update(cx, |this, cx| this.save_setup_script(cx));
                        }),
                )
                .child(
                    div()
                        .id("setup-script-status")
                        .test_support()
                        .aria_label(if dirty { "Unsaved changes" } else { "Saved" })
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if dirty { "Unsaved changes" } else { "Saved" }),
                ),
        )
}

/// The "Thread worktrees" block — one row per live dir under
/// `.worktrees/`, rebuilt every render so opening settings always shows
/// the current set. Orphans (no chat owns the dir) get a Delete button;
/// rows for live threads don't — removing one would break the thread.
fn worktrees_block(s: &SettingsView, cx: &App) -> impl IntoElement {
    let worktrees = {
        let ws = s.ws.read(cx);
        crate::worktree::list_live(ws.project.root(), &ws.chats)
    };
    let mut list = div().id("worktree-list").test_support().flex().flex_col().gap_1();
    if worktrees.is_empty() {
        list = list.child(
            div()
                .id("worktree-empty")
                .test_support()
                .p_3()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No worktrees"),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(group_label("Thread worktrees", &s.search, cx))
        .child(list.children(worktrees.iter().map(|w| worktree_row(w, s, cx))))
}

/// One worktree row: the dir name, the owning thread's title (or
/// "orphan"), the full path, then Reveal — and Delete for orphans only.
fn worktree_row(info: &WorktreeInfo, s: &SettingsView, cx: &App) -> impl IntoElement {
    let name = info
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| info.path.display().to_string());
    let owner = info.chat_title.clone().unwrap_or_else(|| "orphan".to_string());
    let path = info.path.display().to_string();
    let (ws_reveal, ws_delete) = (s.ws.clone(), s.ws.clone());
    let (path_reveal, path_delete) = (info.path.clone(), info.path.clone());
    div()
        .id(SharedString::from(format!("worktree-row-{name}")))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .p_2()
        .rounded_md()
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_xs().font_semibold().overflow_hidden().child(name.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(format!("{owner} · {path}")),
                ),
        )
        .child(div().flex_1())
        .child(
            Button::new(SharedString::from(format!("worktree-reveal-{name}")))
                .label("Reveal")
                .icon(IconName::FolderOpen)
                .small()
                .outline()
                .on_click(move |_, _, cx| {
                    ws_reveal.update(cx, |this, cx| this.reveal_path_in_finder(&path_reveal, cx));
                }),
        )
        .when(info.chat_title.is_none(), |d| {
            d.child(
                Button::new(SharedString::from(format!("worktree-delete-{name}")))
                    .label("Delete")
                    .icon(IconName::Trash)
                    .small()
                    .danger()
                    .on_click(move |_, window, cx| {
                        ws_delete.update(cx, |this, cx| this.remove_orphan_worktree(path_delete.clone(), window, cx));
                    }),
            )
        })
}
