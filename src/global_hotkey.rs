//! The system-wide summon hotkey: an `NSEvent` global key-down monitor that
//! activates the app and focuses its frontmost workspace window when the
//! configured chord is pressed anywhere. gpui-pre exposes no global-hotkey
//! API, so the monitor goes through `objc2-app-kit` (already linked for the
//! sidebar vibrancy view); `NSEvent` is the only new feature flag.
//!
//! The chord is `Settings.global_hotkey` (default "cmd-shift-space") and
//! must carry at least one real modifier — a bare key would fire on every
//! keystroke in every app. `apply` is the single entry point: it replaces
//! whatever monitor is installed, so startup and the settings toggles share
//! it. Registration goes through `imp`, which tests swap for a recorder —
//! the same seam `open_in` uses for spawned commands.

use gpui_kit::*;

/// The chord used when `Settings.global_hotkey` is empty (fresh installs).
pub(crate) const DEFAULT_CHORD: &str = "cmd-shift-space";

/// A parsed hotkey chord: gpui `Keystroke` syntax plus the "needs a real
/// modifier" rule. `fn` is rejected — macOS sets `NSEventModifierFlags::
/// Function` on F-keys and arrows regardless, so it can't be matched
/// reliably and is masked out of event flags instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chord {
    modifiers: Modifiers,
    /// gpui key name: a single char ("a", "1", "-") or a named key
    /// ("space", "f5", "left").
    key: String,
}

impl Chord {
    /// Parse a chord string, rejecting anything that isn't a usable global
    /// hotkey: unparseable input, modifier-only chords (gpui's parser folds
    /// a trailing modifier into the key, e.g. "cmd-shift" → cmd + "shift"),
    /// `fn` chords, and bare keys with no modifier.
    pub(crate) fn parse(source: &str) -> Result<Self, String> {
        let source = source.trim();
        let ks = Keystroke::parse(source).map_err(|_| format!("\"{source}\" isn't a valid shortcut"))?;
        if ks.modifiers.function {
            return Err("the fn key can't be part of a global hotkey".into());
        }
        if !ks.modifiers.modified() {
            return Err("add a modifier — cmd, ctrl, alt or shift".into());
        }
        if matches!(ks.key.as_str(), "shift" | "control" | "alt" | "platform" | "function") {
            return Err("the chord needs a non-modifier key".into());
        }
        if ks.key.chars().count() != 1 && named_key_code(&ks.key).is_none() {
            return Err(format!("\"{}\" isn't a key the hotkey can match", ks.key));
        }
        Ok(Self { modifiers: ks.modifiers, key: ks.key })
    }

    /// The canonical chord string — `Keystroke::unparse` order
    /// (ctrl-alt-cmd-shift-key), what the settings field shows after a save.
    pub(crate) fn canonical(&self) -> String {
        let mut s = String::new();
        if self.modifiers.control {
            s.push_str("ctrl-");
        }
        if self.modifiers.alt {
            s.push_str("alt-");
        }
        if self.modifiers.platform {
            s.push_str("cmd-");
        }
        if self.modifiers.shift {
            s.push_str("shift-");
        }
        s.push_str(&self.key);
        s
    }

    /// Does this key-down event match the chord? Modifier comparison masks
    /// out caps-lock/numeric-pad/help/function so incidental flags (and the
    /// Function bit macOS forces onto F-keys/arrows) can't break a match.
    /// Named keys match by macOS virtual key code; single chars match the
    /// event's `charactersIgnoringModifiers`, so letter chords follow the
    /// user's layout rather than a fixed key position.
    fn matches(&self, event: &KeyEvent) -> bool {
        let want = self.modifier_mask();
        if event.modifiers & MODIFIER_MASK != want {
            return false;
        }
        match named_key_code(&self.key) {
            Some(code) => event.key_code == code,
            None => self.key.chars().count() == 1 && event.chars.eq_ignore_ascii_case(&self.key),
        }
    }

    /// The chord's modifiers as `KeyEvent.modifiers` bits.
    fn modifier_mask(&self) -> u16 {
        let m = &self.modifiers;
        (m.control as u16) | ((m.shift as u16) << 1) | ((m.alt as u16) << 2) | ((m.platform as u16) << 3)
    }
}

/// The modifier bits `Chord::matches` compares — ctrl/shift/alt/cmd only.
const MODIFIER_MASK: u16 = 0b1111;

/// The parts of an `NSEvent` key-down the matcher needs, in plain values so
/// tests can build events without AppKit. `modifiers` uses the
/// `MODIFIER_*` bits below (mirroring `NSEventModifierFlags`, which tests
/// can't link).
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyEvent {
    pub key_code: u16,
    /// `charactersIgnoringModifiers`, lowercased by the caller.
    pub chars: String,
    /// `MODIFIER_*` bits; `MODIFIER_FN` is set for F-keys/arrows by macOS.
    pub modifiers: u16,
}

pub(crate) const MODIFIER_CTRL: u16 = 1 << 0;
pub(crate) const MODIFIER_SHIFT: u16 = 1 << 1;
pub(crate) const MODIFIER_ALT: u16 = 1 << 2;
pub(crate) const MODIFIER_CMD: u16 = 1 << 3;
/// `NSEventModifierFlags::Function` — masked out of matching, never set by
/// `Chord::modifier_mask` (fn chords are rejected at parse).
pub(crate) const MODIFIER_FN: u16 = 1 << 4;

