//! Slash commands: the composer's `/` menu and their local effects.
//!
//! `run_slash` dispatches a typed `/cmd arg` line — commands either act on
//! the workspace directly (`/clear`, `/compact`, `/status`, `/help`,
//! `/model`, `/export`, `/rename`, `/prompts`, `/save`) or send a canned
//! prompt (`/init`). Anything that parses as a command but isn't in
//! `SLASH_COMMANDS` gets a note instead of silently reaching the backend.

use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::send_queue::Queued;
use crate::workspace::Workspace;

/// `(name, description)` — the composer's `/` menu and `/help` both render
/// this table, so a command ships with its help text or not at all.
pub(crate) const SLASH_COMMANDS: [(&str, &str); 10] = [
    ("clear", "Clear this chat's messages"),
    ("compact", "Fold older messages into a context summary"),
    ("export", "Export this chat as Markdown"),
    ("help", "List the slash commands"),
    ("init", "Analyze the codebase and write AGENTS.md"),
    ("model", "Show or switch the model (`/model [instance/]id`)"),
    ("prompts", "List saved prompts"),
    ("rename", "Rename this chat"),
    ("save", "Save a prompt (`/save <name> [text]`)"),
    ("status", "Show provider, model, access and workspace"),
];

/// `/init` prompt — the first line doubles as the chat title.
const INIT_PROMPT: &str = "Write AGENTS.md for this codebase.\n\n\
    Analyze this repository and create an AGENTS.md file at the project root \
    that helps an agent work here: project purpose and layout, build, test \
    and lint commands, code conventions, and pitfalls worth knowing. \
    Read the manifests, entry points and existing docs first; keep the file \
    concise and factual.";

/// Split `/cmd arg` into `(cmd, arg)`; `None` when `text` isn't a slash
/// command. Command names are letters only, so a pasted path like
/// `/tmp/x` still reaches the backend as text.
fn slash_cmd(text: &str) -> Option<(&str, &str)> {
    let body = text.strip_prefix('/')?;
    let (cmd, arg) = body.split_once(' ').map_or((body, ""), |(c, a)| (c, a.trim()));
    if cmd.is_empty() || !cmd.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some((cmd, arg))
}

/// Commands that must run even mid-reply: `/compact` and `/clear` stop the
/// turn themselves, `/rename` never touches the message list. Note-producing
/// commands (`/help`, `/model`, `/status`, `/export`) queue like text
/// instead — run mid-stream their note becomes the last message and the
/// streaming reply appends into (or replaces) it.
fn slash_runs_now(cmd: &str) -> bool {
    matches!(cmd, "clear" | "compact" | "rename")
}

/// True when `text` is a `/command` line — known or not. `send` uses this
/// to keep composer attachments on local commands instead of snapshotting
/// them into the queue item.
pub(crate) fn is_slash(text: &str) -> bool {
    slash_cmd(text).is_some()
}

/// True when `text` is a command allowed to run while a reply streams.
pub(crate) fn runs_now(text: &str) -> bool {
    slash_cmd(text).is_some_and(|(cmd, _)| slash_runs_now(cmd))
}

/// First line of a message, capped at 80 chars — one digest row.
fn snippet(text: &str) -> String {
    let one_line = text.lines().next().unwrap_or("").trim();
    let end = one_line.char_indices().nth(80).map_or(one_line.len(), |(i, _)| i);
    if end < one_line.len() { format!("{}…", &one_line[..end]) } else { one_line.to_string() }
}

/// Compact context block replacing the dropped prefix: a per-message digest
/// so the gist survives while the bulk is gone.
fn compact_digest(dropped: &[ChatMessage]) -> String {
    const SHOWN: usize = 12;
    let mut out = String::from("**Compacted context** — summary of the earlier conversation:\n");
    for m in dropped.iter().take(SHOWN) {
        let row = match &m.kind {
            MessageKind::Text(t) => format!("- {}: {}", if m.role == Role::User { "user" } else { "assistant" }, snippet(t)),
            MessageKind::Tool(t) => format!("- tool `{}`: {}", t.name, snippet(&t.output)),
            MessageKind::Diff(d) => format!("- diff `{}`: +{} −{}", d.path, d.added, d.removed),
            MessageKind::Plan(p) => format!("- plan: {}", p.steps.iter().map(|s| s.label.as_str()).collect::<Vec<_>>().join("; ")),
            MessageKind::Approval(a) => format!("- approval {}: {}", a.kind.label().to_lowercase(), snippet(&a.detail)),
        };
        out.push_str(&row);
        out.push('\n');
    }
    if dropped.len() > SHOWN {
        out.push_str(&format!("- …and {} more messages\n", dropped.len() - SHOWN));
    }
    out
}

