//! Tests for settings search: the label index (`matching_sections` /
//! `row_matches` via `SearchCtx::wrap`) plus a headless check that typing in
//! the nav field filters the rail and marks matching rows in the open
//! section.

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

use crate::composer_testutil::open_workspace;
use crate::views::settings_nav::Section;
use crate::views::settings_search::{matching_sections, row_labels};

#[test]
fn empty_query_matches_every_section() {
    assert_eq!(matching_sections(""), Section::ALL.to_vec());
}

#[test]
fn query_matches_section_labels() {
    assert_eq!(matching_sections("appearance"), vec![Section::Appearance]);
    assert_eq!(matching_sections("mcp"), vec![Section::McpServers]);
    // Case-insensitive: the nav lowercases the query before filtering.
    assert_eq!(matching_sections("voice"), vec![Section::Voice]);
}

#[test]
fn query_matches_row_labels() {
    // "font" hits only Appearance's font rows — the manual-check scenario.
    assert_eq!(matching_sections("font"), vec![Section::Appearance]);
    assert_eq!(matching_sections("notify on reply"), vec![Section::General]);
    assert_eq!(matching_sections("frosted"), vec![Section::Appearance]);
    assert_eq!(matching_sections("setup script"), vec![Section::Project]);
    // Voice's dictation rows AND the "Toggle dictation" shortcut.
    assert_eq!(matching_sections("dictation"), vec![Section::Voice, Section::Shortcuts]);
    // Group headers are indexed too.
    assert_eq!(matching_sections("notifications"), vec![Section::General]);
}

#[test]
fn query_matches_shortcut_descriptions() {
    // Shortcuts index `SHORTCUT_SPECS` descriptions, not literals.
    assert_eq!(matching_sections("new chat"), vec![Section::Shortcuts]);
    assert_eq!(matching_sections("toggle sidebar"), vec![Section::Shortcuts]);
}

#[test]
fn no_match_returns_empty() {
    assert!(matching_sections("zzz-no-such-setting").is_empty());
}

#[test]
fn every_section_has_a_label_index() {
    // A new Section variant without a registry entry would silently drop
    // out of search — the match arm is exhaustive, so this guards the data.
    for section in Section::ALL {
        assert!(!row_labels(section).is_empty(), "{:?} has no indexed labels", section.name());
    }
}

/// Typing "font" leaves only Appearance in the rail and marks the two font
/// rows inside the open section; a query with no hits shows the muted
/// placeholder row.
#[test]
fn search_filters_nav_and_marks_rows() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.open_settings(window, cx));
        window.draw(cx).clear(cx);

        ws.read(cx).settings_panel.clone().update(cx, |panel, cx| {
            panel.search.update(cx, |input, cx| input.set_value("font", window, cx));
        });
        window.draw(cx).clear(cx);

        assert!(window.find("settings-nav-appearance").visible(), "appearance should remain");
        assert!(window.try_find("settings-nav-general").is_none(), "general should filter out");
        assert!(window.try_find("settings-nav-shortcuts").is_none(), "shortcuts should filter out");

        window.click("settings-nav-appearance", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-search-hit-System font").visible(), "interface font row should be marked");
        assert!(window.find("settings-search-hit-Default mono font").visible(), "code font row should be marked");
        assert!(window.try_find("settings-search-hit-Frosted glass sidebar").is_none(), "non-matching row unmarked");

        ws.read(cx).settings_panel.clone().update(cx, |panel, cx| {
            panel.search.update(cx, |input, cx| input.set_value("zzz", window, cx));
        });
        window.draw(cx).clear(cx);
        assert!(window.find("settings-nav-empty").visible(), "no-match placeholder should show");
    });
}
