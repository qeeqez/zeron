//! Voice dictation tests: a scripted `FakeEngine` drives the real
//! `Workspace` dictation path — toggle, phases, composer insertion, settings
//! persistence, and permission-denied notes — headless.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::composer_testutil::{composer_value, open_workspace, until};
use crate::voice::{DictationEngine, DictationEvent, DictationPhase, DictationSession, VoiceError};
use crate::workspace::Workspace;

/// A dictation engine the test scripts: `begin` either fails with `err` or
/// hands back a session whose event stream the test feeds via `tx()`.
#[derive(Clone)]
struct FakeEngine {
    inner: Arc<Mutex<FakeInner>>,
}

struct FakeInner {
    err: Option<VoiceError>,
    tx: Option<Sender<DictationEvent>>,
    cancelled: bool,
}

impl FakeEngine {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeInner { err: None, tx: None, cancelled: false })),
        }
    }

    fn failing(err: VoiceError) -> Self {
        let engine = Self::new();
        engine.inner.lock().err = Some(err);
        engine
    }

    /// The event channel for the in-flight take — present once `begin` ran.
    fn tx(&self) -> Sender<DictationEvent> {
        self.inner.lock().tx.clone().expect("no take started")
    }
}

impl DictationEngine for FakeEngine {
    fn begin(&self, _language: &str, _on_device: bool) -> Result<Box<dyn DictationSession>, VoiceError> {
        let mut inner = self.inner.lock();
        if let Some(err) = inner.err.take() {
            return Err(err);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        inner.tx = Some(tx);
        Ok(Box::new(FakeSession { inner: self.inner.clone(), rx: Some(rx) }))
    }
}

struct FakeSession {
    inner: Arc<Mutex<FakeInner>>,
    rx: Option<std::sync::mpsc::Receiver<DictationEvent>>,
}

impl DictationSession for FakeSession {
    fn finish(&mut self) -> std::sync::mpsc::Receiver<DictationEvent> {
        self.rx.take().unwrap_or_else(|| std::sync::mpsc::channel().1)
    }

    fn cancel(&mut self) {
        self.inner.lock().cancelled = true;
    }
}

/// Enable dictation and install the fake engine.
fn enable_voice(ws: &Entity<Workspace>, engine: FakeEngine, cx: &mut VisualTestContext) {
    cx.update(|_window, cx| {
        ws.update(cx, |this, _cx| {
            this.voice.enabled = true;
            this.voice.set_engine(Box::new(engine));
        });
    });
}

#[gpui_kit::test]
fn dictate_toggles_recording_state(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let engine = FakeEngine::new();
    enable_voice(&ws, engine, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
    });
    assert_eq!(ws.read_with(cx, |w, _| w.voice.phase), DictationPhase::Recording);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
    });
    assert_eq!(ws.read_with(cx, |w, _| w.voice.phase), DictationPhase::Transcribing);
}

#[gpui_kit::test]
fn dictated_text_inserts_into_composer(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let engine = FakeEngine::new();
    enable_voice(&ws, engine.clone(), cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
        ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
    });
    engine.tx().send(DictationEvent::Partial("hello".into())).unwrap();
    engine.tx().send(DictationEvent::Final(Ok("hello world".into()))).unwrap();
    until(&ws, cx, |w| w.voice.phase == DictationPhase::Idle);
    assert_eq!(composer_value(&ws, cx), "hello world");
    assert!(ws.read_with(cx, |w, _| w.voice.note.is_none()));
}

#[gpui_kit::test]
fn dictated_text_appends_after_existing_text(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let engine = FakeEngine::new();
    enable_voice(&ws, engine.clone(), cx);
    cx.update(|window, cx| {
        window.input("draft", cx);
        ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
        ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
    });
    engine.tx().send(DictationEvent::Final(Ok("more words".into()))).unwrap();
    until(&ws, cx, |w| w.voice.phase == DictationPhase::Idle);
    assert_eq!(composer_value(&ws, cx), "draft more words");
}

#[gpui_kit::test]
fn mic_denied_surfaces_note(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    enable_voice(&ws, FakeEngine::failing(VoiceError::MicDenied), cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
    });
    let (phase, note, is_error) = ws.read_with(cx, |w, _| (w.voice.phase, w.voice.note.clone(), w.voice.note_is_error));
    assert_eq!(phase, DictationPhase::Idle);
    assert!(is_error);
    assert!(note.unwrap().contains("Microphone access denied"));
}

#[gpui_kit::test]
fn mic_button_opens_voice_settings_when_disabled(cx: &mut TestAppContext) {
    let (_ws, cx) = open_workspace(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("dictate", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-screen").visible(), "mic click should open settings");
        assert!(window.find("settings-section-voice").visible(), "settings should land on Voice");
    });
}

#[gpui_kit::test]
fn mic_button_toggles_and_lands_text(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let engine = FakeEngine::new();
    enable_voice(&ws, engine.clone(), cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("dictate", cx);
        window.draw(cx).clear(cx);
    });
    assert_eq!(ws.read_with(cx, |w, _| w.voice.phase), DictationPhase::Recording);
    cx.update(|window, cx| {
        window.click("dictate", cx);
        window.draw(cx).clear(cx);
    });
    engine.tx().send(DictationEvent::Final(Ok("ship it".into()))).unwrap();
    until(&ws, cx, |w| w.voice.phase == DictationPhase::Idle);
    assert_eq!(composer_value(&ws, cx), "ship it");
}

#[gpui_kit::test]
fn voice_settings_persist(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.voice.enabled = true;
            this.voice.language = "de-DE".into();
            this.voice.on_device = true;
            this.save_settings();
            this.set_voice_language("fr-FR".into(), cx);
            let _ = window;
        });
    });
    let s = crate::persist::load_settings();
    assert!(s.voice_enabled);
    assert_eq!(s.voice_language, "fr-FR");
    assert!(s.voice_on_device);
}

#[gpui_kit::test]
fn voice_section_renders_and_test_mic_runs(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let engine = FakeEngine::new();
    enable_voice(&ws, engine.clone(), cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.open_voice_settings(window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-voice").visible());
        assert!(window.find("toggle-voice").visible());
        assert!(window.find("voice-language").visible());
        assert!(window.find("toggle-voice-device").visible());
        window.click("voice-test-mic", cx);
        window.draw(cx).clear(cx);
    });
    assert_eq!(ws.read_with(cx, |w, _| w.voice.phase), DictationPhase::Recording);
    cx.update(|window, cx| {
        window.click("voice-test-mic", cx);
        window.draw(cx).clear(cx);
    });
    engine.tx().send(DictationEvent::Final(Ok("check one two".into()))).unwrap();
    until(&ws, cx, |w| w.voice.phase == DictationPhase::Idle);
    let result = ws.read_with(cx, |w, _| w.voice.test_result.clone());
    assert_eq!(result.as_deref(), Some("Heard: check one two"));
    // A test take must not touch the composer.
    assert_eq!(composer_value(&ws, cx), "");
}

#[gpui_kit::test]
fn disabling_voice_mid_take_cancels(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let engine = FakeEngine::new();
    enable_voice(&ws, engine.clone(), cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
        ws.update(cx, |this, cx| this.set_voice_enabled(false, window, cx));
    });
    assert_eq!(ws.read_with(cx, |w, _| w.voice.phase), DictationPhase::Idle);
    assert!(engine.inner.lock().cancelled, "session should be cancelled");
}
