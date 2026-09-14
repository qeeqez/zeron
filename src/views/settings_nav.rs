//! Nav-rail content for the settings screen, rendered inside the shared
//! sidebar column (`sidebar-wrap`) when settings is open — so there's one
//! sidebar, not two. Rows go through the same `NavRow` component as the chat
//! list, inside the same `Sidebar`/`SidebarGroup` chrome, so padding, colors,
//! hover and selected states are identical on both screens. Split from
//! `settings.rs`/`settings_sections.rs` to stay under the 250-SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarGroup};
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
                cx.notify();
            });
        })
}

/// The settings nav column for the shared sidebar — a real `Sidebar` with the
/// same header/search slot and grouped `NavRow` items as the chat list.
/// `width`/`collapsed` mirror the chat sidebar's geometry.
pub fn settings_nav(panel: &Entity<SettingsPanel>, width: Pixels, collapsed: bool, cx: &App) -> impl IntoElement {
    let (ws, search, section, query) = {
        let p = panel.read(cx);
        (p.ws.clone(), p.search.clone(), p.section, p.search.read(cx).value().to_lowercase())
    };

    let header = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().flex().items_center().gap_2().text_sm().font_bold().child(IconName::Settings).child("Settings"))
        .child(Input::new(&search).prefix(IconName::Search).appearance(true));

    let mut groups: Vec<SidebarGroup<NavRow>> = vec![SidebarGroup::new("").child(back_row(&ws))];
    if query.is_empty() {
        groups.extend(Section::GROUPS.iter().map(|(name, sections)| {
            SidebarGroup::new(*name).children(sections.iter().map(|s| nav_item(*s, *s == section, panel)))
        }));
    } else {
        let matches = Section::ALL.iter().filter(|s| s.label().to_lowercase().contains(query.as_str()));
        groups.push(SidebarGroup::new("").children(matches.map(|s| nav_item(*s, *s == section, panel))));
    }

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
    Appearance,
    Voice,
    Profile,
    Shortcuts,
    McpServers,
}

impl Section {
    pub(crate) const ALL: [Section; 6] = [Self::General, Self::Appearance, Self::Voice, Self::Profile, Self::Shortcuts, Self::McpServers];
    /// (group header, sections) pairs for the rail — matches the Codex
    /// settings sidebar grouping.
    pub(crate) const GROUPS: [(&'static str, &'static [Section]); 3] = [
        ("Personal", &[Self::General, Self::Appearance, Self::Voice, Self::Profile]),
        ("Coding", &[Self::Shortcuts]),
        ("Integrations", &[Self::McpServers]),
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Voice => "voice",
            Self::Profile => "profile",
            Self::Shortcuts => "shortcuts",
            Self::McpServers => "mcp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Voice => "Voice",
            Self::Profile => "Profile",
            Self::Shortcuts => "Shortcuts",
            Self::McpServers => "MCP Servers",
        }
    }

    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::General => IconName::SlidersHorizontal,
            Self::Appearance => IconName::Palette,
            Self::Voice => IconName::Mic,
            Self::Profile => IconName::CircleUser,
            Self::Shortcuts => IconName::Keyboard,
            Self::McpServers => IconName::PlugZap,
        }
    }
}
