//! Headless test for the Shortcuts settings section: it renders straight from
//! `crate::shortcuts::SHORTCUT_SPECS` — the same table the keymap and the Cmd-/
//! cheat sheet read — so the section lists every real binding grouped by
//! `ShortcutGroup`, with no separate hardcoded table to drift.

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

use crate::composer_testutil::open_workspace;
use crate::shortcuts::{SHORTCUT_SPECS, ShortcutGroup};

/// Settings → Shortcuts lists every spec — each row renders its description
/// and its key combo, grouped under section headers. Iterating
/// `SHORTCUT_SPECS` asserts the count: a missing row fails `find`, an extra
/// row can't exist without a spec.
#[test]
fn shortcuts_section_lists_every_spec() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.open_settings(window, cx));
        window.draw(cx).clear(cx);
        window.click("settings-nav-shortcuts", cx);
        window.draw(cx).clear(cx);

        assert!(window.find("settings-section-shortcuts").visible(), "shortcuts section should render");
        for group in ShortcutGroup::ALL {
            let id = format!("settings-shortcuts-group-{}", group.label().to_lowercase());
            assert!(window.try_find(id.clone()).is_some(), "group {id} should render");
        }
        for (ix, spec) in SHORTCUT_SPECS.iter().enumerate() {
            window.find(("settings-shortcut-row", ix));
            assert_eq!(
                window.find(("settings-shortcut-desc", ix)).label(),
                Some(spec.description),
                "row {:?} should show its description",
                spec.keys
            );
            assert_eq!(
                window.find(("settings-shortcut-keys", ix)).label(),
                Some(spec.keys),
                "row {:?} should show its key combo",
                spec.keys
            );
        }
    });
}
