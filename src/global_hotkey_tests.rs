//! Tests for the global summon hotkey: chord parsing/validation, event
//! matching, the register/unregister transitions `apply` drives through the
//! `imp` recorder seam, settings round-trip, and `summon`'s window focus.
//! Declared from `global_hotkey.rs` via `#[path]` — `main.rs` is at the
//! SLOC cap.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use super::{Chord, DEFAULT_CHORD, KeyEvent, MODIFIER_ALT, MODIFIER_CMD, MODIFIER_CTRL, MODIFIER_FN, MODIFIER_SHIFT, imp};
use crate::workspace::Workspace;

fn ev(key_code: u16, chars: &str, modifiers: u16) -> KeyEvent {
    KeyEvent { key_code, chars: chars.into(), modifiers }
}

#[test]
fn chord_parse_accepts_modifier_plus_key() {
    let chord = Chord::parse("cmd-shift-space").unwrap();
    assert_eq!(chord.canonical(), "cmd-shift-space");
    // Order and case normalize to the canonical form.
    assert_eq!(Chord::parse("SHIFT-CMD-space").unwrap().canonical(), "cmd-shift-space");
    assert_eq!(Chord::parse("ctrl-alt-k").unwrap().canonical(), "ctrl-alt-k");
    assert_eq!(Chord::parse("cmd-f5").unwrap().canonical(), "cmd-f5");
    // A bare modifier chord folds the last modifier into the key — still
    // rejected as modifier-only.
    assert!(Chord::parse("cmd-shift").is_err());
    assert!(Chord::parse("cmd").is_err());
}

