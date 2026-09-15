//! Read-aloud for assistant messages via macOS `say`.

use std::process::{Child, Command};

use parking_lot::Mutex;

use crate::model::MessageKind;
use crate::workspace::Workspace;

/// The single in-flight `say` process — read-aloud is a toggle, so a new
/// click kills whatever is speaking. Finished children are reaped on the
/// next click via `try_wait`.
static SPEECH: Mutex<Option<Child>> = Mutex::new(None);

impl Workspace {
    /// Read message `ix` aloud via macOS `say`; clicking again stops it.
    pub fn speak_message(&self, ix: usize) {
        let Some(msg) = self.chats[self.active].messages.get(ix) else { return };
        let MessageKind::Text(text) = &msg.kind else { return };
        let mut slot = SPEECH.lock();
        if let Some(mut child) = slot.take()
            && child.try_wait().ok().flatten().is_none()
        {
            let _ = child.kill();
            let _ = child.wait(); // reap — kill alone leaves a zombie
            return;
        }
        if let Ok(child) = Command::new("say").arg(&**text).spawn() {
            *slot = Some(child);
        }
    }
}
