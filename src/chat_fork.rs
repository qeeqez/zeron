//! Fork-here: branch the conversation at a message without editing it.
//! Unlike `chat_edit` (truncate + resend), forking is non-destructive —
//! the original chat keeps its full transcript and a NEW chat opens with
//! the messages up to and including the fork point, ready for a different
//! follow-up.

#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui_kit::*;

use crate::model::{Chat, MessageKind, Role};
use crate::send_queue::Queued;
use crate::workspace::Workspace;

/// `fork_inner`'s options — bundled to stay under the arg-count lint.
struct ForkOpts {
    /// Fork point: copy messages through this index; `None` = the end.
    at: Option<usize>,
    /// Appended to the source title — " · fork" or " · <provider>".
    suffix: String,
    /// Provider/model stamps for the fork — `None` keeps the source's.
    bind: Option<(String, String)>,
    /// Permit forking an empty transcript — compare forks a fresh chat so
    /// the prompt lands as its first message; other callers keep the
    /// empty-transcript no-op.
    allow_empty: bool,
}

impl ForkOpts {
    /// A same-provider fork at `at` — `fork_chat`'s shape.
    fn at(at: Option<usize>) -> Self {
        Self {
            at,
            suffix: " · fork".to_string(),
            bind: None,
            allow_empty: false,
        }
    }
}

impl Workspace {
    /// Branch chat `chat_ix` at message `msg_ix`: a new chat titled
    /// "<title> · fork" opens holding the messages up to and including
    /// `msg_ix`. The original is untouched — unlike `commit_edit`, nothing
    /// is truncated or resent. `None` forks at the end of the transcript.
    /// The fork carries the thread's provider/model/access stamps so its
    /// follow-up turn runs on the same configuration; a worktree thread's
    /// fork gets its own worktree (sharing the original's would break when
    /// either chat is deleted). The backend thread id is NOT copied — a
    /// backend thread can't be partially rewound, so the fork always
    /// starts a fresh backend thread, like `duplicate_chat`.
    pub fn fork_chat(&mut self, chat_ix: usize, msg_ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.fork_inner(chat_ix, ForkOpts::at(msg_ix), window, cx);
    }

    /// "Continue with…": fork chat `chat_ix` onto another provider — the
    /// whole transcript lands in a new chat titled "<title> · <provider>"
    /// bound to `provider_id` (its first effective model), with no backend
    /// thread id: the old thread can't resume across providers, so the
    /// first send starts a fresh thread on the new backend. No-op when the
    /// instance is gone/disabled, already the chat's provider, or the
    /// transcript is empty.
    pub fn continue_chat_with(&mut self, chat_ix: usize, provider_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(src) = self.chats.get(chat_ix) else { return };
        // Legacy chats (empty stamp) ride the live selection — same rule
        // `chat_info`/`usage_dashboard` apply.
        let current = if src.provider.is_empty() { self.selected_provider.as_str() } else { src.provider.as_str() };
        let Some(p) = self.providers.iter().find(|p| p.id == provider_id && p.enabled) else { return };
        if p.id == current {
            return;
        }
        let model = self.models_for(provider_id).first().map_or_else(String::new, |m| m.id.to_string());
        let opts = ForkOpts {
            suffix: format!(" · {}", p.name),
            bind: Some((provider_id.to_string(), model)),
            ..ForkOpts::at(None)
        };
        self.fork_inner(chat_ix, opts, window, cx);
    }

