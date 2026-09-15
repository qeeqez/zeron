//! The real dictation engine: AVAudioEngine taps the mic into an
//! `SFSpeechAudioBufferRecognitionRequest`, and `SFSpeechRecognizer` streams
//! partial + final transcripts back over the session's channel.
//!
//! Everything Objective-C lives on one worker thread per take: permission
//! prompts are async, so the worker blocks on them while the UI stays live.
//! The tap callback (CoreAudio's thread) only appends buffers to the
//! request; the result handler (Speech's queue) forwards events. `finish`
//! flips `stop` → the worker calls `endAudio` and waits for the final
//! result; `cancel` tears the pipeline down.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use objc2_avf_audio::AVAudioEngine;
use objc2_foundation::{NSLocale, NSString};
use objc2_speech::{
    SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionTask, SFSpeechRecognizer, SFSpeechRecognizerAuthorizationStatus,
};

use crate::voice::{DictationEngine, DictationEvent, DictationSession, VoiceError};

/// What the UI asked the worker to do.
const RUNNING: u8 = 0;
const FINISH: u8 = 1;
const CANCEL: u8 = 2;

/// Cap on a single take — Speech itself stops tasks around a minute, so
/// this only guards against a hung recognizer.
const TAKE_LIMIT: Duration = Duration::from_secs(300);
/// How long the worker waits on a permission prompt before giving up.
const PROMPT_LIMIT: Duration = Duration::from_secs(120);

/// macOS dictation via the Speech framework.
pub(crate) struct AppleDictation;

impl DictationEngine for AppleDictation {
    fn begin(&self, language: &str, on_device: bool) -> Result<Box<dyn DictationSession>, VoiceError> {
        // Fail fast on already-decided denials; undetermined states are
        // requested on the worker so the prompts don't block the UI.
        match mic_status() {
            AVAuthorizationStatus::Denied | AVAuthorizationStatus::Restricted => return Err(VoiceError::MicDenied),
            _ => {},
        }
        match unsafe { SFSpeechRecognizer::authorizationStatus() } {
            SFSpeechRecognizerAuthorizationStatus::Denied | SFSpeechRecognizerAuthorizationStatus::Restricted => {
                return Err(VoiceError::SpeechDenied);
            },
            _ => {},
        }
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicU8::new(RUNNING));
        let worker = {
            let stop = stop.clone();
            let language = language.to_string();
            std::thread::spawn(move || run_take(&language, on_device, &stop, &tx))
        };
        Ok(Box::new(AppleSession { stop, events: Some(rx), worker: Some(worker) }))
    }
}

