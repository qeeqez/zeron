//! Fork-here: branch the conversation at a message without editing it.
//! Unlike `chat_edit` (truncate + resend), forking is non-destructive —
//! the original chat keeps its full transcript and a NEW chat opens with
//! the messages up to and including the fork point, ready for a different
//! follow-up.

#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui_kit::*;

use crate::model::Chat;
use crate::workspace::Workspace;

impl Workspace {
    /// Branch chat `chat_ix` at message `msg_ix`: a new chat titled
    /// "<title> (fork)" opens holding the messages up to and including
    /// `msg_ix`. The original is untouched — unlike `commit_edit`, nothing
    /// is truncated or resent. `None` forks at the end of the transcript.
    /// The fork carries the thread's provider/model/access stamps so its
    /// follow-up turn runs on the same configuration; a worktree thread's
    /// fork gets its own worktree (sharing the original's would break when
    /// either chat is deleted). The backend thread id is NOT copied — the
    /// fork starts a fresh backend thread, like `duplicate_chat`.
    pub fn fork_chat(&mut self, chat_ix: usize, msg_ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(src) = self.chats.get(chat_ix) else { return };
        let ix = msg_ix.unwrap_or_else(|| src.messages.len().saturating_sub(1));
        if ix >= src.messages.len() {
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
        let mut fork = Chat::new(id, format!("{} (fork)", src.title));
        fork.messages = Rc::new(src.messages[..=ix].to_vec());
        fork.folder = src.folder.clone();
        fork.color = src.color;
        fork.provider = src.provider.clone();
        fork.model = src.model.clone();
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
        fork.checkpoints = src.checkpoints.iter().filter(|c| c.ix <= ix).cloned().collect();
        // Feedback notes are pinned to message timestamps — keep only the
        // ones whose message made it into the fork.
        fork.feedback = src.feedback.iter().filter(|n| fork.messages.iter().any(|m| m.at == n.at)).cloned().collect();
        self.chats.push(fork);
        let new_ix = self.chats.len() - 1;
        self.select_chat(new_ix, window, cx);
    }
}
