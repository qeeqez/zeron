//! Nav-rail content for the settings screen, rendered inside the shared
//! sidebar column (`sidebar-wrap`) when settings is open — so there's one
//! sidebar, not two. Rows go through the same `NavRow` component as the chat
//! list, inside the same `Sidebar`/`SidebarGroup` chrome, so padding, colors,
//! hover and selected states are identical on both screens. Split from
//! `settings.rs`/`settings_sections.rs` to stay under the 250-SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarGroup, SidebarItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::nav_row::NavRow;
use crate::views::settings::SettingsPanel;

/// "Back to app" row at the top of the nav — closes settings.
fn back_row(ws: &WeakEntity<crate::workspace::Workspace>) -> NavRow {
    let ws = ws.clone();
    NavRow::new("settings-back", "Back to app").icon(IconName::ArrowLeft).on_click(move |_, _, cx| {
        let _ = ws.update(cx, |this, cx| this.close_settings(cx));
    })
}

/// Small X in the settings content header — the only pointer close when the
/// sidebar is collapsed (the nav rail with `back_row` isn't rendered then).
/// `stop_propagation` keeps the press off the titlebar drag strip.
pub fn close_button(ws: &WeakEntity<crate::workspace::Workspace>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let ws = ws.clone();
    div()
        .id("settings-close")
        .test_support()
        .cursor_pointer()
        .text_color(theme.muted_foreground)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(IconName::X)
        .on_click(move |_, _, cx| {
            let _ = ws.update(cx, |this, cx| this.close_settings(cx));
        })
}

/// A nav row for one section — icon + label, highlights when selected.
fn nav_item(section: Section, selected: bool, panel: &Entity<SettingsPanel>) -> NavRow {
    let panel = panel.clone();
    NavRow::new(SharedString::from(format!("settings-nav-{}", section.name())), section.label())
        .icon(section.icon())
        .active(selected)
        .on_click(move |_, _, cx| {
            panel.update(cx, |this, cx| {
                this.section = section;
                // Opening the MCP section kicks a live status fetch — the
                // dots refresh from `mcpServerStatus/list`.
                if section == Section::McpServers {
                    this.refresh_mcp_status(cx);
                }
                cx.notify();
            });
        })
}

/// The settings nav column for the shared sidebar — a real `Sidebar` with the
/// same header/search slot and grouped `NavRow` items as the chat list.
/// `width`/`collapsed` mirror the chat sidebar's geometry.
pub(crate) struct SettingsNav<'a> {
    pub panel: &'a Entity<SettingsPanel>,
    pub width: Pixels,
    pub collapsed: bool,
}

pub fn settings_nav(nav: SettingsNav<'_>, window: &mut Window, cx: &mut App) -> impl IntoElement {
    let SettingsNav { panel, width, collapsed } = nav;
    let (ws, search, section, query) = {
        let p = panel.read(cx);
        (p.ws.clone(), p.search.clone(), p.section, p.search.read(cx).value().to_lowercase())
    };

    // "Back to app" sits at the very top of the rail, above the full-width
    // search field — same header slot the chat sidebar uses.
    let header = div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(back_row(&ws).render("settings-back-wrap", window, cx))
        .child(div().w_full().child(Input::new(&search).prefix(IconName::Search).appearance(true).w_full()));

    let groups: Vec<SidebarGroup<NavRow>> = if query.is_empty() {
        Section::GROUPS
            .iter()
            .map(|(name, sections)| SidebarGroup::new(*name).children(sections.iter().map(|s| nav_item(*s, *s == section, panel))))
            .collect()
    } else {
        let matches = Section::ALL.iter().filter(|s| s.label().to_lowercase().contains(query.as_str()));
        vec![SidebarGroup::new("").children(matches.map(|s| nav_item(*s, *s == section, panel)))]
    };

    // The component paints its own opaque `tokens.sidebar` — clear it so the
    // wrap's fill (translucent when frosted) shows through, same as the chat
    // sidebar.
    Sidebar::new("settings-nav")
        .w(width)
        .collapsible(SidebarCollapsible::Offcanvas)
        .collapsed(collapsed)
        .bg(transparent_black())
        .header(header)
        .children(groups)
}

/// A nav-rail section. `name` feeds element ids; `label` is what renders.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    General,
    Instructions,
    Appearance,
    Voice,
    Profile,
    Project,
    Providers,
    Shortcuts,
    McpServers,
}

impl Section {
    pub(crate) const ALL: [Section; 9] = [
        Self::General,
        Self::Instructions,
        Self::Appearance,
        Self::Voice,
        Self::Profile,
        Self::Project,
        Self::Providers,
        Self::Shortcuts,
        Self::McpServers,
    ];
    /// (group header, sections) pairs for the rail — matches the Codex
    /// settings sidebar grouping.
    pub(crate) const GROUPS: [(&'static str, &'static [Section]); 4] = [
        ("Personal", &[Self::General, Self::Instructions, Self::Appearance, Self::Voice, Self::Profile]),
        ("Project", &[Self::Project]),
        ("Coding", &[Self::Providers, Self::Shortcuts]),
        ("Integrations", &[Self::McpServers]),
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Instructions => "instructions",
            Self::Appearance => "appearance",
            Self::Voice => "voice",
            Self::Profile => "profile",
            Self::Project => "project",
            Self::Shortcuts => "shortcuts",
            Self::McpServers => "mcp",
            Self::Providers => "providers",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Instructions => "Custom Instructions",
            Self::Appearance => "Appearance",
            Self::Voice => "Voice",
            Self::Profile => "Profile",
            Self::Project => "Project",
            Self::Shortcuts => "Shortcuts",
            Self::Providers => "Providers",
            Self::McpServers => "MCP Servers",
        }
    }
    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::General => IconName::SlidersHorizontal,
            Self::Instructions => IconName::ScrollText,
            Self::Appearance => IconName::Palette,
            Self::Voice => IconName::Mic,
            Self::Profile => IconName::CircleUser,
            Self::Project => IconName::FolderCog,
            Self::Shortcuts => IconName::Keyboard,
            Self::McpServers => IconName::PlugZap,
            Self::Providers => IconName::Layers,
        }
    }
}
