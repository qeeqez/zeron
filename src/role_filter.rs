//! `RoleFilter` — the All / You / Assistant picker shared by the in-chat
//! find bar and the global-search dialog. `All` is the no-constraint
//! default; the other two keep only messages of that role.

use crate::model::Role;

/// Which messages a search may match: every role, or only the user's /
/// the assistant's. The find bar cycles through the states; global search
/// picks one from its Role chip's menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RoleFilter {
    /// No role constraint — the default.
    #[default]
    All,
    User,
    Assistant,
}

impl RoleFilter {
    /// The find-bar toggle's label — also the Role chip's menu items.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::User => "You",
            Self::Assistant => "Assistant",
        }
    }

    /// The next state on the find-bar toggle: All → You → Assistant → All.
    pub(crate) fn cycle(self) -> Self {
        match self {
            Self::All => Self::User,
            Self::User => Self::Assistant,
            Self::Assistant => Self::All,
        }
    }

    /// Whether a message of `role` survives the filter.
    pub(crate) fn matches(self, role: Role) -> bool {
        match self {
            Self::All => true,
            Self::User => role == Role::User,
            Self::Assistant => role == Role::Assistant,
        }
    }
}
