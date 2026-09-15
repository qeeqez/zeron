//! Provider sign-in: per-instance auth state, the login/logout flows, and
//! the send gate.
//!
//! Each provider kind reports auth differently: codex answers
//! `account/read` over `codex app-server` (falling back to
//! `codex login status`), claude answers `claude auth status --json`, and
//! the env-keyed kinds (acp/http) are "signed in" when their `key_env` var
//! is set. Probes run on background threads and land via `land_auth`; the
//! last-known states persist to `auth.json` so the settings rows render
//! before the first probe returns.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use gpui_kit::*;

mod ops;

#[cfg(test)]
mod auth_tests;

use crate::providers::{ProviderInstance, ProviderKind};

/// One provider instance's sign-in state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthState {
    /// Never probed (or the probe failed) — treated as signed-in so a
    /// flaky status check can't lock the user out.
    Unknown,
    /// The kind needs no credentials (sim, or env-keyed kinds with no
    /// `key_env` configured).
    NotRequired,
    /// Credentials present; `detail` is the account label ("ChatGPT
    /// Plus", an email, the env var name).
    SignedIn(String),
    /// The provider needs credentials and has none — sends are gated.
    SignedOut,
    /// A login flow is running; the string is the instruction to show
    /// (device URL + code, or a generic "check your browser").
    SigningIn(String),
    /// The flow needs a pasted code (claude's paste-back); the string
    /// carries the sign-in URL/instructions.
    AwaitingCode(String),
}

impl AuthState {
    /// The provider row's second line — `None` falls back to the kind's
    /// tagline (unknown/not-required states don't crowd the row).
    pub(crate) fn row_status(&self) -> Option<String> {
        match self {
            Self::Unknown | Self::NotRequired => None,
            Self::SignedIn(detail) if detail.is_empty() => Some("Authenticated".into()),
            Self::SignedIn(detail) => Some(format!("Authenticated · {detail}")),
            Self::SignedOut => Some("Not signed in".into()),
            Self::SigningIn(_) => Some("Signing in…".into()),
            Self::AwaitingCode(_) => Some("Waiting for sign-in code…".into()),
        }
    }

    /// The detail panel's status line — always a full sentence.
    pub(crate) fn detail_status(&self) -> String {
        match self {
            Self::NotRequired => "No credentials required".into(),
            Self::Unknown => "Sign-in status unknown".into(),
            _ => self.row_status().unwrap_or_default(),
        }
    }
}

/// The persisted shape — only stable states are cached; in-flight login
/// states never reach disk.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedAuth {
    signed_in: bool,
    #[serde(default)]
    detail: String,
}

impl CachedAuth {
    fn from_state(state: &AuthState) -> Option<Self> {
        match state {
            AuthState::SignedIn(detail) => Some(Self { signed_in: true, detail: detail.clone() }),
            AuthState::SignedOut => Some(Self { signed_in: false, detail: String::new() }),
            _ => None,
        }
    }

    fn into_state(self) -> AuthState {
        if self.signed_in { AuthState::SignedIn(self.detail) } else { AuthState::SignedOut }
    }
}

/// Progress from a login worker thread to the UI.
pub(crate) enum AuthEvent {
    /// The flow produced user-facing instructions (device URL + code).
    Prompt(String),
    /// The flow needs the user to paste a code back (claude).
    NeedsCode(String),
    /// The flow failed before completing — the message lands on the
    /// detail panel; `Done` follows with the re-probed state.
    Failed(String),
    /// The flow ended; carries the re-probed auth state.
    Done(AuthState),
}

/// What a backend's `login` returns: the spawned child's slots. `stdin`
/// is `Some` only for flows that take a pasted code.
pub(crate) struct LoginHandle {
    pub child: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    pub stdin: Option<std::sync::Arc<parking_lot::Mutex<Option<std::process::ChildStdin>>>>,
}

/// A live login flow: the event channel the poller drains plus the child
/// slots. Dropping it kills the child — that's how Cancel works.
pub(crate) struct LoginSession {
    pub events: Receiver<AuthEvent>,
    pub child: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    pub stdin: Option<std::sync::Arc<parking_lot::Mutex<Option<std::process::ChildStdin>>>>,
}

