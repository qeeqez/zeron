//! Resume past sessions: the sidebar's Resume section lists threads the
//! backend can reopen (codex `thread/list`), and picking one opens a chat
//! bound to that thread so the next send continues it (`thread/resume`).

use gpui_kit::*;

use crate::backend::{ResumedSession, SessionInfo};
use crate::model::Chat;
use crate::workspace::Workspace;

impl Workspace {
    /// Expand/collapse the sidebar's Resume section. Opening kicks off a
    /// `list_sessions` fetch on the background executor.
    pub fn toggle_resume(&mut self, cx: &mut Context<Self>) {
        self.resume_open = !self.resume_open;
        if self.resume_open {
            self.refresh_sessions(cx);
        }
        cx.notify();
    }

    /// Fetch the backend's resumable threads off the UI thread. No-op when
    /// the backend has no session support or a fetch is already running.
    /// Test builds skip the spawn — tests inject via `land_sessions`.
    pub(crate) fn refresh_sessions(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) || self.sessions_loading || !self.backend.supports_sessions() {
            return;
        }
        self.sessions_loading = true;
        let backend = self.backend.clone();
        let task = cx.background_executor().spawn(async move { backend.list_sessions() });
        cx.spawn(async move |this, cx| {
            let sessions = task.await;
            let _ = this.update(cx, |this, cx| this.land_sessions(sessions, cx));
        })
        .detach();
    }

    /// Publish a fetched session list. `None` means the backend declined
    /// mid-flight (provider switched) — the section shows its empty state.
    pub(crate) fn land_sessions(&mut self, sessions: Option<Vec<SessionInfo>>, cx: &mut Context<Self>) {
        self.sessions_loading = false;
        self.sessions = sessions.unwrap_or_default();
        cx.notify();
    }

    /// Open a past session: select its chat when one is already bound,
    /// otherwise create one stamped with the thread id so sends continue
    /// it, then fetch the transcript in the background.
    pub fn open_session(&mut self, session: &SessionInfo, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.chats.iter().position(|c| c.thread_id == session.id) {
            self.select_chat(ix, window, cx);
            return;
        }
        // Same switch bookkeeping as `new_chat`: the outgoing chat keeps
        // its draft and its provider/model/access stamp.
        if let Some(chat) = self.chats.get_mut(self.active) {
            chat.draft = self.composer.read(cx).value().to_string();
        }
        self.stamp_thread();
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        let mut chat = Chat::new(id, session.title.clone());
        chat.thread_id = session.id.clone();
        // The thread ran in its own directory — keep turns there when it
        // still exists (a vanished path falls back to the project root).
        if !session.cwd.is_empty() && std::path::Path::new(&session.cwd).is_dir() {
            chat.workdir = session.cwd.clone();
        }
        self.chats.push(chat);
        self.active = self.chats.len() - 1;
        // Bind the new thread to the live provider/model/access selection.
        self.stamp_thread();
        self.clear_recall();
        self.search_match_ix = 0;
        self.composer.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        self.scroller.update(cx, |s, cx| s.reset(0, cx));
        window.set_window_title(&format!("{} — Rixl Code", session.title));
        cx.notify();
        self.save();

        if cfg!(test) {
            return; // tests land transcripts via `land_session`
        }
        let backend = self.backend.clone();
        let thread_id = session.id.clone();
        let task = cx.background_executor().spawn(async move { backend.resume_session(&thread_id) });
        cx.spawn(async move |this, cx| {
            let resumed = task.await;
            let _ = this.update(cx, |this, cx| this.land_session(resumed, cx));
        })
        .detach();
    }

    /// Land a fetched transcript on the chat bound to its thread. A chat
    /// the user already typed into keeps its messages — the late-arriving
    /// history must not clobber a live turn.
    pub(crate) fn land_session(&mut self, resumed: Option<ResumedSession>, cx: &mut Context<Self>) {
        let Some(resumed) = resumed else { return };
        let Some(chat) = self.chats.iter_mut().find(|c| c.thread_id == resumed.id) else { return };
        if !chat.messages.is_empty() {
            return;
        }
        chat.messages = std::rc::Rc::new(resumed.messages);
        if chat.title == "New chat" && !resumed.title.is_empty() {
            chat.title = resumed.title.into();
        }
        // The thread's own cwd is authoritative — fill it in when the
        // session row didn't carry one.
        if chat.workdir.is_empty() && !resumed.cwd.is_empty() && std::path::Path::new(&resumed.cwd).is_dir() {
            chat.workdir = resumed.cwd.clone();
        }
        if self.chats.get(self.active).is_some_and(|c| c.thread_id == resumed.id) {
            let count = self.filtered_count(cx);
            self.scroller.update(cx, |s, cx| s.reset(count, cx));
        }
        cx.notify();
        self.save();
    }
}
