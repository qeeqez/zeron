//! AI chat titles: when a chat's first backend turn completes and the
//! title is still the placeholder ("New chat" or the truncated first
//! prompt), a one-off turn asks the backend to name the conversation —
//! the same standalone send+drain pattern as `changes_generate`. The
//! result replaces the placeholder and the window title; a manual rename
//! mid-flight wins because the title is re-checked before it lands.
//! When the backend can't produce a title — no model, no usable reply,
//! or an error — the title is derived from the first user message
//! instead (`derive_title`).

use gpui_kit::*;

use crate::backend::{AgentEvent, ApprovalDecision, ReplyStream};
use crate::model::{Chat, MessageKind, Role};
use crate::workspace::Workspace;

/// Longest auto-generated title — anything past this is truncated at a
/// char boundary.
const MAX_TITLE_CHARS: usize = 60;

/// Longest derived title — the first clause of the user's message,
/// ellipsized past this.
const MAX_DERIVED_CHARS: usize = 48;

/// Per-message cap on the excerpt sent for titling — enough for the
/// backend to grasp the topic, small enough to keep the turn cheap.
const MAX_EXCERPT_CHARS: usize = 2000;

/// What an in-flight title turn carries back to the workspace.
struct PendingTitle {
    chat_id: u64,
    /// The placeholder the chat carried when the turn was sent — a manual
    /// rename since then wins, so the generated title is dropped.
    expected: SharedString,
    /// The sanitized title — `None` asks the landing side to derive one
    /// from the first user message (backend error, empty reply, or no
    /// model to send with).
    title: Option<String>,
}

impl Workspace {
    /// Kick off title generation after `chat_id`'s turn completes.
    /// `had_stream` is the caller's proof a real backend turn ran —
    /// `finish_reply` also fires for local bail-outs (no model, auth
    /// block), which must not title a chat off an error note. No-op when
    /// the turn failed or the title was already generated or renamed.
    /// Without a model — or when the backend turn yields nothing — the
    /// title falls back to `derive_title` on the first user message.
    pub(crate) fn maybe_generate_title(&mut self, chat_id: u64, had_stream: bool, cx: &mut Context<Self>) {
        if !had_stream {
            return;
        }
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        if chat.failed_flag || chat.title_generated || chat.title_custom || !has_placeholder_title(chat) {
            return;
        }
        let prompt = if self.model.is_empty() { None } else { title_prompt(chat) };
        let mut pending = PendingTitle { chat_id, expected: chat.title.clone(), title: None };
        let backend = self.backend.clone();
        let model = self.model.to_string();
        // Read-only like the commit-message turn — it only writes a title.
        let ctx = crate::backend::TurnContext::at(
            crate::worktree::workdir_for(chat, self.project.root()),
            self.effective_access(chat.access.unwrap_or(self.access)),
        );
        cx.spawn(async move |this, cx| {
            if let Some(prompt) = prompt {
                let stream = backend.send(&prompt, &model, "Ask", &ctx);
                pending.title = collect_title(stream, cx).await;
            }
            let _ = this.update_in(cx, |this, window, cx| this.land_generated_title(pending, window, cx));
        })
        .detach();
    }

    /// Publish a generated title. A manual rename committed while the turn
    /// was in flight wins — the title is applied only when the chat still
    /// carries the placeholder it had at send time. A deleted chat keeps
    /// its title; a missing backend title falls back to `derive_title`,
    /// and only a chat with no usable first message keeps the placeholder.
    fn land_generated_title(&mut self, pending: PendingTitle, window: &mut Window, cx: &mut Context<Self>) {
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == pending.chat_id);
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == pending.chat_id) else { return };
        if chat.title != pending.expected || chat.title_custom {
            return;
        }
        let Some(title) = pending.title.or_else(|| first_text(&*chat, Role::User).and_then(derive_title)) else {
            return;
        };
        chat.title = title.into();
        chat.title_generated = true;
        if is_active {
            window.set_window_title(&format!("{} — Rixl Code", chat.title));
        }
        cx.notify();
        self.save();
    }
}

