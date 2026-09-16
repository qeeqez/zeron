//! Chat export: markdown save dialog and clipboard transcript.

use gpui_kit::*;

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

impl Workspace {
    /// Export chat `ix` as markdown via the native save dialog.
    /// Temporary chats can't be exported — nothing about them persists.
    pub fn export_chat(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(ix) else { return };
        if chat.ephemeral {
            self.push_note("Temporary chats can't be exported.".into(), cx);
            return;
        }
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
                MessageKind::Plan(p) => p.markdown(),
                MessageKind::Approval(a) => {
                    let outcome = a.decision.map_or("pending", |d| d.label());
                    format!("**{}:** `{}` — {}", a.kind.label(), a.detail, outcome)
                },
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
                    MessageKind::Plan(p) => p.markdown(),
                    MessageKind::Approval(a) => {
                        let outcome = a.decision.map_or("pending", |d| d.label());
                        format!("**{}:** `{}` — {}", a.kind.label(), a.detail, outcome)
                    },
                };
                format!("{role}: {body}")
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// The shell command that continues the active chat's backend thread in
    /// a terminal: `cd <workdir> && <backend resume cmd>`. `None` when the
    /// chat has no backend thread or the backend has no CLI resume — the ⋯
    /// menu hides its item then. `cd` is always prefixed: claude keys
    /// sessions by project dir, and codex prompts to pick a directory when
    /// the session's recorded cwd differs from the shell's.
    pub fn resume_command(&self) -> Option<String> {
        let chat = &self.chats[self.active];
        if chat.thread_id.is_empty() {
            return None;
        }
        let cmd = self.backend.resume_command(&chat.thread_id)?;
        let dir = crate::worktree::workdir_for(chat, self.project.root());
        Some(format!("cd {} && {cmd}", shell_quote(&dir.to_string_lossy())))
    }

    /// Copy the resume command to the clipboard and note it in the chat —
    /// a silent copy leaves the user guessing whether it worked.
    pub fn copy_resume_command(&mut self, cx: &mut Context<Self>) {
        let Some(cmd) = self.resume_command() else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(cmd.clone()));
        self.push_note(format!("Copied: `{cmd}`"), cx);
    }
}

/// Single-quote a path for the shell — `'` inside becomes `'\''`.
fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}