    /// "Compare providers…" confirm: fork the active chat once per picked
    /// instance — each fork titled "<title> · <provider>" and bound to that
    /// provider's first effective model — then send every fork the same
    /// prompt. The prompt is the composer draft; when that's empty it
    /// falls back to the last user message's text (attachments ride along,
    /// like an edit-resend). The draft itself stays on the source chat —
    /// `select_chat` stashes it there as the forks take focus, so the user
    /// can still send it in the original thread. The first fork ends
    /// selected; the rest run their turns in the background. No-op while
    /// the chat runs, with no prompt to send, or with fewer than two
    /// enabled picks.
    pub fn compare_providers(&mut self, provider_ids: &[String], window: &mut Window, cx: &mut Context<Self>) {
        let src_ix = self.active;
        if self.chats[src_ix].running {
            return;
        }
        let draft = self.composer.read(cx).value().trim().to_string();
        let item = if draft.is_empty() {
            self.chats[src_ix].messages.iter().rev().find(|m| m.role == Role::User).and_then(|m| match &m.kind {
                MessageKind::Text(text) => {
                    Some(Queued::new(crate::chat_edit::strip_attachment_suffix(text, &m.attachments), m.attachments.clone()))
                },
                _ => None,
            })
        } else {
            Some(Queued::new(draft, Vec::new()))
        };
        let Some(item) = item else { return };
        // Resolve picks to (id, title suffix, first effective model) —
        // gone/disabled instances drop out; under two left, no compare.
        let targets: Vec<(String, String, String)> = provider_ids
            .iter()
            .filter_map(|id| {
                let p = self.providers.iter().find(|p| p.id == *id && p.enabled)?;
                let model = self.models_for(&p.id).first().map_or_else(String::new, |m| m.id.to_string());
                Some((p.id.clone(), format!(" · {}", p.name), model))
            })
            .collect();
        if targets.len() < 2 {
            return;
        }
        self.record_prompt(&item.text);
        self.clear_recall();
        let mut first_fork = None;
        for (pid, suffix, model) in targets {
            let before = self.chats.len();
            self.fork_inner(
                src_ix,
                ForkOpts {
                    at: None,
                    suffix,
                    bind: Some((pid, model)),
                    allow_empty: true,
                },
                window,
                cx,
            );
            // `fork_inner` selects its fork — a no-op (empty transcript
            // can't happen with allow_empty, but a vanished source can)
            // must not send the prompt to whatever chat is active.
            if self.chats.len() == before || self.active != self.chats.len() - 1 {
                continue;
            }
            first_fork.get_or_insert(self.active);
            self.send_text(item.clone(), window, cx);
        }
        if let Some(ix) = first_fork {
            self.select_chat(ix, window, cx);
        }
    }

    /// The shared fork body: copy the transcript through `opts.at` (`None`
    /// = the end) into a new chat titled `<src.title><opts.suffix>`, then
    /// select it. `opts.bind` overrides the provider/model stamps — `None`
    /// keeps the source's. The backend thread id never crosses: a fork
    /// always starts a fresh backend thread. An empty transcript no-ops
    /// unless `opts.allow_empty` — compare forks a fresh chat so its
    /// prompt lands as the first message.
    fn fork_inner(&mut self, chat_ix: usize, opts: ForkOpts, window: &mut Window, cx: &mut Context<Self>) {
        let Some(src) = self.chats.get(chat_ix) else { return };
        let end = opts.at.map_or(src.messages.len(), |ix| ix + 1);
        if end > src.messages.len() || (end == 0 && !opts.allow_empty) {
            return;
        }
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        // A worktree thread's fork needs its own worktree — sharing the
        // original's path unowned would break when either chat is deleted.
        // Resolved before `select_chat` so a failure note lands on the fork.
        let (workdir, worktree) = if src.worktree {
            match crate::worktree::create(&self.project, id) {
                Ok(dir) => (dir.to_string_lossy().into_owned(), true),
                Err(_) => (self.project.root().to_string_lossy().into_owned(), false),
            }
        } else {
            (src.workdir.clone(), false)
        };
        let mut fork = Chat::new(id, format!("{}{}", src.title, opts.suffix));
        fork.messages = Rc::new(src.messages[..end].to_vec());
        fork.title_generated = src.title_generated;
        fork.title_custom = src.title_custom;
        fork.folder = src.folder.clone();
        fork.color = src.color;
        if let Some((provider, model)) = opts.bind {
            fork.provider = provider;
            fork.model = model;
        } else {
            fork.provider = src.provider.clone();
            fork.model = src.model.clone();
        }
        fork.access = src.access;
        fork.effort = src.effort.clone();
        fork.workdir = workdir;
        fork.worktree = worktree;
        // A temporary chat's fork stays temporary — forking must not
        // silently persist content the user marked ephemeral.
        fork.ephemeral = src.ephemeral;
        // Checkpoints pinned to retained messages still resolve — a git
        // checkpoint's commit-tree sha is reachable from any worktree of
        // the repo, and copy snapshots live in the shared project store.
        fork.checkpoints = src.checkpoints.iter().filter(|c| c.ix < end).cloned().collect();
        // Feedback notes are pinned to message timestamps — keep only the
        // ones whose message made it into the fork.
        fork.feedback = src.feedback.iter().filter(|n| fork.messages.iter().any(|m| m.at == n.at)).cloned().collect();
        self.chats.push(fork);
        let new_ix = self.chats.len() - 1;
        self.select_chat(new_ix, window, cx);
    }
}