/// The placeholder a fresh chat gets on its first send — the prompt's
/// first line, truncated. `send.rs` writes it; `has_placeholder_title`
/// recognizes it so only an untouched title is replaced.
pub(crate) fn provisional_title(text: &str) -> String {
    text.lines().next().unwrap_or("").chars().take(40).collect()
}

/// Whether `chat`'s title is still the placeholder — "New chat" or the
/// truncated first prompt. A manual rename fails this check, so the
/// user's title is never overwritten.
fn has_placeholder_title(chat: &Chat) -> bool {
    if chat.title == "New chat" {
        return true;
    }
    let Some(first) = chat.messages.iter().find(|m| m.role == Role::User) else { return false };
    let MessageKind::Text(text) = &first.kind else { return false };
    chat.title.as_ref() == provisional_title(text)
}

/// Title from a raw user message, no backend: the first paragraph with
/// markdown stripped and whitespace collapsed, cut at the first clause
/// boundary and ellipsized past `MAX_DERIVED_CHARS`. `None` when nothing
/// usable remains — the caller keeps the placeholder.
fn derive_title(text: &str) -> Option<String> {
    let plain = plain_text(text);
    let clause = first_clause(&plain);
    (!clause.is_empty()).then(|| cap_title(clause))
}

/// The message's first paragraph as flat text: fenced-code markers,
/// headings, list markers, and quotes dropped; links/images reduced to
/// their label; emphasis/backtick delimiters removed; whitespace
/// collapsed to single spaces.
fn plain_text(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            // The title comes from the first paragraph — a blank line
            // ends it (also keeps the "📎 files" attachment footer out).
            if !lines.is_empty() {
                break;
            }
            continue;
        }
        if line.starts_with("```") || line.starts_with("~~~") {
            continue;
        }
        if line.starts_with('#') {
            // A heading is its own block — it ends the paragraph.
            lines.push(strip_inline(line.trim_start_matches(['#', ' '])));
            break;
        }
        let line = line.trim_start_matches(['>', ' ']);
        let line = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .or_else(|| line.strip_prefix("+ "))
            .unwrap_or(line);
        // Ordered-list marker — "1. fix the bug" titles as "fix the bug".
        let line = match line.split_once(' ') {
            Some((num, rest))
                if num.len() > 1
                    && num[..num.len() - 1].chars().all(|c| c.is_ascii_digit())
                    && matches!(num.chars().last(), Some('.') | Some(')')) =>
            {
                rest
            },
            _ => line,
        };
        lines.push(strip_inline(line));
    }
    lines.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Inline markdown out of `line`: `[label](url)` and `![alt](src)` keep