impl Workspace {
    /// Run a `/command` locally. Returns true when the input was consumed —
    /// including unknown commands, which earn a note rather than a silent
    /// send to the backend.
    pub(crate) fn run_slash(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((cmd, arg)) = slash_cmd(text) else { return false };
        match cmd {
            "clear" => self.clear_active_chat(cx),
            "compact" => self.compact_active_chat(cx),
            "export" => self.export_active(cx),
            "help" => {
                let list = SLASH_COMMANDS.iter().map(|(c, d)| format!("- `/{c}` — {d}")).collect::<Vec<_>>().join("\n");
                self.push_note(format!("**Commands:**\n{list}"), cx);
            },
            "init" => self.send_text(Queued::new(INIT_PROMPT.to_string(), Vec::new()), window, cx),
            "model" => self.model_command(arg, cx),
            "prompts" => self.prompts_note(cx),
            "rename" => self.rename_active(window, cx),
            "save" => self.save_prompt_command(arg, cx),
            "status" => self.status_note(cx),
            _ => {
                let known = SLASH_COMMANDS.iter().map(|(c, _)| format!("`/{c}`")).collect::<Vec<_>>().join(" ");
                self.push_note(format!("Unknown command `/{cmd}` — try `/help`. Commands: {known}"), cx);
            },
        }
        true
    }

    /// `/clear` — empty the active chat's transcript; the chat itself stays.
    fn clear_active_chat(&mut self, cx: &mut Context<Self>) {
        if self.chats[self.active].running {
            self.stop_reply(cx);
        }
        let chat = &mut self.chats[self.active];
        Rc::make_mut(&mut chat.messages).clear();
        chat.last_turn = None;
        self.recall_ix = None;
        self.recall_saved = None;
        self.search_match_ix = 0;
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
        self.save();
    }

    /// `/compact` — replace all but the last few messages with a digest, so
    /// the transcript (and the context the backend resumes from) shrinks
    /// while the gist of the dropped prefix survives.
    fn compact_active_chat(&mut self, cx: &mut Context<Self>) {
        if self.chats[self.active].running {
            self.stop_reply(cx);
        }
        let chat = &mut self.chats[self.active];
        let keep = 4.min(chat.messages.len());
        let drain_to = chat.messages.len() - keep;
        if drain_to == 0 {
            self.push_note("Nothing to compact — the transcript is already short.".into(), cx);
            return;
        }
        let dropped: Vec<ChatMessage> = Rc::make_mut(&mut chat.messages).drain(..drain_to).collect();
        Rc::make_mut(&mut chat.messages).insert(
            0,
            ChatMessage {
                role: Role::Assistant,
                kind: MessageKind::Text(compact_digest(&dropped).into()),
                rating: None,
                usage: None,
                attachments: vec![],
                at: SystemTime::now(),
            },
        );
        chat.last_turn = None;
        self.recall_ix = None;
        self.recall_saved = None;
        self.search_match_ix = 0;
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        self.save();
        self.push_note(format!("Compacted — folded {drain_to} messages into a summary, kept the last {keep}."), cx);
    }

    /// `/status` — report the effective provider, model, access and workdir.
    fn status_note(&mut self, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let workdir = if chat.workdir.is_empty() {
            self.project.root().display().to_string()
        } else {
            chat.workdir.clone()
        };
        self.push_note(
            format!(
                "**Status**\n- provider: `{}` ({})\n- model: `{}`\n- mode: {} · access: {}\n- workspace: {workdir}\n- messages: {}",
                self.selected_provider,
                self.backend.name(),
                self.selected_model(),
                self.mode,
                self.access.name(),
                chat.messages.len(),
            ),
            cx,
        );
    }

    /// `/model` — `instance/model` selects across instances; a bare id stays
    /// on the selected instance. No arg reports the current pick.
    fn model_command(&mut self, arg: &str, cx: &mut Context<Self>) {
        let current = self.selected_provider.clone();
        let (instance, model_id) = arg.split_once('/').map_or((current.as_str(), arg), |(i, m)| (i, m));
        let instance = instance.to_string();
        let known: Vec<String> = self.models_for(&instance).iter().map(|m| m.id.to_string()).collect();
        if arg.is_empty() {
            self.push_note(
                format!("Current model: **{} · {}** — pick one of: {}", self.selected_provider, self.selected_model(), known.join(", ")),
                cx,
            );
        } else if self.select_model(&instance, model_id, cx) {
            self.push_note(format!("Model set to **{instance} · {model_id}**"), cx);
        } else {
            self.push_note(format!("Unknown model `{arg}` — pick one of: {} (or `instance/model`)", known.join(", ")), cx);
        }
    }
}
