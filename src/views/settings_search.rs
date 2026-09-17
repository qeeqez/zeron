//! Search inside Settings: the nav-rail field's query filters the section
//! list to sections containing a matching setting, marks the matching rows
//! inside the open section with a translucent wash, and scrolls the first
//! match into view — the macOS System Settings / Codex behavior.
//!
//! Matching runs over a static index of the labels each section passes to
//! `toggle_row`/`default_row`/`group_label` (plus shortcut descriptions,
//! which live in `SHORTCUT_SPECS` rather than literals). The index is the
//! single place a new row must be registered; `row_labels` returning a slice
//! per `Section` keeps the mapping exhaustive by construction.

use std::cell::Cell;
use std::sync::LazyLock;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::shortcuts::{SHORTCUT_SPECS, ShortcutGroup};
use crate::views::settings_nav::Section;

/// Shortcut rows render `spec.description` under `group.label()` headers —
/// `&'static str`s inside non-const tables, so both are materialized once
/// here to keep `row_labels` a plain slice lookup.
static SHORTCUT_LABELS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    ShortcutGroup::ALL
        .iter()
        .map(|g| g.label())
        .chain(SHORTCUT_SPECS.iter().map(|s| s.description))
        .collect()
});

/// The row labels a section renders — group headers, toggle/select/input
/// labels and shortcut descriptions. Dynamic rows (provider instances, MCP
/// servers, accounts, worktrees) have no static label and aren't indexed.
pub(crate) fn row_labels(section: Section) -> &'static [&'static str] {
    match section {
        Section::General => &[
            "Thread defaults",
            "Model",
            "Permissions",
            "Workspace",
            "Files",
            "Editor",
            "Notifications",
            "Notify on reply complete",
            "Notification sound",
            "Notify on background replies",
            "Agent access",
            "Messages",
            "Word wrap",
            "Usage",
            "Budget alert",
            "System",
            "Global hotkey",
            "Summon shortcut",
        ],
        Section::Instructions => &["Global instructions", "Project instructions"],
        Section::Appearance => &[
            "Theme",
            "System",
            "Light",
            "Dark",
            "Interface",
            "System font",
            "Code",
            "Default mono font",
            "Contrast",
            "Sidebar",
            "Frosted glass sidebar",
        ],
        Section::Voice => &[
            "Dictation",
            "Enable dictation",
            "Language",
            "On-device recognition",
            "Microphone",
            "Test microphone",
        ],
        Section::Profile => &["Accounts", "Sign out all", "About", "Version", "Updates", "Data directory"],
        Section::Project => &["Setup script", "Thread worktrees"],
        Section::Providers => &[
            "Model providers", "Display name", "Command", "Endpoint URL", "API key env var", "Base URL", "Account", "Variables", "Models",
        ],
        Section::Shortcuts => SHORTCUT_LABELS.as_slice(),
        Section::McpServers => &["Configured servers"],
    }
}

/// `label` contains `query` — both already lowercase.
pub(crate) fn matches(query: &str, label: &str) -> bool {
    label.to_lowercase().contains(query)
}

/// Sections whose own label or any indexed row label contains `query`
/// (already lowercase). An empty query matches everything — callers keep
/// the grouped rail for it, so this is only a unit-test convenience.
pub(crate) fn matching_sections(query: &str) -> Vec<Section> {
    if query.is_empty() {
        return Section::ALL.to_vec();
    }
    Section::ALL
        .iter()
        .copied()
        .filter(|s| matches(query, s.label()) || row_labels(*s).iter().any(|l| matches(query, l)))
        .collect()
}

/// Per-render search state handed to section bodies via `SettingsView`:
/// the normalized query, the wash painted behind matching rows, and the
/// scroll anchor the FIRST matching row claims so `SettingsPanel::render`
/// can scroll it into view. An empty query makes `wrap` a pass-through —
/// the empty state renders exactly what shipped before search.
pub(crate) struct SearchCtx {
    /// Lowercase, trimmed query — empty means "not searching".
    pub query: String,
    /// Anchored to the content pane's `ScrollHandle`; the first matching
    /// row attaches it so `scroll_to` can bring that row to the top.
    anchor: ScrollAnchor,
    /// Set once the first match claims the anchor — later matches keep
    /// only the wash, so the scroll lands on the first hit.
    pub anchor_taken: Cell<bool>,
    /// `theme().selection` — the same wash the chat find bar paints.
    wash: Hsla,
}

impl SearchCtx {
    pub(crate) fn new(query: String, anchor: ScrollAnchor, cx: &App) -> Self {
        Self {
            query,
            anchor,
            anchor_taken: Cell::new(false),
            wash: cx.theme().selection,
        }
    }

    /// Wrap a rendered row in the match wash when `label` contains the
    /// query. The first match also claims the scroll anchor; the id +
    /// `aria_label` let headless tests see the mark.
    pub(crate) fn wrap(&self, label: &'static str, el: impl IntoElement) -> AnyElement {
        if self.query.is_empty() || !matches(&self.query, label) {
            return el.into_any_element();
        }
        let first = !self.anchor_taken.replace(true);
        div()
            .id(SharedString::from(format!("settings-search-hit-{label}")))
            .test_support()
            .aria_label("settings search match")
            .when(first, |d| d.anchor_scroll(Some(self.anchor.clone())))
            .rounded_md()
            .bg(self.wash)
            .child(el)
            .into_any_element()
    }
}