/// the label, `<tag>`/`<url>` drop, and `*_~` plus backticks are
/// stripped from token edges (inner `_` stays — `snake_case` survives).
fn strip_inline(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '!' if chars.get(i + 1) == Some(&'[') => i += 1,
            '[' => {
                let close = chars[i..].iter().position(|c| *c == ']').map(|p| i + p);
                let open = close.and_then(|c| (chars.get(c + 1) == Some(&'(')).then_some(c + 1));
                let end = open.and_then(|o| chars[o..].iter().position(|c| *c == ')').map(|p| o + p));
                match (close, end) {
                    (Some(c), Some(e)) => {
                        out.push_str(&strip_inline(&chars[i + 1..c].iter().collect::<String>()));
                        i = e + 1;
                    },
                    (Some(c), None) => {
                        out.push_str(&strip_inline(&chars[i + 1..c].iter().collect::<String>()));
                        i = c + 1;
                    },
                    _ => {
                        out.push('[');
                        i += 1;
                    },
                }
            },
            '<' if chars.get(i + 1).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '/' || *c == '!' || *c == '?') => {
                match chars[i..].iter().position(|c| *c == '>') {
                    Some(p) => i += p + 1,
                    None => i = chars.len(),
                }
            },
            c => {
                out.push(c);
                i += 1;
            },
        }
    }
    out.split_whitespace()
        .map(|tok| tok.trim_matches(['*', '_', '~', '`', '"', '\'']))
        .filter(|tok| !tok.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text` up to the first clause boundary: `.`/`!`/`?`/`;`/`:`/`,` at a
/// word boundary, or `(`/`—`/`–` anywhere. The boundary char itself is
/// dropped.
fn first_clause(text: &str) -> &str {
    for (i, c) in text.char_indices() {
        let boundary = match c {
            '(' | '—' | '–' => true,
            '.' | '!' | '?' | ';' | ':' | ',' => text[i + c.len_utf8()..].chars().next().is_none_or(|n| n.is_whitespace()),
            _ => false,
        };
        if boundary {
            return text[..i].trim_end();
        }
    }
    text
}

/// `clause` capped at `MAX_DERIVED_CHARS` — cut back to the last word
/// boundary when one survives, then an ellipsis marks the truncation.
fn cap_title(clause: &str) -> String {
    if clause.chars().count() <= MAX_DERIVED_CHARS {
        return clause.to_string();
    }
    let head: String = clause.chars().take(MAX_DERIVED_CHARS).collect();
    let head = head.trim_end();
    let cut = match head.rfind(' ').filter(|ix| *ix > 0) {
        Some(ix) => head[..ix].trim_end(),
        None => head,
    };
    format!("{cut}…")
}

/// The one-off prompt: the first user message and first assistant reply
/// (each capped) plus the output contract — a bare 3-6 word title.
fn title_prompt(chat: &Chat) -> Option<String> {
    let user = first_text(chat, Role::User)?;
    let assistant = first_text(chat, Role::Assistant)?;
    Some(format!(
        "Summarize this conversation as a 3-6 word title. \
         Reply with only the title — no quotes, no punctuation, no explanation.\n\n\
         User: {}\n\nAssistant: {}",
        excerpt(user),
        excerpt(assistant)
    ))
}

/// First `Text` message from `role` — tool calls and notes don't count.
fn first_text(chat: &Chat, role: Role) -> Option<&str> {
    chat.messages.iter().filter(|m| m.role == role).find_map(|m| match &m.kind {
        MessageKind::Text(t) => Some(t.as_str()),
        _ => None,
    })
}

/// `text` capped at `MAX_EXCERPT_CHARS` on a char boundary.
fn excerpt(text: &str) -> &str {
    if text.len() <= MAX_EXCERPT_CHARS {
        return text;
    }
    &text[..text.floor_char_boundary(MAX_EXCERPT_CHARS)]
}

/// Drain a one-off turn's stream into its reply text. Polls like the chat
/// pump — a blocking `recv` would wedge the test scheduler. Approvals are
/// auto-denied: this turn only writes text, and a pending request would
/// otherwise hang the drain. `None` on error, disconnect, or an empty
/// reply — the caller keeps the existing title.
async fn collect_title(stream: ReplyStream, cx: &mut AsyncApp) -> Option<String> {
    let mut text = String::new();
    'outer: loop {
        loop {
            match stream.events.try_recv() {
                Ok(AgentEvent::TextDelta(delta)) => text.push_str(&delta),
                Ok(AgentEvent::Done | AgentEvent::Error(_)) => break 'outer,
                Ok(AgentEvent::ApprovalRequest { respond, .. }) => {
                    let _ = respond.send(ApprovalDecision::Deny);
                },
                Ok(_) => {},
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break 'outer,
            }
        }
        cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
    }
    clean_title(&text)
}

/// First non-empty line of the reply, stripped of the decoration models
/// like to add — bullets, quotes, fences, emphasis — and of trailing
/// punctuation, capped at `MAX_TITLE_CHARS`. `None` when the reply
/// carried no usable text.
fn clean_title(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches(['-', '*', '>', '#', ' ']).trim();
    let line = line.trim_matches(['`', '"', '*', '\'']);
    let line = line.trim_end_matches(['.', '!', '?', ',', ';', ':']).trim();
    let title: String = line.chars().take(MAX_TITLE_CHARS).collect();
    let title = title.trim_end();
    (!title.is_empty()).then(|| title.to_string())
}

#[cfg(test)]
#[path = "chat_title_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "chat_title_ui_tests.rs"]
mod ui_tests;