/// macOS virtual key codes for the named keys a chord can use — the same
/// names gpui's `Keystroke` parser accepts. Single-char keys don't appear
/// here: they match on `charactersIgnoringModifiers` instead, which follows
/// the active layout.
fn named_key_code(key: &str) -> Option<u16> {
    Some(match key {
        "enter" => 36,
        "tab" => 48,
        "space" => 49,
        "backspace" => 51,
        "escape" => 53,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
        "delete" => 117,
        "home" => 115,
        "end" => 119,
        "pageup" => 116,
        "pagedown" => 121,
        "insert" => 114,
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        _ => return None,
    })
}

/// Install or replace the global monitor from the current settings —
/// `enabled` + `chord` come from the caller (the workspace's live values or
/// freshly loaded settings), so `apply` never reads stale state. An empty
/// chord falls back to `DEFAULT_CHORD`; an unparseable chord unregisters
/// and logs rather than keeping a stale binding.
pub(crate) fn apply(enabled: bool, chord: &str, cx: &App) {
    imp::unregister();
    if !enabled {
        return;
    }
    let chord = if chord.trim().is_empty() { DEFAULT_CHORD } else { chord.trim() };
    match Chord::parse(chord) {
        Ok(chord) => imp::register(chord, cx.to_async()),
        Err(e) => log::warn!("global hotkey not registered: {e}"),
    }
}

/// Bring the app forward and focus its frontmost workspace window — the
/// hotkey's action. With no windows open it opens one, matching the dock
/// icon's reopen behavior.
pub(crate) fn summon(cx: &mut App) {
    cx.activate(false);
    let window = cx
        .window_stack()
        .and_then(|stack| stack.into_iter().next())
        .or_else(|| cx.active_window())
        .or_else(|| cx.windows().into_iter().next());
    match window {
        Some(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        },
        None => crate::lifecycle::open_new_window(cx),
    }
}

/// The real monitor: an `NSEvent` global key-down monitor retained in a
/// thread-local (register/unregister/summon all run on the main thread).
/// The handler can't hold `&mut App` — the event fires mid-dispatch — so it
/// schedules `summon` on the foreground executor instead.
#[cfg(all(target_os = "macos", not(test)))]
mod imp {
    use gpui_kit::{App, AsyncApp};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
    use std::cell::RefCell;
    use std::ptr::NonNull;

    use super::{Chord, KeyEvent, MODIFIER_ALT, MODIFIER_CMD, MODIFIER_CTRL, MODIFIER_FN, MODIFIER_SHIFT};

    thread_local! {
        static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }

    pub(super) fn register(chord: Chord, app: AsyncApp) {
        let block = block2::RcBlock::new(move |event: NonNull<NSEvent>| {
            let event = unsafe { event.as_ref() };
            if chord.matches(&key_event(event)) {
                summon_deferred(&app);
            }
        });
        let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block);
        MONITOR.with(|slot| *slot.borrow_mut() = monitor);
        if MONITOR.with(|slot| slot.borrow().is_none()) {
            log::warn!("global hotkey: macOS refused the key monitor (Input Monitoring permission?)");
        }
    }

    pub(super) fn unregister() {
        let monitor = MONITOR.with(|slot| slot.borrow_mut().take());
        if let Some(monitor) = monitor {
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    }

    /// Pull the matchable fields out of the raw event.
    fn key_event(event: &NSEvent) -> KeyEvent {
        let flags = event.modifierFlags();
        let mut modifiers = 0;
        modifiers |= flags.contains(NSEventModifierFlags::Control) as u16 * MODIFIER_CTRL;
        modifiers |= flags.contains(NSEventModifierFlags::Shift) as u16 * MODIFIER_SHIFT;
        modifiers |= flags.contains(NSEventModifierFlags::Option) as u16 * MODIFIER_ALT;
        modifiers |= flags.contains(NSEventModifierFlags::Command) as u16 * MODIFIER_CMD;
        modifiers |= flags.contains(NSEventModifierFlags::Function) as u16 * MODIFIER_FN;
        let chars = event.charactersIgnoringModifiers().map(|s| s.to_string().to_lowercase()).unwrap_or_default();
        KeyEvent { key_code: event.keyCode(), chars, modifiers }
    }

    /// Schedule `summon` on the main executor — the monitor fires inside
    /// AppKit's event dispatch, where taking the app lock directly could
    /// re-enter a window update.
    fn summon_deferred(app: &AsyncApp) {
        let executor = app.foreground_executor().clone();
        let app = app.clone();
        executor
            .spawn(async move {
                app.update(|cx: &mut App| super::summon(cx));
            })
            .detach();
    }
}

/// The test seam: registration calls are recorded instead of touching
/// AppKit, so tests assert register/unregister transitions headlessly.
#[cfg(test)]
mod imp {
    use gpui_kit::AsyncApp;

    use super::Chord;

    /// One recorded registration call, in order.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum MonitorEvent {
        Register(Chord),
        Unregister,
    }

    pub(crate) static EVENTS: parking_lot::Mutex<Vec<MonitorEvent>> = parking_lot::Mutex::new(Vec::new());

    pub(super) fn register(chord: Chord, _app: AsyncApp) {
        EVENTS.lock().push(MonitorEvent::Register(chord));
    }

    pub(super) fn unregister() {
        EVENTS.lock().push(MonitorEvent::Unregister);
    }
}

/// The non-macOS stub — the crate's objc2 deps are macOS-only, so other
/// targets get a no-op monitor (the settings UI still works).
#[cfg(all(not(target_os = "macos"), not(test)))]
mod imp {
    use gpui_kit::AsyncApp;

    use super::Chord;

    pub(super) fn register(_chord: Chord, _app: AsyncApp) {}

    pub(super) fn unregister() {}
}

#[cfg(test)]
#[path = "global_hotkey_tests.rs"]
mod global_hotkey_tests;
