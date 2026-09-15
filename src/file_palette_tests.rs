//! File-palette tests: `rank_files` units plus headless coverage of the
//! Cmd-P dialog — open, fuzzy filter, and confirm inserting an @-mention.
//! Narrow imports on purpose (see `composer_testutil`).

use gpui_kit::component::IndexPath;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Focusable, SharedString, TestAppContext};

use crate::composer_testutil::{composer_value, open_workspace};
use crate::file_palette::rank_files;

fn files(paths: &[&str]) -> Vec<SharedString> {
    paths.iter().map(|p| SharedString::from(*p)).collect()
}

#[test]
fn empty_query_lists_recent_first_then_scan_order() {
    let all = files(&["src/main.rs", "src/lib.rs", "docs/guide.md"]);
    // No picks yet: plain scan order.
    assert_eq!(rank_files(&all, &[], ""), all);
    // A pick leads; the rest keep scan order.
    let recent = files(&["docs/guide.md"]);
    assert_eq!(rank_files(&all, &recent, ""), files(&["docs/guide.md", "src/main.rs", "src/lib.rs"]));
}

#[test]
fn fuzzy_query_ranks_and_drops_non_matches() {
    let all = files(&["src/main.rs", "src/domain.rs", "docs/guide.md"]);
    // "smr" matches both src files — main.rs wins on the tighter spread.
    assert_eq!(rank_files(&all, &[], "smr"), files(&["src/main.rs", "src/domain.rs"]));
    // A query that isn't a subsequence drops the file entirely.
    assert_eq!(rank_files(&all, &[], "lib"), Vec::<SharedString>::new());
    // Recency breaks score ties: both "main" files score the same.
    let mains = files(&["src/main.rs", "docs/main.md"]);
    let recent = files(&["docs/main.md"]);
    assert_eq!(rank_files(&mains, &recent, "main"), files(&["docs/main.md", "src/main.rs"]));
    // A query matching nothing yields an empty list.
    assert!(rank_files(&all, &[], "zzz").is_empty());
}

#[test]
fn cmd_p_opens_filters_and_inserts_mention() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.project_files = files(&["src/main.rs", "src/lib.rs", "docs/guide.md"]);
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.press("cmd-p", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible(), "cmd-p opens the file picker");
        assert_eq!(ws.read(cx).file_palette.read(cx).matched_count(), 3, "empty query lists every file");

        // Typing filters by fuzzy score.
        ws.update(cx, |this, cx| {
            this.file_palette.update(cx, |state, cx| state.set_query("lib", window, cx));
        });
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).file_palette.read(cx).matched_count(), 1, "only the fuzzy match remains");

        ws.read(cx)
            .file_palette
            .read(cx)
            .focus_handle(cx)
            .dispatch_action(&Confirm { secondary: false }, window, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).recent_files, files(&["src/lib.rs"]), "the pick leads recency");
        assert!(window.try_find("command").is_none(), "the picker closes on confirm");
    });
    assert_eq!(composer_value(&ws, cx), "@src/lib.rs ");
}

#[test]
fn cmd_p_toggles_closed_and_mention_appends_to_draft() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.project_files = files(&["src/main.rs", "src/lib.rs"]);
            this.composer.update(cx, |s, cx| {
                s.set_value("check this", window, cx);
                s.focus(window, cx);
            });
        });
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.press("cmd-p", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible());
        // Cmd-P again closes, like the command palette.
        window.press("cmd-p", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("command").is_none(), "cmd-p toggles the picker closed");

        window.press("cmd-p", cx);
        window.draw(cx).clear(cx);
        // Row 1 is src/lib.rs — confirming appends to the existing draft.
        ws.update(cx, |this, cx| {
            this.file_palette
                .update(cx, |state, cx| state.set_selected_index(Some(IndexPath::new(1).section(0)), window, cx));
        });
        ws.read(cx)
            .file_palette
            .read(cx)
            .focus_handle(cx)
            .dispatch_action(&Confirm { secondary: false }, window, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(composer_value(&ws, cx), "check this @src/lib.rs ");
}

#[test]
fn go_to_file_command_runs_from_palette() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.project_files = files(&["src/main.rs"]);
            this.open_palette(window, cx);
        });
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| {
            this.palette.update(cx, |state, cx| state.set_query("go to file", window, cx));
        });
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).palette.read(cx).matched_count(), 1, "the command fuzzy-matches");
        ws.read(cx)
            .palette
            .read(cx)
            .focus_handle(cx)
            .dispatch_action(&Confirm { secondary: false }, window, cx);
    });
    // Confirm → close palette → Run opens the file picker, all deferred.
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible(), "the file picker opens after the palette closes");
        assert_eq!(ws.read(cx).file_palette.read(cx).matched_count(), 1);
    });
}
