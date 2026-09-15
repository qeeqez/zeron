//! Workspace auth operations: state reads, the sign-in/out flows, the
//! login-event poller, and the send gate. Split from `auth.rs` (state +
//! persistence) to stay under the SLOC cap.

use gpui_kit::*;

use crate::auth::{AuthEvent, AuthState, LoginSession, can_sign_in, env_auth, env_key, login_flow, logout_flow, probe, save_auth_cache};
use crate::workspace::Workspace;

impl Workspace {
    /// The instance's auth state. Env-keyed kinds report live (the probe
    /// is free); the rest read the last landed state — `Unknown` until the
    /// first refresh lands.
    pub fn auth_state(&self, instance_id: &str) -> AuthState {
        let Some(p) = self.providers.iter().find(|p| p.id == instance_id) else {
            return AuthState::Unknown;
        };
        if let Some(key) = env_key(p) {
            return env_auth(key);
        }
        if !can_sign_in(p.kind) {
            return AuthState::NotRequired;
        }
        self.auth.states.get(instance_id).cloned().unwrap_or(AuthState::Unknown)
    }

    /// The last login error for the detail panel, if any.
    pub(crate) fn auth_error(&self, instance_id: &str) -> Option<&str> {
        self.auth.errors.get(instance_id).map(String::as_str)
    }

    /// Land a probed state: update the map, persist the stable states,
    /// and clear a stale login error on success.
    pub(crate) fn land_auth(&mut self, instance_id: &str, state: AuthState, cx: &mut Context<Self>) {
        if matches!(state, AuthState::SignedIn(_)) {
            self.auth.errors.remove(instance_id);
        }
        self.auth.states.insert(instance_id.to_string(), state);
        save_auth_cache(&self.auth.states);
        cx.notify();
    }

    /// Probe every instance off the UI thread — one task per instance,
    /// like `refresh_model_catalogs`. Test builds skip the spawn; tests
    /// inject states via `land_auth`.
    pub(crate) fn refresh_auth(&self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        for p in &self.providers {
            // Env kinds probe live at read time; an in-flight login owns
            // its instance's state until it lands `Done`.
            if env_key(p).is_some() || !can_sign_in(p.kind) || self.auth.sessions.contains_key(&p.id) {
                continue;
            }
            let instance = p.clone();
            let task = cx.background_executor().spawn(async move { probe(&instance) });
            let id = p.id.clone();
            cx.spawn(async move |this, cx| {
                let state = task.await;
                let _ = this.update(cx, |this, cx| this.land_auth(&id, state, cx));
            })
            .detach();
        }
    }

    /// Start the instance's login flow. The worker thread reports
    /// `AuthEvent`s; `poll_login` lands them until `Done`.
    pub fn sign_in(&mut self, instance_id: &str, cx: &mut Context<Self>) {
        let Some(p) = self.providers.iter().find(|p| p.id == instance_id).cloned() else { return };
        if !can_sign_in(p.kind) || self.auth.sessions.contains_key(&p.id) {
            return;
        }
        self.auth.errors.remove(&p.id);
        self.auth.states.insert(p.id.clone(), AuthState::SigningIn(String::new()));
        // Test builds never spawn real subprocesses — a stub session keeps
        // the flow cancellable and the row shows "Signing in…"; flow logic
        // is unit-tested on canned output instead.
        if cfg!(test) {
            let (_tx, rx) = std::sync::mpsc::channel();
            self.auth.sessions.insert(
                p.id.clone(),
                LoginSession {
                    events: rx,
                    child: std::sync::Arc::new(parking_lot::Mutex::new(None)),
                    stdin: None,
                },
            );
        } else {
            let (tx, rx) = std::sync::mpsc::channel();
            match login_flow(p.kind, tx) {
                Some(Ok(handle)) => {
                    self.auth
                        .sessions
                        .insert(p.id.clone(), LoginSession { events: rx, child: handle.child, stdin: handle.stdin });
                    self.poll_login(p.id.clone(), cx);
                },
                Some(Err(e)) => {
                    self.auth.errors.insert(p.id.clone(), e);
                    self.land_auth(&p.id, AuthState::SignedOut, cx);
                },
                None => {},
            }
        }
        cx.notify();
    }

