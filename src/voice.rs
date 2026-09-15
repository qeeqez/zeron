//! Voice dictation: mic capture → speech recognition → composer text.
//!
//! The pipeline is split behind `DictationEngine`/`DictationSession` so the
//! workspace logic (toggle, phase, note, composer insertion) is testable
//! headless with a fake engine, while the real implementation
//! (`crate::voice_apple`) drives AVAudioEngine + SFSpeechRecognizer on macOS.
//!
//! `VoiceState` lives on `Workspace` and is persisted via `persist::Settings`
//! (`voice_enabled`, `voice_language`, `voice_on_device`).

use std::sync::mpsc::Receiver;

use gpui_kit::*;

use crate::workspace::Workspace;

/// Where a dictation take is in its lifecycle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DictationPhase {
    /// No take in flight.
    Idle,
    /// The mic is live and audio is streaming to the recognizer.
    Recording,
    /// Audio ended; the final transcript hasn't landed yet.
    Transcribing,
}

/// Why a take failed — each maps to a user-facing note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VoiceError {
    /// Microphone access was denied or restricted (System Settings →
    /// Privacy & Security → Microphone).
    MicDenied,
    /// Speech recognition was denied or restricted (System Settings →
    /// Privacy & Security → Speech Recognition).
    SpeechDenied,
    /// The recognizer/engine isn't available — no speech service for the
    /// locale, no audio input device, or the engine failed to start.
    Unavailable(String),
    /// The take was cancelled before a final transcript arrived.
    Cancelled,
}

impl VoiceError {
    /// What the composer note shows for this failure.
    pub(crate) fn note(&self) -> String {
        match self {
            Self::MicDenied => "Microphone access denied — allow it in System Settings → Privacy & Security → Microphone.".into(),
            Self::SpeechDenied => {
                "Speech recognition denied — allow it in System Settings → Privacy & Security → Speech Recognition.".into()
            },
            Self::Unavailable(why) => format!("Dictation unavailable: {why}"),
            Self::Cancelled => "Dictation cancelled.".into(),
        }
    }
}

/// One event from a running take. `Partial` carries the recognizer's
/// in-progress transcript; `Final` ends the take — `Ok` is the transcript to
/// insert, `Err` is the failure note.
pub(crate) enum DictationEvent {
    Partial(String),
    Final(Result<String, VoiceError>),
}

/// A live take. `finish` ends the mic and returns the event stream; `cancel`
/// aborts. Dropping without `finish`/`cancel` must stop the mic.
pub(crate) trait DictationSession: Send {
    fn finish(&mut self) -> Receiver<DictationEvent>;
    fn cancel(&mut self);
}

/// Produces dictation takes. `begin` fails fast on already-denied
/// permissions; undetermined permissions are requested inside the session.
pub(crate) trait DictationEngine: Send {
    fn begin(&self, language: &str, on_device: bool) -> Result<Box<dyn DictationSession>, VoiceError>;
}

/// Voice settings + live dictation state for one window.
pub(crate) struct VoiceState {
    /// Master switch — persisted as `Settings.voice_enabled`.
    pub enabled: bool,
    /// BCP-47 locale id for the recognizer; empty = system default.
    pub language: String,
    /// Prefer on-device recognition — persisted as `voice_on_device`.
    pub on_device: bool,
    pub phase: DictationPhase,
    /// Status line under the composer: partial transcript while listening,
    /// failure reason after a take ends badly.
    pub note: Option<String>,
    /// `note` is a failure (red) vs. progress (muted).
    pub note_is_error: bool,
    /// The running take, if any.
    pub session: Option<Box<dyn DictationSession>>,
    /// Where the result lands: the composer, or the settings test row.
    pub test_take: bool,
    /// Last test-mic result shown in the Voice settings section.
    pub test_result: Option<String>,
    engine: Option<Box<dyn DictationEngine>>,
}

impl VoiceState {
    pub(crate) fn new(enabled: bool, language: String, on_device: bool) -> Self {
        Self {
            enabled,
            language,
            on_device,
            phase: DictationPhase::Idle,
            note: None,
            note_is_error: false,
            session: None,
            test_take: false,
            test_result: None,
            engine: None,
        }
    }

    /// Swap in an engine — tests install a fake; production lazily builds
    /// the platform one on first use.
    #[cfg(test)]
    pub(crate) fn set_engine(&mut self, engine: Box<dyn DictationEngine>) {
        self.engine = Some(engine);
    }

    fn engine(&mut self) -> &mut dyn DictationEngine {
        self.engine.get_or_insert_with(default_engine).as_mut()
    }
}

/// The platform engine. macOS uses AVAudioEngine + SFSpeechRecognizer;
/// anything else reports unavailable so the UI still behaves.
#[cfg(target_os = "macos")]
fn default_engine() -> Box<dyn DictationEngine> {
    Box::new(crate::voice_apple::AppleDictation)
}

#[cfg(not(target_os = "macos"))]
fn default_engine() -> Box<dyn DictationEngine> {
    struct Unsupported;
    impl DictationEngine for Unsupported {
        fn begin(&self, _language: &str, _on_device: bool) -> Result<Box<dyn DictationSession>, VoiceError> {
            Err(VoiceError::Unavailable("dictation needs macOS Speech framework".into()))
        }
    }
    Box::new(Unsupported)
}

