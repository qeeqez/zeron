//! Nav-rail content for the settings screen, rendered inside the shared
//! sidebar column (`sidebar-wrap`) when settings is open — so there's one
//! sidebar, not two. Split from `settings.rs`/`settings_sections.rs` to stay
//! under the 250-SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::settings::SettingsPanel;

/// "Back to app" row at the top of the nav — closes settings.
fn back_row(ws: &WeakEntity<crate::workspace::Workspace>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let ws = ws.clone();
    div()
        .id("settings-back")
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(IconName::ArrowLeft)
        .child("Back to app")
        .on_click(move |_, _, cx| {
            let _ = ws.update(cx, |this, cx| this.close_settings(cx));
        })
}

/// A nav row for one section — icon + label, highlights when selected.
fn nav_item(section: Section, selected: bool, panel: &Entity<SettingsPanel>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let panel = panel.clone();
    div()
        .id(SharedString::from(format!("settings-nav-{}", section.name())))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .when(!selected, |d| d.text_color(theme.muted_foreground))
        .when(selected, |d| d.bg(theme.list_active))
        .child(section.icon())
        .child(section.label())
        .on_click(move |_, _, cx| {
            panel.update(cx, |this, cx| {
                this.section = section;
                cx.notify();
            });
        })
}

/// The settings nav column for the shared sidebar — back row + search field +
/// the grouped section items, scrollable. Fills the sidebar's width.
pub fn settings_nav(panel: &Entity<SettingsPanel>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (ws, search, section, query) = {
        let p = panel.read(cx);
        (p.ws.clone(), p.search.clone(), p.section, p.search.read(cx).value().to_lowercase())
    };
    let items: Vec<AnyElement> = if query.is_empty() {
        Section::GROUPS
            .iter()
            .map(|(name, sections)| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pt_3()
                    .child(div().px_2().pb_1().text_xs().text_color(theme.muted_foreground).child(*name))
                    .children(sections.iter().map(|s| nav_item(*s, *s == section, panel, cx)))
                    .into_any_element()
            })
            .collect()
    } else {
        Section::ALL
            .iter()
            .filter(|s| s.label().to_lowercase().contains(query.as_str()))
            .map(|s| nav_item(*s, *s == section, panel, cx).into_any_element())
            .collect()
    };
    div()
        .id("settings-nav")
        .test_support()
        .w_full()
        .h_full()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .overflow_y_scroll()
        .child(back_row(&ws, cx))
        .child(Input::new(&search).prefix(IconName::Search).appearance(true))
        .children(items)
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
