//! Slash commands: the composer's `/` menu and their local effects.
//!
//! `run_slash` dispatches a typed `/cmd arg` line — commands either act on
//! the workspace directly (`/clear`, `/compact`, `/status`, `/help`,
//! `/model`, `/export`, `/rename`, `/prompts`, `/save`) or send a canned
//! prompt (`/init`). Anything that parses as a command but isn't in
//! `SLASH_COMMANDS` gets a note instead of silently reaching the backend.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::send_queue::Queued;
use crate::workspace::Workspace;

/// `(name, icon, description)` — the composer's `/` menu and `/help` both
/// render this table, so a command ships with its help text or not at all.
pub(crate) const SLASH_COMMANDS: [(&str, IconName, &str); 10] = [
    ("clear", IconName::Eraser, "Clear this chat's messages"),
    ("compact", IconName::ListCollapse, "Compact the conversation context"),
    ("export", IconName::FileDown, "Export this chat as Markdown"),
    ("help", IconName::CircleQuestionMark, "List the slash commands"),
    ("init", IconName::Sparkles, "Analyze the codebase and write AGENTS.md"),
    ("model", IconName::Cpu, "Show or switch the model (`/model [instance/]id`)"),
    ("prompts", IconName::Star, "List saved prompts"),
    ("rename", IconName::PenLine, "Rename this chat"),
    ("save", IconName::Save, "Save a prompt (`/save <name> [text]`)"),
    ("status", IconName::Info, "Show provider, model, access and workspace"),
];

/// `/init` prompt — the first line doubles as the chat title.
const INIT_PROMPT: &str = "Write AGENTS.md for this codebase.\n\n\
    Analyze this repository and create an AGENTS.md file at the project root \
    that helps an agent work here: project purpose and layout, build, test \
    and lint commands, code conventions, and pitfalls worth knowing. \
    Read the manifests, entry points and existing docs first; keep the file \
    concise and factual.";

/// `/compact`'s fallback turn when the backend can't compact natively:
/// ask the model for a context handoff over the serialized transcript.
/// Backends without threads see only this prompt, so the transcript rides
/// along — without it the model would summarize nothing.
const COMPACT_PROMPT: &str = "Summarize this conversation so far into a compact context handoff \
    for continuing the work. Cover: the user's goal, decisions made, files and code touched, \
    current state, and what remains. Be concise and factual — the summary replaces the transcript \
    as working context.\n\nTranscript:\n";

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

/// Commands that must run even mid-reply: `/clear` stops the turn itself,
/// `/rename` never touches the message list. Note-producing commands
/// (`/help`, `/model`, `/status`, `/export`) and `/compact` queue like
/// text instead — run mid-stream their note becomes the last message and
/// the streaming reply appends into (or replaces) it, and a compaction
/// turn must not race the reply it would fold.
fn slash_runs_now(cmd: &str) -> bool {
    matches!(cmd, "clear" | "rename")
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

/// One message's body for the compact prompt, capped so a giant tool dump
/// can't crowd out the rest of the transcript.
fn compact_body(m: &ChatMessage) -> String {
    const MAX: usize = 2_000;
    let body = match &m.kind {
        MessageKind::Text(t) => t.to_string(),
        MessageKind::Tool(t) => format!("`{} {}`\n{}", t.name, t.detail, t.output),
        MessageKind::Diff(d) => format!("`{}` +{} -{}\n{}", d.path, d.added, d.removed, d.hunks),
        MessageKind::Plan(p) => p.markdown(),
        MessageKind::Approval(a) => format!("{}: {}", a.kind.label(), a.detail),
    };
    if body.len() <= MAX {
        return body;
    }
    format!("{}…", &body[..body.floor_char_boundary(MAX)])
}

/// The transcript as the compact prompt's input: `Role: body` sections,
/// newest kept when the whole thing would exceed the cap — recent context
/// matters most to a handoff.
fn compact_transcript(messages: &[ChatMessage]) -> String {
    const MAX_TOTAL: usize = 150_000;
    let mut kept: Vec<String> = Vec::new();
    let mut total = 0usize;
    for m in messages.iter().rev() {
        let role = match m.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        let section = format!("{role}: {}\n\n", compact_body(m));
        if total + section.len() > MAX_TOTAL {
            break;
        }
        total += section.len();
        kept.push(section);
    }
    kept.reverse();
    let mut out = String::new();
    if kept.len() < messages.len() {
        out.push_str(&format!("[{} earlier messages omitted]\n\n", messages.len() - kept.len()));
    }
    out.push_str(&kept.concat());
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
            "compact" => self.compact_active_chat(window, cx),
            "export" => self.export_active(cx),
            "help" => {
                let list = SLASH_COMMANDS.iter().map(|(c, _, d)| format!("- `/{c}` — {d}")).collect::<Vec<_>>().join("\n");
                self.push_note(format!("**Commands:**\n{list}"), cx);
            },
            "init" => self.send_text(Queued::new(INIT_PROMPT.to_string(), Vec::new()), window, cx),
            "model" => self.model_command(arg, cx),
            "prompts" => self.prompts_note(cx),
            "rename" => self.rename_active(window, cx),
            "save" => self.save_prompt_command(arg, cx),
            "status" => self.status_note(cx),
            _ => {
                let known = SLASH_COMMANDS.iter().map(|(c, ..)| format!("`/{c}`")).collect::<Vec<_>>().join(" ");
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
        self.clear_recall();
        self.search_match_ix = 0;
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
        self.save();
    }

    /// `/compact` — fold the conversation's context. A chat bound to a
    /// codex thread compacts server-side (`thread/compact/start`); every
    /// other chat gets a summarization turn carrying the transcript, so
    /// the reply is a context handoff the user can continue from.
    fn compact_active_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if crate::backend_run::run_compact(self, cx) {
            return;
        }
        let chat = &self.chats[self.active];
        if chat.messages.is_empty() {
            self.push_note("Nothing to compact — the transcript is empty.".into(), cx);
            return;
        }
        let prompt = format!("{COMPACT_PROMPT}\n{}", compact_transcript(&chat.messages));
        self.send_prompt_as("/compact", prompt, window, cx);
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
