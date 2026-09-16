//! Headless UI tests for stale-load handling in the Changes panel — split
//! from `changes_ui_tests.rs` to stay under the SLOC cap. Same
//! `TestAppContext::single()` pattern (the `#[gpui_kit::test]` macro and a
//! bare `use gpui_kit::*` crash the proc-macro on this nightly).

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

use crate::changes_ui_tests::{change, mount, sample_diff};
use crate::git::ChangeStatus;

/// A refresh result stamped with an older generation must not publish: a
/// newer refresh was requested while it ran, so landing it would revert the
/// panel to a stale snapshot.
#[test]
fn stale_refresh_result_is_discarded() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![change("old.rs", ChangeStatus::Modified, 1, 0)];
            let stale_gen = this.changes_generation;
            this.changes_generation += 1; // a newer refresh was requested
            this.land_changes(
                stale_gen,
                crate::changes::ChangesSnapshot {
                    changes: vec![change("stale.rs", ChangeStatus::Added, 5, 0)],
                    branch: None,
                    commits: vec![],
                    stashes: vec![],
                    conflicts: vec![],
                    pr: None,
                },
                cx,
            );
            assert_eq!(this.changes[0].path, "old.rs", "stale collection did not publish");
            this.land_changes(
                this.changes_generation,
                crate::changes::ChangesSnapshot {
                    changes: vec![change("fresh.rs", ChangeStatus::Added, 5, 0)],
                    branch: None,
                    commits: vec![],
                    stashes: vec![],
                    conflicts: vec![],
                    pr: None,
                },
                cx,
            );
            assert_eq!(this.changes[0].path, "fresh.rs", "current collection publishes");
        });
    });
}

/// Clicking a row while its diff load is still running collapses it: the
/// pending token clears and the late result is discarded instead of opening
/// the row.
#[test]
fn second_click_during_load_collapses_and_discards() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![change("src/edited.rs", ChangeStatus::Modified, 1, 1)];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);

        window.click(("change-row", 0usize), cx);
        assert_ne!(ws.read(cx).changes[0].diff_load, 0, "first click started a load");
        window.click(("change-row", 0usize), cx);
        assert_eq!(ws.read(cx).changes[0].diff_load, 0, "second click cancelled the load");
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("change-diff", 0usize)).is_none(), "late result did not reopen the row");
        assert!(ws.read(cx).changes[0].diff.is_none());
        assert_eq!(ws.read(cx).changes[0].diff_load, 0);
    });
}

/// A diff load that outlives a refresh must not attach to the refreshed row:
/// the generation it was issued under no longer matches. A collapse+re-expand
/// under the same generation likewise discards the older load — its token is
/// stale even though the row is pending again.
#[test]
fn stale_diff_results_are_discarded() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![change("f.rs", ChangeStatus::Modified, 1, 1)];
            let generation = this.changes_generation;
            this.toggle_change_diff(0, cx);
            let token = this.changes[0].diff_load;

            // Refresh requested under the load → the diff's generation is stale.
            this.changes_generation += 1;
            this.land_change_diff((generation, token), Some(sample_diff()), cx);
            assert!(this.changes[0].diff.is_none(), "pre-refresh diff did not attach");

            // Collapse + re-expand → the older load's token no longer matches.
            this.changes_generation = generation;
            this.toggle_change_diff(0, cx);
            this.toggle_change_diff(0, cx);
            let newer = this.changes[0].diff_load;
            assert_ne!(newer, token, "re-expand stamped a fresh token");
            this.land_change_diff((generation, token), Some(sample_diff()), cx);
            assert!(this.changes[0].diff.is_none(), "stale-token diff did not attach");
            this.land_change_diff((generation, newer), Some(sample_diff()), cx);
            assert!(this.changes[0].diff.is_some(), "current-token diff attaches");
        });
    });
}
