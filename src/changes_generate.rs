//! AI commit messages: the Changes panel's ✦ button collects the staged
//! diff (`git diff --cached`, byte-capped) and asks the active backend for a
//! conventional-commit subject — a one-off turn that never touches a chat.
//! The reply fills the commit box so the user can edit before committing;
//! failures land as the panel's status note.

use gpui_kit::*;

use crate::backend::{AgentEvent, ApprovalDecision, ReplyStream, TurnContext};
use crate::workspace::Workspace;

/// Most staged-diff bytes sent to the backend — enough for a real change
/// set, small enough to keep the one-off turn cheap.
const MAX_DIFF_BYTES: u64 = 64 * 1024;

/// What a generation run produced: `Ok` is the message for the commit box;
/// `Err` is the note text plus its error flag (an empty diff isn't a
/// failure, a backend error is).
type GenerateResult = Result<String, (String, bool)>;

impl Workspace {
    /// Generate a commit message from the staged diff. Refused while a git
    /// op or an earlier generation is in flight — the box would be filled
    /// under a commit that already ran.
    pub fn generate_commit_message(&mut self, cx: &mut Context<Self>) {
        if self.git.busy || self.git.generating {
            return;
        }
        if let Some(reason) = self.auth_block_note() {
            self.git.note = Some((reason, true));
            cx.notify();
            return;
        }
        if self.model.is_empty() {
            self.git.note = Some(("Select a model first — the selected provider has no models.".to_string(), true));
            cx.notify();
            return;
        }
        self.git.generating = true;
        self.git.note = None;
        let dir = self.project.root().to_path_buf();
        let backend = self.backend.clone();
        let model = self.model.to_string();
        // Read-only like a task agent — the turn only writes a message.
        let mut ctx = TurnContext::at(dir.clone(), self.access);
        ctx.instructions = crate::instructions::for_turn(&self.instructions, self.project.root());
        cx.spawn(async move |this, cx| {
            let staged = cx.background_executor().spawn(async move { staged_diff(&dir) }).await;
            let result = match staged {
                Err(note) => Err((note, false)),
                Ok((diff, capped)) => {
                    let stream = backend.send(&prompt_for(&diff, capped), &model, "Ask", &ctx);
                    collect_message(stream, cx).await
                },
            };
            let _ = this.update_in(cx, |this, window, cx| this.land_commit_message(result, window, cx));
        })
        .detach();
        cx.notify();
    }

    /// Publish a generation result: a message fills the commit box (left
    /// editable — the user reviews before committing), a note lands under
    /// the buttons.
    fn land_commit_message(&mut self, result: GenerateResult, window: &mut Window, cx: &mut Context<Self>) {
        self.git.generating = false;
        match result {
            Ok(message) => self.git.commit_input.update(cx, |s, cx| s.set_value(message, window, cx)),
            Err(note) => self.git.note = Some(note),
        }
        cx.notify();
    }
}

/// `git diff --cached` under `dir`, capped at `MAX_DIFF_BYTES`. `Err` when
/// git can't run or nothing is staged — both surface as a note, not an
/// error.
fn staged_diff(dir: &std::path::Path) -> Result<(String, bool), String> {
    let Some((diff, capped)) = crate::git::git_diff(dir, &["diff", "--cached"], MAX_DIFF_BYTES) else {
        return Err("Couldn't read the staged diff — is this a git repository?".to_string());
    };
    if diff.trim().is_empty() {
        return Err("Nothing staged — stage changes first.".to_string());
    }
    Ok((diff, capped))
}

/// The one-off prompt: the staged diff plus the output contract — one
/// conventional-commit subject, nothing else.
fn prompt_for(diff: &str, capped: bool) -> String {
    let mut prompt = String::from(
        "Write a commit message for the staged changes below.\n\
         Reply with only the message: a single conventional-commit subject line \
         (type(scope): summary), imperative mood, at most 72 characters. \
         No body, no code fence, no explanation.\n\n",
    );
    if capped {
        prompt.push_str("The diff is truncated — summarize the visible changes.\n\n");
    }
    prompt.push_str(diff);
    prompt
}

/// Drain a one-off turn's stream into its reply text. Polls like the chat
/// pump — a blocking `recv` would wedge the test scheduler. Approvals are
/// auto-denied: this turn only writes text, and a pending request would
/// otherwise hang the drain.
async fn collect_message(stream: ReplyStream, cx: &mut AsyncApp) -> GenerateResult {
    let mut text = String::new();
    let mut error = None;
    'outer: loop {
        loop {
            match stream.events.try_recv() {
                Ok(AgentEvent::TextDelta(delta)) => text.push_str(&delta),
                Ok(AgentEvent::Error(e)) => {
                    error = Some(e.to_string());
                    break 'outer;
                },
                Ok(AgentEvent::Done) => break 'outer,
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
    if let Some(e) = error {
        return Err((e, true));
    }
    match clean_subject(&text) {
        Some(subject) => Ok(subject),
        None => Err(("The backend returned no message.".to_string(), true)),
    }
}

/// First non-empty line of the reply, stripped of the markdown decoration
/// models like to add — bullets, quotes, fences, emphasis. `None` when the
/// reply carried no usable text.
fn clean_subject(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches(['-', '*', '>', '#', ' ']).trim();
    let line = line.trim_matches(['`', '"', '*']);
    (!line.is_empty()).then(|| line.to_string())
}

#[cfg(test)]
#[path = "changes_generate_tests.rs"]
mod tests;
