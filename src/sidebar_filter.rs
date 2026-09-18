//! Sidebar filter chips — the toggle row under the chat-list search field.
//! `SidebarFilter` is one chip's predicate; `SidebarFilters` is the active
//! set the workspace carries. Chips AND together and combine with the title
//! query: a chat shows only when it passes every active chip and its title
//! contains the query. Session-scoped — nothing here persists.

use gpui_kit::assets::IconName;

use crate::model::{Agent, AgentStatus, Chat};
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

    /// Whether `chat` satisfies this chip alone. `Running` reads the
    /// `chat_working` aggregate (reply streaming OR an attributed agent
    /// still running) so the chip agrees with the row's spinner. `HasPlan`
    /// also consults `pending_has_plan` — the pending-bookmark scan sets
    /// it so unopened chats aren't invisible to the chip.
    pub fn matches(self, chat: &Chat, agents: &[Agent]) -> bool {
        match self {
            Self::Running => chat_working(chat, agents),
            Self::Unread => chat.unread,
            Self::HasPlan => chat.latest_plan().is_some() || (chat.pending_load.is_some() && chat.pending_has_plan),
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
    pub fn matches(&self, chat: &Chat, agents: &[Agent]) -> bool {
        self.active.iter().all(|f| f.matches(chat, agents))
    }

    /// The "N of M" label shown while chips are on — `shown` is the listed
    /// rows (live + archived), `total` every chat in the workspace.
    pub fn count_text(&self, shown: usize, total: usize) -> Option<String> {
        self.any().then(|| format!("{shown} of {total}"))
    }
}

/// The sidebar's "working" aggregate: a chat counts as busy while its own
/// reply streams (`chat.running`) OR an agent attributed to it
/// (`Agent.chat_id`) is still Running — a turn can end while its subagents
/// keep going. Deliberately separate from `Chat.running`: that flag also
/// drives the reply footer, scroller remeasure and the save path's
/// foreign-turn semantics, none of which subagents should trip.
pub(crate) fn chat_working(chat: &Chat, agents: &[Agent]) -> bool {
    chat.running || agents.iter().any(|a| a.chat_id == Some(chat.id) && a.status == AgentStatus::Running)
}

impl Workspace {
    /// `chat_working` over this workspace's agents — the row's spinner and
    /// the Running chip read the same aggregate so they can't disagree.
    pub(crate) fn chat_working(&self, chat: &Chat) -> bool {
        chat_working(chat, &self.agents)
    }

    /// `sidebar_order` narrowed by the active filter chips — the chat list
    /// the sidebar renders and Cmd+1..9 resolves against. The Archived
    /// section applies `SidebarFilters::matches` directly in render.
    pub(crate) fn sidebar_visible(&self, query: &str) -> Vec<usize> {
        self.sidebar_order(query)
            .into_iter()
            .filter(|ix| self.sidebar_filters.matches(&self.chats[*ix], &self.agents))
            .collect()
    }

    /// Working chats — the Running chip's count, and the gate for the
    /// palette command and sidebar stop-all bar (both appear only at 2+).
    /// `running_agents` is the agent-row analog.
    pub(crate) fn running_chats(&self) -> usize {
        self.chats.iter().filter(|c| self.chat_working(c)).count()
    }

    /// The vec index `chat_cycle` selects next: `active`'s position in the
    /// visible list stepped forward or back, wrapping at both ends. An
    /// active chat outside the visible set (filtered out) starts from the
    /// nearest edge. `None` when fewer than 2 chats are visible.
    pub(crate) fn cycle_target(&self, forward: bool, cx: &gpui_kit::App) -> Option<usize> {
        let query = self.search.read(cx).value().to_lowercase();
        let vis = self.sidebar_visible(&query);
        if vis.len() < 2 {
            return None;
        }
        Some(match vis.iter().position(|&ix| ix == self.active) {
            Some(p) if forward => vis[(p + 1) % vis.len()],
            Some(p) => vis[(p + vis.len() - 1) % vis.len()],
            None if forward => vis[0],
            None => *vis.last().unwrap(),
        })
    }
}

/// Cmd+Shift+] / [ — cycle through the visible chat list in sidebar order
/// (same resolution as Cmd+1..9: filter chips and the query box apply).
/// The index math lives in `Workspace::cycle_target` — testable headless.
pub(crate) fn chat_cycle<A: gpui_kit::Action + CycleDir>(
    cx: &mut gpui_kit::Context<Workspace>,
) -> impl Fn(&A, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static {
    let ws = cx.entity();
    move |_: &A, window, cx| {
        ws.update(cx, |this, cx| {
            if let Some(next) = this.cycle_target(A::FWD, cx) {
                this.select_chat(next, window, cx);
            }
        });
    }
}

/// Which way `chat_cycle` steps — carried on the action type like
/// `ChatIx` carries a position.
pub(crate) trait CycleDir {
    const FWD: bool;
}
impl CycleDir for crate::NextChat {
    const FWD: bool = true;
}
impl CycleDir for crate::PrevChat {
    const FWD: bool = false;
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "sidebar_filter_tests.rs"]
mod sidebar_filter_tests;