impl Drop for LoginSession {
    fn drop(&mut self) {
        crate::backend::kill_slot(&self.child);
    }
}

/// The workspace's auth bookkeeping: last-known states, in-flight login
/// sessions, and the last login error per instance.
#[derive(Default)]
pub(crate) struct AuthBook {
    pub(crate) states: HashMap<String, AuthState>,
    pub(crate) sessions: HashMap<String, LoginSession>,
    pub(crate) errors: HashMap<String, String>,
}

impl AuthBook {
    /// Seed from the persisted cache — real probes land over it.
    pub(crate) fn seeded() -> Self {
        Self { states: load_auth_cache(), ..Default::default() }
    }
}

/// The env var a kind reads its credentials from — `None` for kinds with
/// a real login flow or no auth at all.
pub(crate) fn env_key(p: &ProviderInstance) -> Option<&str> {
    match p.kind {
        ProviderKind::Acp | ProviderKind::Http if !p.key_env.is_empty() => Some(p.key_env.as_str()),
        _ => None,
    }
}

/// Whether the kind has a sign-in flow the Sign-in button can start.
pub(crate) fn can_sign_in(kind: ProviderKind) -> bool {
    matches!(kind, ProviderKind::CodexCli | ProviderKind::ClaudeCli)
}

/// Env-var presence check for the key-based kinds — the only auth probe
/// that runs synchronously (it's free).
pub(crate) fn env_auth(key_env: &str) -> AuthState {
    match std::env::var(key_env) {
        Ok(v) if !v.is_empty() => AuthState::SignedIn(key_env.to_string()),
        _ => AuthState::SignedOut,
    }
}

/// Blocking probe for one instance — call off the UI thread.
pub(crate) fn probe(p: &ProviderInstance) -> AuthState {
    if let Some(key) = env_key(p) {
        return env_auth(key);
    }
    match p.kind {
        ProviderKind::CodexCli => crate::backend::codex_auth_status(),
        ProviderKind::ClaudeCli => crate::backend::claude_auth_status(),
        _ => AuthState::NotRequired,
    }
}

/// Spawn the kind's login flow. `tx` carries progress back to the UI.
pub(crate) fn login_flow(kind: ProviderKind, tx: Sender<AuthEvent>) -> Option<Result<LoginHandle, String>> {
    match kind {
        ProviderKind::CodexCli => Some(crate::backend::codex_login(tx)),
        ProviderKind::ClaudeCli => Some(crate::backend::claude_login(tx)),
        _ => None,
    }
}

/// A kind's logout — a plain fn so `sign_out` can run it on the
/// background executor.
type Logout = fn() -> Result<(), String>;

/// The kind's logout, if it has one.
pub(crate) fn logout_flow(kind: ProviderKind) -> Option<Logout> {
    match kind {
        ProviderKind::CodexCli => Some(crate::backend::codex_logout),
        ProviderKind::ClaudeCli => Some(crate::backend::claude_logout),
        _ => None,
    }
}

fn auth_cache_path() -> std::path::PathBuf {
    crate::persist::dirs_home().join(".rixl/rixlcode/auth.json")
}

/// Last-known auth states keyed by instance id; empty on any error.
pub(crate) fn load_auth_cache() -> HashMap<String, AuthState> {
    std::fs::read_to_string(auth_cache_path())
        .ok()
        .and_then(|s| serde_json::from_str::<HashMap<String, CachedAuth>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(id, c)| (id, c.into_state()))
        .collect()
}

/// Persist the stable states (atomic tmp+rename, like settings.json).
pub(crate) fn save_auth_cache(states: &HashMap<String, AuthState>) {
    let path = auth_cache_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let cached: HashMap<&String, CachedAuth> = states.iter().filter_map(|(id, s)| CachedAuth::from_state(s).map(|c| (id, c))).collect();
    if let Ok(json) = serde_json::to_string_pretty(&cached) {
        let tmp = path.with_extension("json.tmp");
        let _ = std::fs::write(&tmp, json);
        let _ = std::fs::rename(&tmp, &path);
    }
}
