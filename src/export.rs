//! Chat export: markdown save dialog and clipboard transcript. The HTML
//! variant lives in `crate::export::html` — split for the SLOC cap.

use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Printable-HTML export — split into `export_html.rs` for the SLOC cap;
/// reached as `crate::export::html`.
#[path = "export_html.rs"]
pub(crate) mod html;

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
        let name = format!("{}.md", export_stem(&chat.title));
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

    /// Copy the active chat's messages to the clipboard as markdown —
    /// `# <title>`, then `**You**` / `**Assistant**` + body per message.
    /// Cards collapse to a one-line italic summary so the paste stays a
    /// readable conversation; the markdown export keeps their full detail.
    /// A toast confirms — a silent copy leaves the user guessing.
    pub fn copy_transcript(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        if chat.messages.is_empty() {
            window.push_notification(Notification::warning("Nothing to copy — the transcript is empty"), cx);
            return;
        }
        let mut text = format!("# {}\n\n", chat.title);
        for m in chat.messages.iter() {
            let role = if m.role == Role::User { "You" } else { "Assistant" };
            text.push_str(&format!("**{role}**\n\n{}\n\n", transcript_body(m)));
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.trim_end().to_string()));
        window.push_notification(Notification::success("Transcript copied to clipboard"), cx);
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

/// A message's body in a copied transcript: text verbatim (it's already
/// markdown, fences and all), every card kind as a one-line italic
/// summary — the paste reads as a conversation, not a log dump.
fn transcript_body(m: &ChatMessage) -> String {
    match &m.kind {
        MessageKind::Text(t) => t.to_string(),
        MessageKind::Tool(t) => {
            let detail = t.detail.lines().next().unwrap_or_default();
            if detail.is_empty() {
                format!("*ran tool: {}*", t.name)
            } else {
                format!("*ran tool: {} — {detail}*", t.name)
            }
        },
        MessageKind::Diff(d) => format!("*edited `{}` (+{} -{})*", d.path, d.added, d.removed),
        MessageKind::Plan(p) => format!("*plan: {} step{}*", p.steps.len(), if p.steps.len() == 1 { "" } else { "s" }),
        MessageKind::Approval(a) => {
            let outcome = a.decision.map_or("pending", |d| d.label());
            format!("*{}: `{}` — {outcome}*", a.kind.label(), a.detail)
        },
    }
}

/// Single-quote a path for the shell — `'` inside becomes `'\''`.
fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Filesystem-safe filename stem from a chat title — shared by the markdown
/// and HTML exports so both suggest the same base name.
pub(crate) fn export_stem(title: &str) -> String {
    let stem: String = title.replace(['/', '\\', ':', '?', '*', '"', '<', '>', '|'], "-").chars().take(80).collect();
    if stem.is_empty() { "chat".into() } else { stem }
}

// Declared here, not in `main.rs` — that file is at the SLOC cap.
#[cfg(test)]
#[path = "export_tests.rs"]
mod export_tests;
