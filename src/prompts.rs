//! Saved prompts — named, reusable composer text, persisted per project as
//! `<project>/prompts.json` (see `crate::persist::{save_prompts, load_prompts}`).
//!
//! `/save <name> [text]` stores a prompt (no text saves the chat's last user
//! message), `/prompts` lists them, and the composer's star popover loads one
//! back into the composer or manages the list (rename/delete/save-current).

use gpui_kit::component::WindowExt;
use gpui_kit::component::input::Input;
use gpui_kit::*;

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

/// One saved prompt: a name the user picks plus the text a pick loads into
/// the composer.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavedPrompt {
    pub name: String,
    pub text: String,
}

/// The project's saved prompts, in save order — the popover's display order.
#[derive(Debug, Default)]
pub struct PromptStore {
    pub prompts: Vec<SavedPrompt>,
}

impl PromptStore {
    /// Insert or overwrite the prompt named `name` (trimmed). Empty name or
    /// text saves nothing — callers surface that as a usage note.
    pub fn save(&mut self, name: &str, text: &str) -> bool {
        let name = name.trim();
        let text = text.trim();
        if name.is_empty() || text.is_empty() {
            return false;
        }
        if let Some(p) = self.prompts.iter_mut().find(|p| p.name == name) {
            p.text = text.to_string();
        } else {
            self.prompts.push(SavedPrompt { name: name.to_string(), text: text.to_string() });
        }
        true
    }

    pub fn get(&self, name: &str) -> Option<&SavedPrompt> {
        self.prompts.iter().find(|p| p.name == name)
    }

    /// Rename `old` to `new` (trimmed). Refuses empty names and collisions —
    /// a rename must not silently clobber another saved prompt.
    pub fn rename(&mut self, old: &str, new: &str) -> bool {
        let new = new.trim();
        if new.is_empty() || new == old || self.get(new).is_some() {
            return false;
        }
        let Some(p) = self.prompts.iter_mut().find(|p| p.name == old) else { return false };
        p.name = new.to_string();
        true
    }

    /// Drop the prompt named `name`; returns whether one existed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.prompts.len();
        self.prompts.retain(|p| p.name != name);
        self.prompts.len() != before
    }
}

impl Workspace {
    /// `/save <name> [text]` — store `text` under `name`. With no inline
    /// text the chat's last user message is saved, so a prompt worth keeping
    /// can be captured right after sending it.
    pub(crate) fn save_prompt_command(&mut self, arg: &str, cx: &mut Context<Self>) {
        let (name, text) = arg.split_once(char::is_whitespace).map_or((arg.trim(), ""), |(n, t)| (n.trim(), t.trim()));
        if name.is_empty() {
            self.push_note("Usage: `/save <name> [text]` — no text saves the last message.".into(), cx);
            return;
        }
        let text = if text.is_empty() { self.last_user_text() } else { text.to_string() };
        if text.is_empty() {
            self.push_note("Nothing to save — pass text (`/save <name> <text>`) or send a message first.".into(), cx);
            return;
        }
        let existed = self.prompts.get(name).is_some();
        if self.save_prompt(name, &text) {
            let verb = if existed { "Updated" } else { "Saved" };
            self.push_note(format!("{verb} prompt `{name}` — the composer's ★ menu can load it."), cx);
        }
    }

    /// `/prompts` — list the saved prompts (the popover is the interactive
    /// list; this is the transcript-visible one).
    pub(crate) fn prompts_note(&mut self, cx: &mut Context<Self>) {
        if self.prompts.prompts.is_empty() {
            self.push_note("No saved prompts — `/save <name> <text>` adds one.".into(), cx);
            return;
        }
        let list = self
            .prompts
            .prompts
            .iter()
            .map(|p| format!("- `{name}` — {preview}", name = p.name, preview = first_line(&p.text)))
            .collect::<Vec<_>>()
            .join("\n");
        self.push_note(format!("**Saved prompts:**\n{list}"), cx);
    }

    /// The active chat's most recent user message text — `/save <name>`'s
    /// fallback when no inline text is given.
    fn last_user_text(&self) -> String {
        self.chats[self.active]
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                if m.role == Role::User
                    && let MessageKind::Text(t) = &m.kind
                {
                    return Some(t.to_string());
                }
                None
            })
            .unwrap_or_default()
    }

    /// Store `text` under `name` and persist. Shared by `/save` and the
    /// popover's save dialog; returns false on an empty name or text.
    pub(crate) fn save_prompt(&mut self, name: &str, text: &str) -> bool {
        if !self.prompts.save(name, text) {
            return false;
        }
        crate::persist::save_prompts(self.project.dir(), &self.prompts);
        true
    }

    /// Load a saved prompt into the composer — the popover row's click.
    pub(crate) fn load_prompt(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.prompts.get(name).map(|p| p.text.clone()) else { return };
        self.composer.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// Rename a saved prompt and persist — the rename dialog's OK.
    pub(crate) fn rename_prompt(&mut self, old: &str, new: &str, cx: &mut Context<Self>) {
        if !self.prompts.rename(old, new) {
            return;
        }
        crate::persist::save_prompts(self.project.dir(), &self.prompts);
        cx.notify();
    }

    /// Delete a saved prompt and persist — the popover row's ✕.
    pub(crate) fn delete_prompt(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.prompts.remove(name) {
            return;
        }
        crate::persist::save_prompts(self.project.dir(), &self.prompts);
        cx.notify();
    }

    /// The popover's "Save current…" row — a small dialog naming the
    /// composer's current text. Shares `prompt_input` with the rename dialog.
    pub(crate) fn open_save_prompt_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.prompt_input.update(cx, |state, cx| state.set_value("", window, cx));
        let ws = cx.entity();
        let input = self.prompt_input.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ws_ok = ws.clone();
            let text = text.clone();
            dialog
                .title("Save prompt")
                .overlay_closable(true)
                .child(Input::new(&input))
                .on_ok(move |_, _window, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_save_prompt(&text, cx));
                    true
                })
        });
        self.prompt_input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Dialog OK for "Save prompt" — empty names save nothing.
    fn commit_save_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
        let name = self.prompt_input.read(cx).value().trim().to_string();
        if self.save_prompt(&name, text) {
            self.push_note(format!("Saved prompt `{name}` — the composer's ★ menu can load it."), cx);
        }
    }

    /// The popover row's ✎ — same dialog seeded with the current name; OK
    /// renames the prompt (empty or colliding input is a no-op).
    pub(crate) fn open_rename_prompt_dialog(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |state, cx| state.set_value(name.to_string(), window, cx));
        let ws = cx.entity();
        let input = self.prompt_input.clone();
        let old = name.to_string();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ws_ok = ws.clone();
            let old = old.clone();
            dialog
                .title("Rename prompt")
                .overlay_closable(true)
                .child(Input::new(&input))
                .on_ok(move |_, _window, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_rename_prompt(&old, cx));
                    true
                })
        });
        self.prompt_input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Dialog OK for "Rename prompt" — reads the new name from the shared
    /// input.
    fn commit_rename_prompt(&mut self, old: &str, cx: &mut Context<Self>) {
        let new = self.prompt_input.read(cx).value().trim().to_string();
        self.rename_prompt(old, &new, cx);
    }
}

/// First line of a prompt's text, capped at 60 chars — the `/prompts` note's
/// per-row preview.
fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(60).collect::<String>())
    } else {
        line.to_string()
    }
}
