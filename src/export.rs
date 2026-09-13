//! Chat export: markdown save dialog and clipboard transcript.

use gpui_kit::*;

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

impl Workspace {
    /// Export chat `ix` as markdown via the native save dialog.
    pub fn export_chat(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(ix) else { return };
        let mut out = format!("# {}\n\n", chat.title);
        for msg in chat.messages.iter() {
            let role = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
            };
            let body = match &msg.kind {
                MessageKind::Text(t) => t.to_string(),
                MessageKind::Tool(t) => format!("`{} {}`\n```\n{}\n```", t.name, t.detail, t.output),
                MessageKind::Diff(d) => format!("`{}` +{} -{}\n```diff\n{}\n```", d.path, d.added, d.removed, d.hunks),
            };
            out.push_str(&format!("## {role}\n\n{body}\n\n"));
        }
        let stem: String = chat.title.replace(['/', '\\', ':', '?', '*', '"', '<', '>', '|'], "-").chars().take(80).collect();
        let name = format!("{}.md", if stem.is_empty() { "chat" } else { &stem });
        let home = std::env::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&home, Some(&name));
        let ws = cx.entity();
        cx.spawn(async move |_this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let msg = match std::fs::write(&path, out) {
                Ok(()) => format!("Exported to `{}`", path.display()),
                Err(_) => format!("Export failed — could not write `{}`", path.display()),
            };
            ws.update(cx, |this, cx| this.push_note(msg, cx));
        })
        .detach();
    }

    pub fn export_active(&mut self, cx: &mut Context<Self>) {
        let ix = self.active;
        self.export_chat(ix, cx);
    }

    /// Copy the active chat's messages to the clipboard as markdown.
    pub fn copy_transcript(&mut self, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let text = chat
            .messages
            .iter()
            .map(|m| {
                let role = if m.role == Role::User { "You" } else { "Rixl" };
                let body = match &m.kind {
                    MessageKind::Text(t) => t.to_string(),
                    MessageKind::Tool(t) => format!("`{} {}`\n```\n{}\n```", t.name, t.detail, t.output),
                    MessageKind::Diff(d) => format!("`{}` +{} -{}\n```diff\n{}\n```", d.path, d.added, d.removed, d.hunks),
                };
                format!("{role}: {body}")
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}
