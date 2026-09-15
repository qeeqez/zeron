//! Chat folders — a `Chat::folder` string groups chats under collapsible
//! sidebar sections. Folders are implicit: they exist only while a chat
//! names one, so "rename"/"delete" rewrite member chats rather than a
//! folder table. The dialogs share `Workspace::folder_input` like the
//! rename dialog shares `Workspace::rename`.

use gpui_kit::component::WindowExt;
use gpui_kit::component::input::Input;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// Folder names in sidebar order — alphabetical, case-insensitive.
    /// Only chats the sidebar lists (non-archived) contribute a folder.
    pub fn folder_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .chats
            .iter()
            .filter(|c| !c.archived && !c.folder.is_empty())
            .map(|c| c.folder.clone())
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    /// File the chat with `id` under `folder`; empty unfiles it.
    pub fn set_chat_folder(&mut self, id: u64, folder: &str, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.iter_mut().find(|c| c.id == id) {
            chat.folder = folder.trim().to_string();
        }
        cx.notify();
        self.save();
    }

    /// Rename `old` to `new` across every chat filed under it — the folder
    /// "entity" is just the shared name, so this is the rename. An empty
    /// `new` unfiles the members (delete).
    pub fn rename_folder(&mut self, old: &str, new: &str, cx: &mut Context<Self>) {
        let new = new.trim();
        if old.is_empty() || new.is_empty() || old == new {
            return;
        }
        for chat in &mut self.chats {
            if chat.folder == old {
                chat.folder = new.to_string();
            }
        }
        if self.collapsed_folders.remove(old) {
            self.collapsed_folders.insert(new.to_string());
        }
        cx.notify();
        self.save();
    }

    /// Delete a folder — its chats fall back to Unfiled.
    pub fn delete_folder(&mut self, name: &str, cx: &mut Context<Self>) {
        if name.is_empty() {
            return;
        }
        for chat in &mut self.chats {
            if chat.folder == name {
                chat.folder = String::new();
            }
        }
        self.collapsed_folders.remove(name);
        cx.notify();
        self.save();
    }

    /// Expand/collapse a folder's sidebar section.
    pub fn toggle_folder(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.collapsed_folders.remove(name) {
            self.collapsed_folders.insert(name.to_string());
        }
        cx.notify();
    }

    /// "New folder…" from the Move-to-folder submenu — a small dialog whose
    /// OK files the chat under the typed name.
    pub fn open_folder_dialog(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.folder_input.update(cx, |state, cx| state.set_value("", window, cx));
        let ws = cx.entity();
        let input = self.folder_input.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ws_ok = ws.clone();
            dialog
                .title("Move to folder")
                .overlay_closable(true)
                .child(Input::new(&input))
                .on_ok(move |_, _window, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_folder_move(id, cx));
                    true
                })
        });
    }

    /// "Rename folder" from the folder header's menu — same dialog, seeded
    /// with the current name; OK rewrites every member chat.
    pub fn open_rename_folder_dialog(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.folder_input.update(cx, |state, cx| state.set_value(name.to_string(), window, cx));
        let ws = cx.entity();
        let input = self.folder_input.clone();
        let old = name.to_string();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ws_ok = ws.clone();
            let old = old.clone();
            dialog
                .title("Rename folder")
                .overlay_closable(true)
                .child(Input::new(&input))
                .on_ok(move |_, _window, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_folder_rename(&old, cx));
                    true
                })
        });
    }

    /// Dialog OK for "Move to folder" — empty input unfiles the chat.
    fn commit_folder_move(&mut self, id: u64, cx: &mut Context<Self>) {
        let name = self.folder_input.read(cx).value().trim().to_string();
        self.set_chat_folder(id, &name, cx);
    }

    /// Dialog OK for "Rename folder" — empty or unchanged input is a no-op.
    fn commit_folder_rename(&mut self, old: &str, cx: &mut Context<Self>) {
        let new = self.folder_input.read(cx).value().trim().to_string();
        self.rename_folder(old, &new, cx);
    }
}
