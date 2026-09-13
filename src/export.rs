//! Chat export: markdown save dialog and clipboard transcript.

use gpui_kit::*;

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

impl Workspace {
    /// Export chat `ix` as markdown via the native save dialog.
    pub fn export_chat(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(ix) else { return };
        let mut out = format!("# {}\n\n", chat.title);
        for msg in &chat.messages {
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
        let name = format!("{}.md", chat.title.replace(['/', '\\', ':', '?', '*', '"', '<', '>', '|'], "-"));
        let home = std::env::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&home, Some(&name));
        cx.spawn(async move |_this, _cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                std::fs::write(path, out).ok();
            }
        })
        .detach();
    }

    pub fn export_active(&mut self, cx: &mut Context<Self>) {
        let ix = self.active;
        self.export_chat(ix, cx);
    }

    /// Copy the active chat's text messages to the clipboard.
    pub fn copy_transcript(&mut self, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let text = chat
            .messages
            .iter()
            .filter_map(|m| match &m.kind {
                MessageKind::Text(t) => Some(format!("{}: {}", if m.role == Role::User { "You" } else { "Rixl" }, t)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}
