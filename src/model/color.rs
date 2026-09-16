//! Chat color tags — split from `model.rs` for the SLOC cap. Reached as
//! `crate::model::ChatColor` via the re-export there.

/// A user-picked color tag for a chat — shown as a dot on the sidebar row
/// and the chat titlebar for visual grouping. `None` means untagged.
/// Persisted by name (see `StoredChat::color`); the hues are fixed so the
/// tag reads the same in every theme — mid-lightness, high-saturation
/// swatches stay distinguishable on both light and dark backgrounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ChatColor {
    /// Menu order — the order the ⋯ menu's Color submenu lists them.
    pub const ALL: [ChatColor; 6] = [
        ChatColor::Red,
        ChatColor::Orange,
        ChatColor::Yellow,
        ChatColor::Green,
        ChatColor::Blue,
        ChatColor::Purple,
    ];

    /// Stable id persisted in the chat file.
    pub fn name(self) -> &'static str {
        match self {
            ChatColor::Red => "red",
            ChatColor::Orange => "orange",
            ChatColor::Yellow => "yellow",
            ChatColor::Green => "green",
            ChatColor::Blue => "blue",
            ChatColor::Purple => "purple",
        }
    }

    /// Label shown in the Color submenu.
    pub fn label(self) -> &'static str {
        match self {
            ChatColor::Red => "Red",
            ChatColor::Orange => "Orange",
            ChatColor::Yellow => "Yellow",
            ChatColor::Green => "Green",
            ChatColor::Blue => "Blue",
            ChatColor::Purple => "Purple",
        }
    }

    /// Parse a persisted name; unknown names (a tag from a newer build)
    /// read as untagged rather than dropping the chat.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.name() == name)
    }

    /// The swatch color — fixed hues so a tag stays distinguishable in both
    /// light and dark themes (theme tokens shift with the palette).
    pub fn hsla(self) -> gpui_kit::Hsla {
        match self {
            ChatColor::Red => gpui_kit::hsla(0.0, 0.72, 0.55, 1.0),
            ChatColor::Orange => gpui_kit::hsla(0.07, 0.85, 0.55, 1.0),
            ChatColor::Yellow => gpui_kit::hsla(0.13, 0.90, 0.45, 1.0),
            ChatColor::Green => gpui_kit::hsla(0.38, 0.60, 0.45, 1.0),
            ChatColor::Blue => gpui_kit::hsla(0.58, 0.80, 0.55, 1.0),
            ChatColor::Purple => gpui_kit::hsla(0.75, 0.60, 0.60, 1.0),
        }
    }
}
