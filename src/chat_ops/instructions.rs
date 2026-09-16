//! Per-chat custom instructions — a thread-level system-prompt override
//! appended after the global + project merge (see
//! `crate::instructions::for_chat_turn`). The ⋯ menu's "Custom
//! instructions…" opens a small dialog whose textarea shares
//! `Workspace::chat_instructions_input` like the rename dialog shares
//! `Workspace::rename`.

use gpui_kit::component::WindowExt;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// Set the chat's custom instructions; empty clears the override.
    pub fn set_chat_instructions(&mut self, id: u64, text: &str, cx: &mut Context<Self>) {
        let text = text.trim();
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == id) else { return };
        // Saving an unchanged value would only churn the chat file.
        if chat.instructions.as_deref() == Some(text) || (text.is_empty() && chat.instructions.is_none()) {
            return;
        }
        chat.instructions = if text.is_empty() { None } else { Some(text.to_string()) };
        cx.notify();
        self.save();
    }

    /// "Custom instructions…" from the ⋯ menu — a small dialog seeded with
    /// the chat's current text; OK saves, empty input clears the override.
    pub fn open_chat_instructions(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter().find(|c| c.id == id) else { return };
        let current = chat.instructions.clone().unwrap_or_default();
        self.chat_instructions_input.update(cx, |state, cx| state.set_value(current, window, cx));
        let ws = cx.entity();
        let input = self.chat_instructions_input.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ws_ok = ws.clone();
            dialog
                .title("Custom instructions")
                .overlay_closable(true)
                .child(Textarea::new(&input).aria_label("Chat instructions"))
                .on_ok(move |_, _window, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_chat_instructions(id, cx));
                    true
                })
        });
        self.chat_instructions_input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Dialog OK for "Custom instructions" — empty input clears the chat's
    /// override.
    fn commit_chat_instructions(&mut self, id: u64, cx: &mut Context<Self>) {
        let text = self.chat_instructions_input.read(cx).value().to_string();
        self.set_chat_instructions(id, &text, cx);
    }
}

/// The titlebar chip for a chat with custom instructions — same muted
/// styling as the worktree badge; a chat can carry both. `id` keeps it
/// findable in headless tests.
pub fn instructions_badge(id: &'static str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .flex()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_color(cx.theme().muted_foreground)
        .child(assets::IconName::NotebookPen)
        .tooltip(|window, cx| Tooltip::new("Custom instructions").build(window, cx))
        .into_any_element()
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "../chat_instructions_tests.rs"]
mod chat_instructions_tests;
