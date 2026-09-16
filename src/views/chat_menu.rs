//! The chat header's ⋯ menu and worktree badge — split from `chat_view.rs`
//! for the SLOC cap.

mod bookmarks;
mod color;
mod continue_with;
use bookmarks::bookmarks_submenu;
use color::color_submenu;
use continue_with::continue_with_submenu;

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

use crate::workspace::Workspace;

/// Per-render state the ⋯ menu needs — bundled to stay under the arg-count
/// lint. `worktree` gates the worktree-only items.
#[derive(Clone, Copy)]
pub struct ChatMenuState {
    pub pinned: bool,
    pub word_wrap: bool,
    /// The chat's color tag (`Chat.color`) — the Color submenu checks it.
    pub color: Option<crate::model::ChatColor>,
    /// The chat runs in a per-thread git worktree (`Chat.worktree`).
    pub worktree: bool,
    /// Temporary chat (`Chat.ephemeral`) — disables the items that need a
    /// persisted chat (export, open-in-new-window).
    pub ephemeral: bool,
    /// The chat has 2+ messages and no reply running — gates "Split chat…".
    pub can_split: bool,
}

/// The color-tag dot — one shape for the sidebar row, the titlebar and the
/// ⋯ menu's swatches. `id` keeps it findable in headless tests.
pub fn color_dot(id: impl Into<ElementId>, color: crate::model::ChatColor, size: Pixels) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .w(size)
        .h(size)
        .rounded_full()
        .flex_shrink_0()
        .bg(color.hsla())
        .into_any_element()
}

/// The worktree chip on the chat titlebar — a muted icon + "worktree" label
/// whose tooltip carries the checkout path. `id` keeps it findable in
/// headless tests.
pub fn worktree_badge(id: &'static str, workdir: &str, cx: &App) -> AnyElement {
    let tip = workdir.to_string();
    div()
        .id(id)
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(IconName::FolderGit)
        .child("worktree")
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .into_any_element()
}

/// The "Temporary" chip on the chat titlebar — same muted styling as the
/// worktree badge; a chat can carry both. `id` keeps it findable in
/// headless tests.
pub fn temp_badge(id: &'static str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(IconName::Ghost)
        .child("Temporary")
        .tooltip(|window, cx| Tooltip::new("Not saved — closes with the chat").build(window, cx))
        .into_any_element()
}

/// The context-window meter on the chat titlebar — `NN%` of the window
/// used (Codex-style; the tooltip carries the exact counts), muted under
/// 80%, warning at 80%+, danger at 95%+. Token backends report no window
/// size, so the chip falls back to the chat's cumulative token count and
/// stays muted. `None` until the first usage report — a fresh chat or a
/// backend without usage (sim) shows nothing.
pub fn context_chip(id: &'static str, usage: &crate::usage::ChatUsage, cx: &App) -> Option<AnyElement> {
    use crate::usage::{ContextMeter, MeterTier};
    let meter = usage.meter()?;
    let color = match meter {
        ContextMeter::Fill { tier: MeterTier::Warning, .. } => cx.theme().warning,
        ContextMeter::Fill { tier: MeterTier::Danger, .. } => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    };
    let tip = meter.detail();
    Some(
        div()
            .id(id)
            .test_support()
            .aria_label(tip.clone())
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded_md()
            .text_xs()
            .text_color(color)
            .child(IconName::CircleGauge)
            .child(meter.label())
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .into_any_element(),
    )
}

