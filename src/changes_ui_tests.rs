//! Headless UI tests for the Changes panel — same `TestAppContext::single()`
//! pattern as `ui_tests.rs` (the `#[gpui_kit::test]` macro and a bare
//! `use gpui_kit::*` crash the proc-macro on this nightly).

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, KeyBinding, TestAppContext, VisualTestContext};

use crate::git::{ChangeStatus, FileChange};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-changes-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    cx.add_window_view(Workspace::new)
}

#[test]
fn changes_panel_toggles_via_keybinding_and_close_button() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys([KeyBinding::new("cmd-shift-j", crate::ToggleChanges, Some("workspace"))]);
        window.draw(cx).clear(cx);
        assert!(window.try_find("changes-panel").is_none(), "panel starts closed");

        window.press("cmd-shift-j", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("changes-panel").visible(), "cmd-shift-j opens the panel");
        assert!(ws.read(cx).changes_panel_open);

        window.click("close-changes", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("changes-panel").is_none(), "close button hides the panel");
        assert!(!ws.read(cx).changes_panel_open);
    });
}

#[test]
fn changes_panel_lists_rows_and_refresh_recollects() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![
                FileChange {
                    path: "src/edited.rs".into(),
                    status: ChangeStatus::Modified,
                    added: 3,
                    deleted: 1,
                },
                FileChange {
                    path: "src/new.rs".into(),
                    status: ChangeStatus::Added,
                    added: 12,
                    deleted: 0,
                },
            ];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find(("change-row", 0usize)).visible(), "first row renders");
        assert!(window.find(("change-row", 1usize)).visible(), "second row renders");

        // Refresh re-runs real git collection — the test cwd is this repo, so
        // the panel keeps working; only the state round-trip is asserted.
        window.click("refresh-changes", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("changes-panel").visible(), "panel stays open after refresh");
    });
}
