//! Diff review: pending comments collected from the Changes panel's diff
//! lines, the inline comment editor's state, and the send path that turns
//! them into a structured review message for the agent.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::model::{ReviewComment, ReviewTarget};
use crate::workspace::Workspace;

/// Pending diff-review state for the Changes panel: the collected comments,
/// the line the inline editor is anchored to, and the editor's input.
pub struct Review {
    /// Comments collected so far, kept sorted by `path` then `line` so the
    /// banner and the sent review read in file order.
    pub comments: Vec<ReviewComment>,
    /// The diff row the comment editor is open on — `None` when closed.
    pub target: Option<ReviewTarget>,
    /// The inline editor's text — seeded from the existing comment when the
    /// anchor already has one (click-to-edit).
    pub input: Entity<InputState>,
}

impl Review {
    /// Build the state and wire Enter in the comment input to commit.
    /// `cx.subscribe` (not `subscribe_in`) — commit doesn't need the window.
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Comment on this line…"));
        cx.subscribe(&input, |this: &mut Workspace, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_review_comment(cx);
            }
        })
        .detach();
        Self { comments: Vec::new(), target: None, input }
    }
}

impl Workspace {
    /// Index into `review.comments` for the comment anchored at `target`.
    pub(crate) fn review_comment_at(&self, target: ReviewTarget) -> Option<usize> {
        let anchor = crate::changes_diff::review_anchor(&self.changes, target)?;
        self.review
            .comments
            .iter()
            .position(|c| c.path == anchor.path && c.line == anchor.line && c.old_side == anchor.old_side)
    }

    /// The diff row `comment` is anchored to — `None` when its file's diff
    /// isn't expanded or the line no longer resolves, so the banner's
    /// click-to-edit only mounts while the target is on screen.
    pub(crate) fn review_target_for(&self, comment: &ReviewComment) -> Option<ReviewTarget> {
        let file_ix = self.changes.iter().position(|c| c.path == comment.path)?;
        let line_ix = self.changes[file_ix]
            .diff
            .as_ref()?
            .lines
            .iter()
            .position(|l| if comment.old_side { l.old == Some(comment.line) } else { l.new == Some(comment.line) })?;
        Some(ReviewTarget { file_ix, line_ix })
    }

    /// Open (or toggle closed) the inline comment editor on diff line
    /// `line_ix` of change `file_ix`. Lines without a line number (hunk
    /// headers) aren't commentable — the click is ignored. Opening a line
    /// that already has a comment seeds the editor with its text.
    pub fn open_review_comment(&mut self, file_ix: usize, line_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let target = ReviewTarget { file_ix, line_ix };
        if self.review.target == Some(target) {
            self.review.target = None;
            cx.notify();
            return;
        }
        if crate::changes_diff::review_anchor(&self.changes, target).is_none() {
            return;
        }
        let text = self.review_comment_at(target).map_or("", |ix| self.review.comments[ix].text.as_str()).to_string();
        self.review.target = Some(target);
        self.review.input.update(cx, |s, cx| s.set_value(text, window, cx));
        // The editor only exists after this render — focus it next frame.
        let input = self.review.input.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.notify();
    }

    /// Commit the editor's text to its anchored line: a non-empty value
    /// updates the existing comment or inserts a new one in sorted order;
    /// an empty value just closes the editor (removal is the banner's ✕).
    pub fn commit_review_comment(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.review.target.take() else { return };
        let text = self.review.input.read(cx).value().trim().to_string();
        if text.is_empty() {
            cx.notify();
            return;
        }
        let Some(mut comment) = crate::changes_diff::review_anchor(&self.changes, target) else { return };
        comment.text = text;
        match self.review_comment_at(target) {
            Some(ix) => self.review.comments[ix] = comment,
            None => {
                let at = self
                    .review
                    .comments
                    .iter()
                    .position(|c| (c.path.as_str(), c.line, c.old_side) > (comment.path.as_str(), comment.line, comment.old_side))
                    .unwrap_or(self.review.comments.len());
                self.review.comments.insert(at, comment);
            },
        }
        cx.notify();
    }

    /// Close the comment editor without changing the review.
    pub fn cancel_review_comment(&mut self, cx: &mut Context<Self>) {
        if self.review.target.take().is_some() {
            cx.notify();
        }
    }

    /// Drop comment `ix` from the pending review.
    pub fn remove_review_comment(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.review.comments.len() {
            self.review.comments.remove(ix);
            cx.notify();
        }
    }

    /// Format the pending comments as a structured review message — one
    /// `path:line` entry per comment with the code line quoted for context.
    pub(crate) fn format_review(comments: &[ReviewComment]) -> String {
        let mut out = String::from("Review comments on the working tree:\n");
        for c in comments {
            let side = if c.old_side { " (removed line)" } else { "" };
            out.push_str(&format!("\n- {}:{}{}", c.path, c.line, side));
            if !c.code.trim().is_empty() {
                out.push_str(&format!("\n  > {}", c.code.trim()));
            }
            out.push_str(&format!("\n  {}", c.text));
        }
        out
    }

    /// Send the pending review to the agent through the normal send path —
    /// queued behind a running turn exactly like a typed message. An empty
    /// review is a no-op so the button can't fire a bare header.
    pub fn send_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review.comments.is_empty() {
            return;
        }
        let text = Self::format_review(&self.review.comments);
        self.review.comments.clear();
        self.review.target = None;
        self.send_or_queue(&text, window, cx);
    }
}
