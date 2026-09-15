//! Tests for the keyboard-shortcuts cheat sheet: the `SHORTCUT_SPECS` table
//! drives both the keymap and the overlay (no drift), Cmd-/ opens it, and
//! Esc / backdrop click / Cmd-/ again close it.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AsKeystroke, Keystroke, TestAppContext, point, px};

use crate::composer_testutil::open_workspace;
use crate::shortcuts::{SHORTCUT_SPECS, ShortcutGroup};

/// The keymap and the cheat sheet read the same table: every bound spec
/// produces exactly one `KeyBinding` for its own keystroke, and the overlay
/// lists every spec — bound or display-only.
#[test]
fn keymap_and_overlay_share_one_table() {
    let bound = SHORTCUT_SPECS.iter().filter(|s| s.bind.is_some()).count();
    // One extra binding beyond the table: cmd-f is also registered in the
    // "Input" context so it still opens the find bar while an input is
    // focused (inputs bind cmd-f to their own Search and swallow it).
    assert_eq!(crate::workspace_keys().len(), bound + 1, "workspace_keys registers every bound spec plus the Input-context cmd-f");
    assert!(bound > 0);

    for spec in SHORTCUT_SPECS {
        let parsed = Keystroke::parse(spec.keys).unwrap_or_else(|_| panic!("spec {:?} is not a valid keystroke", spec.keys));
        assert!(!spec.description.is_empty(), "spec {:?} needs a description", spec.keys);
        if let Some(bind) = spec.bind {
            let binding = bind(spec.keys);
            let keystrokes = binding.keystrokes();
            assert_eq!(keystrokes.len(), 1, "{:?} should bind a single keystroke", spec.keys);
            assert_eq!(
                *keystrokes[0].as_keystroke(),
                parsed,
                "binding for {:?} must register the same keystroke the overlay shows",
                spec.keys
            );
        }
    }

    for group in ShortcutGroup::ALL {
        assert!(SHORTCUT_SPECS.iter().any(|s| s.group == group), "group {} must have at least one row", group.label());
    }
}

/// Cmd-/ opens the overlay and it lists every spec — each row renders its
/// description and its key combo, grouped under section headers. Rows past
/// the scroll fold exist in the tree but aren't `visible()`, so rows assert
/// presence + labels rather than on-screen bounds.
#[test]
fn shortcuts_overlay_opens_on_cmd_slash() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        assert!(window.try_find("shortcuts-overlay").is_none(), "overlay starts closed");

        window.press("cmd-/", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).shortcuts_open, "cmd-/ should open the cheat sheet");
        assert!(window.find("shortcuts-overlay").visible());
        for group in ShortcutGroup::ALL {
            let id = format!("shortcuts-group-{}", group.label().to_lowercase());
            assert!(window.try_find(id.clone()).is_some(), "group {id} should render");
        }
        for (ix, spec) in SHORTCUT_SPECS.iter().enumerate() {
            window.find(("shortcut-row", ix));
            assert_eq!(
                window.find(("shortcut-desc", ix)).label(),
                Some(spec.description),
                "row {:?} should show its description",
                spec.keys
            );
            assert_eq!(window.find(("shortcut-keys", ix)).label(), Some(spec.keys), "row {:?} should show its key combo", spec.keys);
        }
    });
}

/// Esc, a backdrop click, and Cmd-/ again all dismiss the overlay.
#[test]
fn shortcuts_overlay_closes_on_escape_backdrop_and_toggle() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);

        // Esc closes.
        window.press("cmd-/", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("shortcuts-overlay").visible());
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).shortcuts_open, "esc should close the cheat sheet");
        assert!(window.try_find("shortcuts-overlay").is_none());

        // A press on the dimmed backdrop (outside the centered panel) closes.
        window.press("cmd-/", cx);
        window.draw(cx).clear(cx);
        window.click_at("shortcuts-backdrop", point(px(8.), px(8.)), cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).shortcuts_open, "backdrop click should close the cheat sheet");
        assert!(window.try_find("shortcuts-overlay").is_none());

        // Cmd-/ toggles it closed; the header ✕ does too.
        window.press("cmd-/", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("shortcuts-overlay").visible());
        window.press("cmd-/", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).shortcuts_open, "cmd-/ should toggle the cheat sheet closed");

        window.press("cmd-/", cx);
        window.draw(cx).clear(cx);
        window.click("shortcuts-close", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).shortcuts_open, "header close should dismiss the cheat sheet");
    });
}
