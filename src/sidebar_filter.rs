//! Sidebar filter chips — the toggle row under the chat-list search field.
//! `SidebarFilter` is one chip's predicate; `SidebarFilters` is the active
//! set the workspace carries. Chips AND together and combine with the title
//! query: a chat shows only when it passes every active chip and its title
//! contains the query. Session-scoped — nothing here persists.

use gpui_kit::assets::IconName;

use crate::model::Chat;
use crate::workspace::Workspace;

/// One sidebar filter chip — the predicate a chat must satisfy while the
/// chip is on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SidebarFilter {
    /// A reply turn is streaming in this chat.
    Running,
    /// The chat has replies the user hasn't seen.
    Unread,
    /// The transcript holds a plan checklist (`update_plan` card).
    HasPlan,
}

impl SidebarFilter {
    /// Chip order in the filter row.
    pub const ALL: [Self; 3] = [Self::Running, Self::Unread, Self::HasPlan];

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Unread => "Unread",
            Self::HasPlan => "Has plan",
        }
    }

    /// Mirrors the row's own indicators: the streaming spinner, the unread
    /// dot, and the plan panel's checklist icon.
    pub fn icon(self) -> IconName {
        match self {
            Self::Running => IconName::LoaderCircle,
            Self::Unread => IconName::CircleDot,
            Self::HasPlan => IconName::ListTodo,
        }
    }

    /// Element id of the chip — keeps it findable in tests.
    pub fn id(self) -> &'static str {
        match self {
            Self::Running => "sidebar-filter-running",
            Self::Unread => "sidebar-filter-unread",
            Self::HasPlan => "sidebar-filter-has-plan",
        }
    }

    /// Whether `chat` satisfies this chip alone.
    pub fn matches(self, chat: &Chat) -> bool {
        match self {
            Self::Running => chat.running,
            Self::Unread => chat.unread,
            Self::HasPlan => chat.latest_plan().is_some(),
        }
    }
}

/// The sidebar's active chip set — runtime state on `Workspace`, reset on
/// launch. An empty set filters nothing.
#[derive(Default)]
pub struct SidebarFilters {
    active: std::collections::HashSet<SidebarFilter>,
}

impl SidebarFilters {
    /// Flip one chip on/off.
    pub fn toggle(&mut self, filter: SidebarFilter) {
        if !self.active.remove(&filter) {
            self.active.insert(filter);
        }
    }

    /// Whether `filter`'s chip is on.
    pub fn is_active(&self, filter: SidebarFilter) -> bool {
        self.active.contains(&filter)
    }

    /// Whether any chip is on — gates the "N of M" count.
    pub fn any(&self) -> bool {
        !self.active.is_empty()
    }

    /// Whether `chat` passes every active chip — the chips AND together.
    pub fn matches(&self, chat: &Chat) -> bool {
        self.active.iter().all(|f| f.matches(chat))
    }

    /// The "N of M" label shown while chips are on — `shown` is the listed
    /// rows (live + archived), `total` every chat in the workspace.
    pub fn count_text(&self, shown: usize, total: usize) -> Option<String> {
        self.any().then(|| format!("{shown} of {total}"))
    }
}

impl Workspace {
    /// `sidebar_order` narrowed by the active filter chips — the chat list
    /// the sidebar renders and Cmd+1..9 resolves against. The Archived
    /// section applies `SidebarFilters::matches` directly in render.
    pub(crate) fn sidebar_visible(&self, query: &str) -> Vec<usize> {
        self.sidebar_order(query)
            .into_iter()
            .filter(|ix| self.sidebar_filters.matches(&self.chats[*ix]))
            .collect()
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "sidebar_filter_tests.rs"]
mod sidebar_filter_tests;