#[test]
fn chord_parse_rejects_unusable_input() {
    for bad in ["", "  ", "space", "a", "cmd-", "cmd-bogus-key", "fn-cmd-a", "cmd-shift-"] {
        assert!(Chord::parse(bad).is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn chord_matches_key_down_events() {
    let chord = Chord::parse("cmd-shift-space").unwrap();
    assert!(chord.matches(&ev(49, " ", MODIFIER_CMD | MODIFIER_SHIFT)));
    // Extra/missing modifiers don't match.
    assert!(!chord.matches(&ev(49, " ", MODIFIER_CMD)));
    assert!(!chord.matches(&ev(49, " ", MODIFIER_CMD | MODIFIER_SHIFT | MODIFIER_ALT)));
    // The Function bit macOS forces onto F-keys/arrows is masked out.
    let f5 = Chord::parse("cmd-f5").unwrap();
    assert!(f5.matches(&ev(96, "", MODIFIER_CMD | MODIFIER_FN)));
    // Letter chords match charactersIgnoringModifiers, not the key code.
    let k = Chord::parse("cmd-shift-k").unwrap();
    assert!(k.matches(&ev(40, "k", MODIFIER_CMD | MODIFIER_SHIFT)));
    assert!(!k.matches(&ev(40, "j", MODIFIER_CMD | MODIFIER_SHIFT)));
    // Named keys match by key code.
    let esc = Chord::parse("ctrl-escape").unwrap();
    assert!(esc.matches(&ev(53, "", MODIFIER_CTRL)));
    assert!(!esc.matches(&ev(49, " ", MODIFIER_CTRL)));
}

#[test]
fn apply_registers_and_unregisters() {
    imp::EVENTS.lock().clear();
    let app = TestAppContext::single();
    app.update(|cx| {
        super::apply(true, "cmd-shift-space", cx);
        super::apply(false, "cmd-shift-space", cx);
        super::apply(true, "ctrl-alt-p", cx);
    });
    let events = imp::EVENTS.lock().clone();
    assert_eq!(
        events,
        vec![
            imp::MonitorEvent::Unregister,
            imp::MonitorEvent::Register(Chord::parse("cmd-shift-space").unwrap()),
            imp::MonitorEvent::Unregister,
            imp::MonitorEvent::Unregister,
            imp::MonitorEvent::Register(Chord::parse("ctrl-alt-p").unwrap()),
        ]
    );
}

#[test]
fn apply_falls_back_to_default_chord() {
    imp::EVENTS.lock().clear();
    let app = TestAppContext::single();
    app.update(|cx| super::apply(true, "", cx));
    let events = imp::EVENTS.lock().clone();
    assert_eq!(events, vec![imp::MonitorEvent::Unregister, imp::MonitorEvent::Register(Chord::parse(DEFAULT_CHORD).unwrap())]);
}

#[test]
fn apply_invalid_chord_unregisters_without_registering() {
    imp::EVENTS.lock().clear();
    let app = TestAppContext::single();
    app.update(|cx| super::apply(true, "not-a-key", cx));
    assert_eq!(imp::EVENTS.lock().clone(), vec![imp::MonitorEvent::Unregister]);
}

#[test]
fn settings_round_trip_and_defaults() {
    // Missing fields deserialize to disabled + default chord.
    let s: crate::persist::Settings = serde_json::from_str("{}").unwrap();
    assert!(!s.global_hotkey_enabled);
    assert!(s.global_hotkey.is_empty());
    // Values round-trip through the file format.
    let s: crate::persist::Settings = serde_json::from_str(r#"{"global_hotkey_enabled":true,"global_hotkey":"ctrl-alt-p"}"#).unwrap();
    assert!(s.global_hotkey_enabled);
    assert_eq!(s.global_hotkey, "ctrl-alt-p");
    let json = serde_json::to_string(&s).unwrap();
    let back: crate::persist::Settings = serde_json::from_str(&json).unwrap();
    assert!(back.global_hotkey_enabled);
    assert_eq!(back.global_hotkey, "ctrl-alt-p");
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir — same seam `settings_general_tests` uses.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-hotkey-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

#[test]
fn toggle_registers_and_persists() {
    imp::EVENTS.lock().clear();
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| {
        this.set_global_hotkey_enabled(true, cx);
        this.save_settings();
    });
    assert_eq!(
        imp::EVENTS.lock().clone(),
        vec![imp::MonitorEvent::Unregister, imp::MonitorEvent::Register(Chord::parse(DEFAULT_CHORD).unwrap())]
    );
    let s = crate::persist::load_settings();
    assert!(s.global_hotkey_enabled, "toggle must persist");
    ws.update(cx, |this, cx| {
        this.set_global_hotkey_enabled(false, cx);
        this.save_settings();
    });
    assert_eq!(imp::EVENTS.lock().last(), Some(&imp::MonitorEvent::Unregister));
    assert!(!crate::persist::load_settings().global_hotkey_enabled);
}

#[test]
fn commit_validates_and_registers() {
    imp::EVENTS.lock().clear();
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| this.set_global_hotkey_enabled(true, cx));
    imp::EVENTS.lock().clear();
    // An invalid chord keeps the old binding and records the error.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.hotkey_input.update(cx, |input, cx| input.set_value("bogus-chord-xx", window, cx));
            this.commit_global_hotkey(window, cx);
        });
    });
    ws.read_with(cx, |this, _| {
        assert!(this.hotkey_error.is_some());
        assert!(this.global_hotkey.is_empty(), "invalid chord must not overwrite the stored one");
    });
    // A valid chord persists canonicalized and re-registers.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.hotkey_input.update(cx, |input, cx| input.set_value("SHIFT-CMD-p", window, cx));
            this.commit_global_hotkey(window, cx);
        });
    });
    ws.read_with(cx, |this, _| {
        assert!(this.hotkey_error.is_none());
        assert_eq!(this.global_hotkey, "cmd-shift-p");
    });
    assert_eq!(
        imp::EVENTS.lock().clone(),
        vec![imp::MonitorEvent::Unregister, imp::MonitorEvent::Register(Chord::parse("cmd-shift-p").unwrap())]
    );
    assert_eq!(crate::persist::load_settings().global_hotkey, "cmd-shift-p");
}

#[test]
fn summon_opens_a_window_when_none_exist() {
    let app = TestAppContext::single();
    let dir = std::env::temp_dir().join(format!("rixlcode-hotkey-summon-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process.
    unsafe { std::env::set_var("HOME", &dir) };
    app.update(|cx| {
        gpui_kit::init(cx);
        assert!(cx.windows().is_empty());
        super::summon(cx);
    });
    app.run_until_parked();
    assert_eq!(app.update(|cx| cx.windows().len()), 1, "summon with no windows must open one");
}