/// One live take. `finish` hands the event stream to the UI; the worker
/// thread keeps running until the final transcript (or an error) lands.
struct AppleSession {
    stop: Arc<AtomicU8>,
    events: Option<Receiver<DictationEvent>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl DictationSession for AppleSession {
    fn finish(&mut self) -> Receiver<DictationEvent> {
        self.stop.store(FINISH, Ordering::SeqCst);
        self.events.take().unwrap_or_else(|| channel().1)
    }

    fn cancel(&mut self) {
        self.stop.store(CANCEL, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for AppleSession {
    fn drop(&mut self) {
        // A session dropped without finish/cancel still has to stop the mic.
        self.stop.store(CANCEL, Ordering::SeqCst);
    }
}

fn mic_status() -> AVAuthorizationStatus {
    let Some(media) = (unsafe { AVMediaTypeAudio }) else { return AVAuthorizationStatus::Authorized };
    unsafe { AVCaptureDevice::authorizationStatusForMediaType(media) }
}

/// Block the worker on a permission prompt; `false` on denial, cancel, or
/// timeout. The completion handler can fire on any queue.
fn await_permission(request: impl FnOnce(Sender<bool>), stop: &AtomicU8) -> bool {
    let (tx, rx) = channel();
    request(tx);
    let deadline = Instant::now() + PROMPT_LIMIT;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(granted) => return granted,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if stop.load(Ordering::SeqCst) != RUNNING || Instant::now() > deadline {
                    return false;
                }
            },
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// Ensure mic + speech permission, prompting when undetermined.
fn ensure_permissions(stop: &AtomicU8) -> Result<(), VoiceError> {
    if mic_status() == AVAuthorizationStatus::NotDetermined {
        let granted = await_permission(
            |tx| unsafe {
                let Some(media) = AVMediaTypeAudio else {
                    let _ = tx.send(true);
                    return;
                };
                let block = RcBlock::new(move |granted: Bool| {
                    let _ = tx.send(granted.as_bool());
                });
                AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &block);
            },
            stop,
        );
        if !granted {
            return Err(if stop.load(Ordering::SeqCst) == CANCEL { VoiceError::Cancelled } else { VoiceError::MicDenied });
        }
    }
    if unsafe { SFSpeechRecognizer::authorizationStatus() } == SFSpeechRecognizerAuthorizationStatus::NotDetermined {
        let granted = await_permission(
            |tx| unsafe {
                let block = RcBlock::new(move |status: SFSpeechRecognizerAuthorizationStatus| {
                    let _ = tx.send(status == SFSpeechRecognizerAuthorizationStatus::Authorized);
                });
                SFSpeechRecognizer::requestAuthorization(&block);
            },
            stop,
        );
        if !granted {
            return Err(if stop.load(Ordering::SeqCst) == CANCEL { VoiceError::Cancelled } else { VoiceError::SpeechDenied });
        }
    }
    Ok(())
}

/// The take's whole lifecycle on one thread: permissions → engine + tap →
/// recognition task → wait for finish/cancel → teardown. Sends exactly one
/// `Final` event.
fn run_take(language: &str, on_device: bool, stop: &AtomicU8, tx: &Sender<DictationEvent>) {
    let result = ensure_permissions(stop)
        .and_then(|()| {
            if stop.load(Ordering::SeqCst) == CANCEL {
                Err(VoiceError::Cancelled)
            } else {
                start_pipeline(language, on_device, tx)
            }
        })
        .map(|live| drive(live, stop, tx));
    if let Err(e) = result {
        let _ = tx.send(DictationEvent::Final(Err(e)));
    }
}

/// Everything the take needs alive until the final result: the engine (tap
/// installed), the request the tap appends to, and the recognition task.
struct Live {
    engine: Retained<AVAudioEngine>,
    request: Retained<SFSpeechAudioBufferRecognitionRequest>,
    task: Retained<SFSpeechRecognitionTask>,
    /// Set by the result handler once it sent `Final` — the worker's cue to
    /// tear down without waiting for `stop`.
    done: Arc<AtomicBool>,
}

fn start_pipeline(language: &str, on_device: bool, tx: &Sender<DictationEvent>) -> Result<Live, VoiceError> {
    // SAFETY: every Speech/AVFAudio call below is confined to this worker
    // thread; the tap block only appends buffers to `request`, which Apple
    // documents as callable from the tap's render thread.
    unsafe {
        let recognizer = if language.is_empty() {
            SFSpeechRecognizer::new()
        } else {
            let locale = NSLocale::initWithLocaleIdentifier(objc2::AllocAnyThread::alloc(), &NSString::from_str(language));
            SFSpeechRecognizer::initWithLocale(objc2::AllocAnyThread::alloc(), &locale)
                .ok_or_else(|| VoiceError::Unavailable(format!("no recognizer for {language}")))?
        };
        if !recognizer.isAvailable() {
            return Err(VoiceError::Unavailable("speech service is unavailable".into()));
        }
        if on_device && !recognizer.supportsOnDeviceRecognition() {
            return Err(VoiceError::Unavailable("on-device recognition isn't supported for this language".into()));
        }

        let request = SFSpeechAudioBufferRecognitionRequest::new();
        request.setShouldReportPartialResults(true);
        request.setAddsPunctuation(true);
        request.setRequiresOnDeviceRecognition(on_device);

        let done = Arc::new(AtomicBool::new(false));
        let handler = {
            let tx = tx.clone();
            let done = done.clone();
            RcBlock::new(move |result: *mut objc2_speech::SFSpeechRecognitionResult, error: *mut objc2_foundation::NSError| {
                if let Some(result) = result.as_ref() {
                    let text = result.bestTranscription().formattedString().to_string();
                    if result.isFinal() {
                        done.store(true, Ordering::SeqCst);
                        let _ = tx.send(DictationEvent::Final(Ok(text)));
                    } else if !text.is_empty() {
                        let _ = tx.send(DictationEvent::Partial(text));
                    }
                }
                if let Some(error) = error.as_ref()
                    && !done.swap(true, Ordering::SeqCst)
                {
                    let _ = tx.send(DictationEvent::Final(Err(VoiceError::Unavailable(error.localizedDescription().to_string()))));
                }
            })
        };
        let task = recognizer.recognitionTaskWithRequest_resultHandler(&request, &handler);

        let engine = AVAudioEngine::new();
        let input = engine.inputNode();
        let format = input.outputFormatForBus(0);
        let tap = {
            let request = request.clone();
            RcBlock::new(
                move |buffer: std::ptr::NonNull<objc2_avf_audio::AVAudioPCMBuffer>,
                      _when: std::ptr::NonNull<objc2_avf_audio::AVAudioTime>| {
                    request.appendAudioPCMBuffer(buffer.as_ref());
                },
            )
        };
        input.installTapOnBus_bufferSize_format_block(0, 1024, Some(&format), RcBlock::as_ptr(&tap));
        engine.prepare();
        engine
            .startAndReturnError()
            .map_err(|e| VoiceError::Unavailable(e.localizedDescription().to_string()))?;
        Ok(Live { engine, request, task, done })
    }
}
fn drive(live: Live, stop: &AtomicU8, tx: &Sender<DictationEvent>) {
    let deadline = Instant::now() + TAKE_LIMIT;
    let mut ended = false;
    loop {
        match stop.load(Ordering::SeqCst) {
            CANCEL => {
                unsafe {
                    live.task.cancel();
                    live.engine.stop();
                    live.engine.inputNode().removeTapOnBus(0);
                }
                let _ = tx.send(DictationEvent::Final(Err(VoiceError::Cancelled)));
                return;
            },
            // `endAudio` once — it tells the recognizer no more audio is
            // coming; the final result then arrives via the handler.
            FINISH if !ended => {
                unsafe { live.request.endAudio() };
                ended = true;
            },
            _ => {},
        }
        if live.done.load(Ordering::SeqCst) {
            break;
        }
        if Instant::now() > deadline {
            let _ = tx.send(DictationEvent::Final(Err(VoiceError::Unavailable("recognition timed out".into()))));
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    unsafe {
        live.engine.stop();
        live.engine.inputNode().removeTapOnBus(0);
    }
}