/// Pin/rename/export/copy/snapshots/word-wrap — the ⋯ menu on the chat
/// titlebar. Worktree chats also get reveal/open items for their checkout.
pub fn chat_menu(
    menu: PopupMenu, ws: &Entity<Workspace>, state: ChatMenuState, window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let ChatMenuState { pinned, word_wrap, color, worktree, ephemeral, can_split } = state;
    let ws_pin = ws.clone();
    let ws_rename = ws.clone();
    let ws_export = ws.clone();
    let ws_copy = ws.clone();
    let ws_wrap = ws.clone();
    let ws_snap = ws.clone();
    let ws_fork = ws.clone();
    let ws_window = ws.clone();
    let ws_temp = ws.clone();
    let ws_info = ws.clone();
    let ws_budget = ws.clone();
    let menu = menu
        .item(PopupMenuItem::new("New Temporary Chat").icon(IconName::Ghost).on_click(move |_, _, cx| {
            ws_temp.update(cx, |this, cx| this.new_temp_chat(cx));
        }))
        .item(
            PopupMenuItem::new(if pinned { "Unpin" } else { "Pin" })
                .icon(IconName::Star)
                .on_click(move |_, _, cx| {
                    ws_pin.update(cx, |this, cx| this.toggle_pin(this.active, cx));
                }),
        )
        .item(PopupMenuItem::new("Rename").icon(IconName::Pencil).on_click(move |_, window, cx| {
            ws_rename.update(cx, |this, cx| this.rename_active(window, cx));
        }))
        .submenu("Color", window, cx, {
            let ws = ws.clone();
            move |m, _w, _cx| color_submenu(&ws, color, m)
        })
        .item(PopupMenuItem::new("Custom instructions…").icon(IconName::NotebookPen).on_click({
            let ws = ws.clone();
            move |_, window, cx| {
                ws.update(cx, |this, cx| {
                    let id = this.chats[this.active].id;
                    this.open_chat_instructions(id, window, cx);
                });
            }
        }))
        .item(PopupMenuItem::new("Budget alert…").icon(IconName::CircleDollarSign).on_click(move |_, window, cx| {
            ws_budget.update(cx, |this, cx| {
                let id = this.chats[this.active].id;
                this.open_chat_budget(id, window, cx);
            });
        }))
        .item(PopupMenuItem::new("Export").icon(IconName::Share).disabled(ephemeral).on_click(move |_, _, cx| {
            ws_export.update(cx, |this, cx| this.export_active(cx));
        }))
        .item(PopupMenuItem::new("Copy transcript").icon(IconName::Copy).on_click(move |_, _, cx| {
            ws_copy.update(cx, |this, cx| this.copy_transcript(cx));
        }))
        .item(PopupMenuItem::new("Fork chat").icon(IconName::GitFork).on_click(move |_, window, cx| {
            ws_fork.update(cx, |this, cx| {
                let ix = this.active;
                this.fork_chat(ix, None, window, cx);
            });
        }))
        .item(PopupMenuItem::new("Split chat…").icon(IconName::Scissors).disabled(!can_split).on_click({
            let ws = ws.clone();
            move |_, window, cx| {
                ws.update(cx, |this, cx| {
                    let id = this.chats[this.active].id;
                    this.open_split_dialog(id, window, cx);
                });
            }
        }))
        .item(
            PopupMenuItem::new("Open in New Window")
                .icon(IconName::WindowRestore)
                .disabled(ephemeral)
                .on_click(move |_, _, cx| {
                    ws_window.update(cx, |this, cx| {
                        let id = this.chats[this.active].id;
                        this.open_chat_in_new_window(id, cx);
                    });
                }),
        );
    // "Continue with" — fork the transcript onto another provider. After
    // the chat-shape items, before the conditional tail.
    let menu = continue_with_submenu(menu, ws, window, cx);
    // "Copy resume command" only exists when the chat is bound to a backend
    // thread AND the backend has a CLI resume (codex/claude).
    let menu = if ws.read(cx).resume_command().is_some() {
        let ws_resume = ws.clone();
        menu.item(PopupMenuItem::new("Copy resume command").icon(IconName::Terminal).on_click(move |_, _, cx| {
            ws_resume.update(cx, |this, cx| this.copy_resume_command(cx));
        }))
    } else {
        menu
    };
    let menu = bookmarks_submenu(menu, ws, window, cx);
    let menu = if worktree { worktree_items(menu, ws, window, cx) } else { menu };
    menu.item(PopupMenuItem::new("Snapshots").icon(IconName::Camera).on_click(move |_, _, cx| {
        ws_snap.update(cx, |this, cx| this.toggle_snapshots_panel(cx));
    }))
    .item(PopupMenuItem::new("Word wrap").icon(IconName::Check).checked(word_wrap).on_click(move |_, _, cx| {
        ws_wrap.update(cx, |this, cx| {
            this.word_wrap = !this.word_wrap;
            this.save_settings();
            // Every message's height changed — remeasure the whole list.
            this.scroller.update(cx, |s, cx| s.remeasure(cx));
            cx.notify();
        });
    }))
    .item(PopupMenuItem::new("Chat info").icon(IconName::Info).on_click(move |_, window, cx| {
        ws_info.update(cx, |this, cx| this.open_chat_info(window, cx));
    }))
}

/// The worktree-only section of the ⋯ menu: reveal the checkout in Finder
/// and open it in the preferred editor (`Ask` expands to a picker, same as
/// the file menu). Paths resolve at click time so a deleted worktree falls
/// back to the project root via `workdir_for`.
fn worktree_items(menu: PopupMenu, ws: &Entity<Workspace>, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let preferred = ws.read(cx).preferred_editor;
    let ws_reveal = ws.clone();
    let menu = menu.item(
        PopupMenuItem::new("Reveal Worktree in Finder")
            .icon(IconName::FolderOpen)
            .on_click(move |_, _w, cx| {
                ws_reveal.update(cx, |this, cx| {
                    let dir = worktree_dir(this);
                    this.reveal_path_in_finder(&dir, cx);
                });
            }),
    );
    if preferred == crate::open_in::PreferredEditor::Ask {
        let ws_pick = ws.clone();
        menu.submenu("Open Worktree in Editor", window, cx, move |m, _w, _cx| {
            crate::open_in::PreferredEditor::CHOICES
                .into_iter()
                .fold(m, |m, editor| m.item(worktree_pick_item(&ws_pick, editor)))
        })
    } else {
        let ws_open = ws.clone();
        menu.item(
            PopupMenuItem::new(format!("Open Worktree in {}", preferred.label()))
                .icon(IconName::ExternalLink)
                .on_click(move |_, _w, cx| {
                    ws_open.update(cx, |this, cx| {
                        let dir = worktree_dir(this);
                        this.open_path_in_editor(&dir, None, cx);
                    });
                }),
        )
    }
}

/// One "Open in <editor>" pick for the `Ask` submenu — a free fn so the
/// submenu fold stays under the nesting lint.
fn worktree_pick_item(ws: &Entity<Workspace>, editor: crate::open_in::PreferredEditor) -> PopupMenuItem {
    let ws = ws.clone();
    PopupMenuItem::new(editor.label()).on_click(move |_, _w, cx| {
        ws.update(cx, |this, cx| {
            let dir = worktree_dir(this);
            this.open_path_in_editor(&dir, Some(editor), cx);
        });
    })
}

/// The active chat's worktree directory — `workdir_for` falls back to the
/// project root when the checkout is gone, so the items never target a
/// missing path.
fn worktree_dir(this: &Workspace) -> std::path::PathBuf {
    crate::worktree::workdir_for(&this.chats[this.active], this.project.root())
}
