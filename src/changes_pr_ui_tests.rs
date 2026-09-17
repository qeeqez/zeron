//! Headless UI tests for the Changes panel's PR row — the state/check
//! readout, the Open button's `cx.open_url`, and the refresh affordance.
//! Same `TestAppContext::single()` pattern as `changes_git_ui_tests.rs`.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, VisualTestContext};

use crate::changes_ui_tests::mount;
use crate::git::{BranchStatus, PrChecks, PrState, PrStatus};
use crate::workspace::Workspace;

fn branch() -> BranchStatus {
    BranchStatus {
        name: "main".into(),
        upstream: Some("origin/main".into()),
        ahead: 0,
        behind: 0,
    }
}

fn pr(number: u64, state: PrState, checks: PrChecks) -> PrStatus {
    PrStatus {
        number,
        url: format!("https://example.test/pr/{number}"),
        state,
        checks,
    }
}

/// A workspace whose project root is a plain temp dir — not a repo — so a
/// `refresh_pr` deterministically lands `None` whether or not `gh` exists.
fn non_repo_workspace(app: &mut TestAppContext) -> (gpui_kit::Entity<Workspace>, &mut VisualTestContext) {
    let (ws, cx) = mount(app);
    let dir = std::env::temp_dir().join(format!("rixlcode-pr-ui-norepo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    cx.update(|_window, cx| {
        ws.update(cx, |this, _cx| {
            this.project = crate::project::Project::open(&dir);
        });
    });
    (ws, cx)
}

#[test]
fn pr_row_shows_number_state_and_checks() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.git.pr = Some(pr(42, PrState::Open, PrChecks { pass: 2, fail: 1, pending: 3, failures: vec!["lint".into()] }));
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        let row = window.find("pr-row");
        assert!(row.visible(), "PR row renders");
        assert_eq!(row.label(), Some("#42 Open"), "row label carries number and state");
        assert!(window.find("pr-open").visible(), "Open button renders");
        assert!(window.find("pr-refresh").visible(), "refresh affordance renders");
        let chip = window.find("pr-checks");
        assert!(chip.visible(), "checks chip renders");
        assert_eq!(chip.label(), Some("Failed: lint\n1 failed · 3 pending · 2 passed"), "chip label carries the failing name and tallies");
    });
}

/// Clicking the chip opens the PR's checks page — `pr.url` + `/checks`.
#[test]
fn pr_checks_chip_opens_the_checks_page() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.git.pr = Some(pr(7, PrState::Open, PrChecks { pass: 1, fail: 1, pending: 0, failures: vec!["build".into()] }));
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        window.click("pr-checks", cx);
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.test/pr/7/checks"));
}

/// A PR whose rollup reported nothing renders no chip — no checks ran, so
/// there's no verdict to claim and no checks page worth opening.
#[test]
fn pr_checks_chip_hides_without_checks() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.git.pr = Some(pr(5, PrState::Open, PrChecks::default()));
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find("pr-row").visible(), "PR row still renders");
        assert!(window.try_find("pr-checks").is_none(), "empty rollup → no chip");
    });
}

#[test]
fn pr_row_hides_without_a_pr() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.git.pr = None;
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find("changes-git").visible(), "git block still renders");
        assert!(window.try_find("pr-row").is_none(), "no PR → no row");
    });
}

/// Clicking Open hands the PR's URL to the browser — the test platform
/// records `cx.open_url` as `opened_url`.
#[test]
fn pr_open_button_opens_the_url() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.git.pr = Some(pr(7, PrState::Merged, PrChecks::default()));
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        window.click("pr-open", cx);
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.test/pr/7"));
}

/// A landed refresh publishes `git.pr` — the path that fills the row after
/// `gh pr create` (the op's trailing `refresh_changes` carries the status).
#[test]
fn landed_snapshot_publishes_the_pr() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.changes_panel_open = true;
            this.land_changes(
                this.changes_generation,
                crate::changes::ChangesSnapshot {
                    changes: vec![],
                    scope: crate::changes::ChangesScope::default(),
                    branch: Some(branch()),
                    commits: vec![],
                    stashes: vec![],
                    conflicts: vec![],
                    pr: Some(pr(12, PrState::Open, PrChecks { pass: 4, fail: 0, pending: 0, failures: vec![] })),
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
        assert_eq!(window.find("pr-row").label(), Some("#12 Open"), "snapshot's PR renders");
    });
}

/// The row's refresh button re-fetches just the PR status: on a non-repo
/// root `gh` can't name a PR, so a stale seeded row clears — deterministic
/// whether or not `gh` is installed.
#[test]
fn pr_refresh_clears_a_gone_pr() {
    let mut app = TestAppContext::single();
    let (ws, cx) = non_repo_workspace(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.git.pr = Some(pr(9, PrState::Open, PrChecks::default()));
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find("pr-row").visible(), "seeded row renders");
        window.click("pr-refresh", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).git.pr.is_none(), "refresh landed None on a non-repo root");
        assert!(window.try_find("pr-row").is_none(), "row hid after refresh");
    });
}