    /// Drain a login session's events on a timer — the same poll loop
    /// `spawn_queue_drain` uses. Ends on `Done` or when the session is
    /// gone (cancel/sign-out removed it).
    fn poll_login(&mut self, instance_id: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| poll_login_loop(this, instance_id, cx).await).detach();
    }

    /// Land a batch of login events; `true` once the flow ended.
    fn apply_auth_events(&mut self, instance_id: &str, events: Vec<AuthEvent>, cx: &mut Context<Self>) -> bool {
        let mut done = false;
        for e in events {
            done |= self.apply_auth_event(instance_id, e, cx);
        }
        done
    }

    /// Pull every pending event off the session's channel — `None` when
    /// the session is gone; a disconnect lands `Done(Unknown)` so the
    /// poller stops and the next probe re-checks.
    fn drain_auth_events(&self, instance_id: &str) -> Option<Vec<AuthEvent>> {
        let session = self.auth.sessions.get(instance_id)?;
        let mut events = Vec::new();
        loop {
            match session.events.try_recv() {
                Ok(e) => events.push(e),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    events.push(AuthEvent::Done(AuthState::Unknown));
                    break;
                },
            }
        }
        Some(events)
    }

    /// Land one login event; `true` when the flow ended.
    fn apply_auth_event(&mut self, instance_id: &str, event: AuthEvent, cx: &mut Context<Self>) -> bool {
        match event {
            AuthEvent::Prompt(text) => {
                self.auth.states.insert(instance_id.to_string(), AuthState::SigningIn(text));
                cx.notify();
                false
            },
            AuthEvent::NeedsCode(text) => {
                self.auth.states.insert(instance_id.to_string(), AuthState::AwaitingCode(text));
                cx.notify();
                false
            },
            AuthEvent::Failed(msg) => {
                self.auth.errors.insert(instance_id.to_string(), msg);
                cx.notify();
                false
            },
            AuthEvent::Done(state) => {
                self.auth.sessions.remove(instance_id);
                self.land_auth(instance_id, state, cx);
                true
            },
        }
    }

    /// Abort an in-flight login — dropping the session kills the child.
    /// The state goes back to `Unknown` until the next probe.
    pub fn cancel_sign_in(&mut self, instance_id: &str, cx: &mut Context<Self>) {
        if self.auth.sessions.remove(instance_id).is_none() {
            return;
        }
        self.auth.states.insert(instance_id.to_string(), AuthState::Unknown);
        self.refresh_auth(cx);
        cx.notify();
    }

    /// Paste-back for flows that need a code (claude): write it to the
    /// child's stdin and flip the row back to "Signing in…".
    pub fn submit_auth_code(&mut self, instance_id: &str, code: &str, cx: &mut Context<Self>) {
        let Some(session) = self.auth.sessions.get(instance_id) else { return };
        let Some(stdin) = &session.stdin else { return };
        let code = code.trim();
        if code.is_empty() {
            return;
        }
        if let Some(stdin) = stdin.lock().as_mut() {
            use std::io::Write;
            let _ = writeln!(stdin, "{code}");
            let _ = stdin.flush();
        }
        self.auth.states.insert(instance_id.to_string(), AuthState::SigningIn("Submitting code…".into()));
        cx.notify();
    }

    /// Sign the instance out: cancel any in-flight login, run the kind's
    /// logout off the UI thread, and land `SignedOut` — the provider's
    /// own state is the source of truth, so a failed logout still shows
    /// signed-out only when the next probe agrees.
    pub fn sign_out(&mut self, instance_id: &str, cx: &mut Context<Self>) {
        let Some(p) = self.providers.iter().find(|p| p.id == instance_id).cloned() else { return };
        self.auth.sessions.remove(&p.id);
        self.auth.errors.remove(&p.id);
        let Some(logout) = logout_flow(p.kind) else { return };
        self.land_auth(&p.id, AuthState::SignedOut, cx);
        if cfg!(test) {
            return;
        }
        cx.background_executor()
            .spawn(async move {
                let _ = logout();
            })
            .detach();
    }

    /// Sign out every provider that has a logout flow — the Profile
    /// section's "Sign out all". Env-keyed credentials aren't sessions, so
    /// they're untouched (the env var stays set).
    pub fn sign_out_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.providers.iter().filter(|p| logout_flow(p.kind).is_some()).map(|p| p.id.clone()).collect();
        for id in ids {
            self.sign_out(&id, cx);
        }
    }

    /// `Some(reason)` when the selected provider can't take a send — the
    /// caller shows the reason instead of spawning a turn. Only a known
    /// `SignedOut` gates; `Unknown` stays permissive so a broken status
    /// probe can't lock the user out.
    pub(crate) fn auth_block_note(&self) -> Option<String> {
        let p = self.providers.iter().find(|p| p.id == self.selected_provider)?;
        if let Some(key) = env_key(p) {
            return match std::env::var(key) {
                Ok(v) if !v.is_empty() => None,
                _ => Some(format!("{key} isn't set — export it, then send again.")),
            };
        }
        match self.auth_state(&p.id) {
            AuthState::SignedOut => Some(format!("{} isn't signed in — sign in from Settings → Providers.", p.name)),
            AuthState::SigningIn(_) | AuthState::AwaitingCode(_) => {
                Some(format!("{} sign-in is in progress — finish it in Settings → Providers.", p.name))
            },
            _ => None,
        }
    }
}

/// The login poller's loop — a free fn so the spawn closure stays flat.
/// Ends on `Done`, a dropped session, or a dropped workspace.
async fn poll_login_loop(this: WeakEntity<Workspace>, instance_id: String, cx: &mut AsyncApp) {
    loop {
        let events = this.update(cx, |this, _| this.drain_auth_events(&instance_id));
        let Ok(Some(events)) = events else { return };
        let done = this.update(cx, |this, cx| this.apply_auth_events(&instance_id, events, cx)).unwrap_or(true);
        if done {
            return;
        }
        cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
    }
}