impl Workspace {
    /// The mic button / Cmd-Shift-D: start a take, or finish the running
    /// one. With voice disabled the button opens the Voice settings section
    /// instead of failing silently.
    pub fn toggle_dictation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.voice.enabled {
            self.open_voice_settings(window, cx);
            return;
        }
        self.start_or_finish_take(false, window, cx);
    }

    /// The settings "Test microphone" button — same pipeline, but the
    /// transcript lands on the test row instead of the composer.
    pub fn toggle_dictation_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_or_finish_take(true, window, cx);
    }

    fn start_or_finish_take(&mut self, test: bool, window: &mut Window, cx: &mut Context<Self>) {
        match self.voice.phase {
            DictationPhase::Recording => self.finish_take(window, cx),
            // A result is already in flight — don't stack takes.
            DictationPhase::Transcribing => {},
            DictationPhase::Idle => {
                self.voice.note = None;
                self.voice.note_is_error = false;
                self.voice.test_take = test;
                if test {
                    self.voice.test_result = None;
                }
                let (language, on_device) = (self.voice.language.clone(), self.voice.on_device);
                match self.voice.engine().begin(&language, on_device) {
                    Ok(session) => {
                        self.voice.session = Some(session);
                        self.voice.phase = DictationPhase::Recording;
                    },
                    Err(e) => {
                        self.voice.note = Some(e.note());
                        self.voice.note_is_error = true;
                    },
                }
                cx.notify();
            },
        }
    }

    /// Stop the mic and wait for the transcript — the second click of the
    /// toggle. The event stream is polled on the test clock, so headless
    /// tests drive it with `run_until_parked`/`advance_clock`.
    fn finish_take(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut session) = self.voice.session.take() else { return };
        let events = session.finish();
        self.voice.phase = DictationPhase::Transcribing;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            while !poll_dictation(&events, &this, cx) {
                cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
            }
        })
        .detach();
    }

    /// Abort a take without inserting — used when voice is switched off
    /// mid-recording.
    pub(crate) fn cancel_dictation(&mut self, cx: &mut Context<Self>) {
        if let Some(mut session) = self.voice.session.take() {
            session.cancel();
        }
        self.voice.phase = DictationPhase::Idle;
        self.voice.note = None;
        cx.notify();
    }

    /// One event from the take's stream: partials update the status line,
    /// the final transcript inserts at the composer cursor (or the test row).
    fn land_dictation(&mut self, event: DictationEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            DictationEvent::Partial(text) => {
                if self.voice.test_take {
                    self.voice.test_result = Some(format!("Listening: {text}"));
                } else {
                    self.voice.note = Some(format!("Listening: {text}"));
                    self.voice.note_is_error = false;
                }
            },
            DictationEvent::Final(Ok(text)) => self.land_transcript(text.trim(), window, cx),
            DictationEvent::Final(Err(e)) => {
                self.voice.phase = DictationPhase::Idle;
                if self.voice.test_take {
                    self.voice.test_result = Some(e.note());
                } else {
                    self.voice.note = Some(e.note());
                    self.voice.note_is_error = true;
                }
            },
        }
        cx.notify();
    }

    /// The take's final transcript: test takes report on the settings row,
    /// real takes insert at the composer cursor.
    fn land_transcript(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.voice.phase = DictationPhase::Idle;
        if self.voice.test_take {
            let heard = if text.is_empty() { "Heard silence — try again.".to_string() } else { format!("Heard: {text}") };
            self.voice.test_result = Some(heard);
            return;
        }
        self.voice.note = None;
        self.insert_dictated(text, window, cx);
    }

    /// Open settings on the Voice section — the mic button's affordance when
    /// dictation is disabled.
    pub(crate) fn open_voice_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(window, cx);
        self.settings_panel.update(cx, |panel, cx| {
            panel.section = crate::views::settings_nav::Section::Voice;
            cx.notify();
        });
    }

    /// Insert a transcript at the composer cursor, separating it from
    /// existing text with a space.
    fn insert_dictated(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        let needs_space = self.composer.read(cx).value().chars().next_back().is_some_and(|c| !c.is_whitespace());
        let text = if needs_space { format!(" {text}") } else { text.to_string() };
        self.composer.update(cx, |composer, cx| composer.insert(text, window, cx));
    }

    /// `toggle_row` setter for the Voice enable switch — disabling mid-take
    /// cancels the recording.
    pub(crate) fn set_voice_enabled(&mut self, on: bool, _window: &mut Window, cx: &mut Context<Self>) {
        self.voice.enabled = on;
        if !on {
            self.cancel_dictation(cx);
        }
    }

    /// Select Confirm handler for the dictation language.
    pub(crate) fn set_voice_language(&mut self, language: String, cx: &mut Context<Self>) {
        self.voice.language = language;
        self.save_settings();
        cx.notify();
    }

    /// `toggle_row` setter for on-device recognition.
    pub(crate) fn set_voice_on_device(&mut self, on: bool, _window: &mut Window, _cx: &mut Context<Self>) {
        self.voice.on_device = on;
    }
}

/// One poll of the take's event stream from the `spawn_in` task. Returns
/// `true` once the stream is done (final event landed or the sender hung
/// up) so the caller stops polling.
fn poll_dictation(events: &Receiver<DictationEvent>, this: &WeakEntity<Workspace>, cx: &mut AsyncWindowContext) -> bool {
    match events.try_recv() {
        Ok(event) => {
            let done = matches!(event, DictationEvent::Final(_));
            let _ = this.update_in(cx, |ws, window, cx| ws.land_dictation(event, window, cx));
            done
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => false,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            let _ = this.update_in(cx, |ws, window, cx| {
                ws.land_dictation(DictationEvent::Final(Err(VoiceError::Unavailable("recognizer stopped".into()))), window, cx)
            });
            true
        },
    }
}
