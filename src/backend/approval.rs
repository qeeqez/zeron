//! Approval-prompt types shared by every backend: the user's decision,
//! what kind of action is gated, and how a backend routes the request.

/// The user's answer to an approval prompt. Backends map this onto their
/// protocol's wire value (codex `ReviewDecision`, ACP option kinds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ApprovalDecision {
    /// Allow this one action.
    Approve,
    /// Deny this action — the tool call aborts, the turn continues.
    Deny,
    /// Allow this and every equivalent action for the rest of the turn
    /// (codex `approved_for_session`, ACP `allow_always`).
    ApproveForSession,
}

impl ApprovalDecision {
    /// Human-readable outcome for transcripts and the answered card.
    pub fn label(self) -> &'static str {
        match self {
            ApprovalDecision::Approve => "Approved",
            ApprovalDecision::Deny => "Denied",
            ApprovalDecision::ApproveForSession => "Always allowed",
        }
    }
}

/// What kind of action an approval prompt gates — drives the card's icon
/// and title.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ApprovalKind {
    /// A shell command (codex `commandExecution`/`execCommandApproval`).
    Command,
    /// A file patch (codex `fileChange`/`applyPatchApproval`).
    Patch,
    /// ACP `session/request_permission` — the tool call's own title is
    /// the detail.
    Permission,
}

impl ApprovalKind {
    /// Card title for this kind of prompt.
    pub fn label(self) -> &'static str {
        match self {
            ApprovalKind::Command => "Run command",
            ApprovalKind::Patch => "Apply patch",
            ApprovalKind::Permission => "Permission request",
        }
    }
}

/// Channel back to the backend thread blocked on an approval request.
/// One answer per prompt — the UI takes it out of the card when clicked.
pub type ApprovalResponder = std::sync::mpsc::Sender<ApprovalDecision>;

/// How a backend answers an approval request: surface it to the user as
/// an `ApprovalRequest` event, or reply immediately with a fixed decision.
#[derive(Clone, Copy, Debug)]
pub enum ApprovalRoute {
    /// Emit `AgentEvent::ApprovalRequest` and block until the UI answers.
    Ask,
    /// Answer on the spot — auto modes approve, read-only modes deny.
    Auto(ApprovalDecision),
}

/// A pending (or answered) approval prompt — the card the backend's
/// `ApprovalRequest` event becomes. `respond` is the channel back to the
/// blocked backend thread; it's `Some` only while the request is live, so
/// a persisted card reloads with `None` and renders as expired.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ApprovalCard {
    /// Hash of the backend's request/item id — matches tool-card keys.
    pub request_ix: usize,
    pub kind: ApprovalKind,
    /// What will run — the command line, patch summary, or tool title.
    pub detail: gpui_kit::SharedString,
    /// The user's choice once clicked; `None` while undecided.
    #[serde(default)]
    pub decision: Option<ApprovalDecision>,
    /// Channel to the waiting backend — skipped on disk; `None` means the
    /// prompt is no longer answerable (answered, stopped, or reloaded).
    #[serde(skip)]
    pub respond: Option<ApprovalResponder>,
}
