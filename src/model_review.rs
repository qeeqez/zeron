//! Review-comment types — split from `model.rs` for the SLOC cap.

/// One pending review comment on a diff line — collected in the Changes
/// panel and sent to the agent as a structured review message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewComment {
    /// Project-relative file path the comment is anchored to.
    pub path: String,
    /// Line number in the file — the new side; removed lines aren't
    /// commentable, so every anchor counts on the new side.
    pub line: u32,
    /// `line` counts on the old side — part of the anchor's identity; only
    /// new-side lines are commentable, so this is always `false` today.
    pub old_side: bool,
    /// The diff line's content, quoted in the review for context.
    pub code: String,
    /// The reviewer's comment text.
    pub text: String,
}

/// The diff row the comment editor is anchored to: `file_ix` indexes
/// `Workspace::changes`, `line_ix` indexes that row's `FileDiff::lines`.
/// Indices (not line numbers) so the editor tracks its row across re-renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewTarget {
    pub file_ix: usize,
    pub line_ix: usize,
}
