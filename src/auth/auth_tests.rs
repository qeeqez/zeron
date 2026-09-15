//! Auth-state tests: the workspace's sign-in/sign-out bookkeeping, the
//! persisted cache, the env-keyed kinds' live probe, and the send gate.
//! No real subprocess runs — probes are injected via `land_auth` and the
//! login flows are unit-tested on canned output in the backend tests.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::auth::{AuthState, load_auth_cache, save_auth_cache};
use crate::providers::ProviderKind;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats/auth reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-auth-test-{}", std::process::id()));
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

/// The seeded codex instance's id — the default providers carry one per kind.
fn codex_id(ws: &Workspace) -> String {
    ws.providers.iter().find(|p| p.kind == ProviderKind::CodexCli).unwrap().id.clone()
}

#[test]
fn landed_states_roundtrip_through_the_cache() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let id = cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let id = codex_id(w);
            w.land_auth(&id, AuthState::SignedIn("ChatGPT Plus".into()), cx);
            id
        })
    });
    // The cache file holds the stable state; a fresh workspace would seed it.
    let cached = load_auth_cache();
    assert_eq!(cached.get(&id), Some(&AuthState::SignedIn("ChatGPT Plus".into())));
    // In-flight states never persist.
    let mut states = std::collections::HashMap::new();
    states.insert("x".to_string(), AuthState::SigningIn("…".into()));
    states.insert("y".to_string(), AuthState::SignedOut);
    save_auth_cache(&states);
    let reloaded = load_auth_cache();
    assert_eq!(reloaded.get("x"), None);
    assert_eq!(reloaded.get("y"), Some(&AuthState::SignedOut));
}

#[test]
fn sign_in_marks_signing_in_and_cancel_restores() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let id = codex_id(w);
            w.land_auth(&id, AuthState::SignedOut, cx);
            w.sign_in(&id, cx);
            assert!(matches!(w.auth_state(&id), AuthState::SigningIn(_)), "sign-in starts the flow");
            w.cancel_sign_in(&id, cx);
            assert_eq!(w.auth_state(&id), AuthState::Unknown, "cancel drops back to unknown");
        });
    });
}

#[test]
fn sign_out_clears_the_state() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let id = codex_id(w);
            w.land_auth(&id, AuthState::SignedIn("ChatGPT Plus".into()), cx);
            w.sign_out(&id, cx);
            assert_eq!(w.auth_state(&id), AuthState::SignedOut);
        })
    });
    assert_eq!(load_auth_cache().values().next(), Some(&AuthState::SignedOut));
}

#[test]
fn signed_out_provider_gates_the_send() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |w, cx| {
            let id = codex_id(w);
            w.land_auth(&id, AuthState::SignedOut, cx);
            w.composer.update(cx, |c, cx| c.set_value("hello", window, cx));
            w.send(window, cx);
            let chat = &w.chats[w.active];
            assert!(!chat.running, "a gated send never starts a turn");
            let last = chat.messages.last().unwrap();
            let crate::model::MessageKind::Text(t) = &last.kind else { panic!("expected a note") };
            assert!(t.contains("isn't signed in"), "the note prompts sign-in: {t}");
        });
    });
}

#[test]
fn unknown_state_does_not_gate() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |w, _| {
            // Never probed — the gate stays permissive so a flaky status
            // check can't lock the user out.
            assert_eq!(w.auth_block_note(), None);
        });
    });
}

#[test]
fn env_keyed_providers_report_live() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |w, _| {
            let http = w.providers.iter().find(|p| p.kind == ProviderKind::Http).unwrap().clone();
            // SAFETY: nextest isolates each test in its own process.
            unsafe { std::env::remove_var(&http.key_env) };
            assert_eq!(w.auth_state(&http.id), AuthState::SignedOut);
            assert!(w.auth_block_note().is_none(), "http isn't selected — no gate");
            unsafe { std::env::set_var(&http.key_env, "tok") };
            assert_eq!(w.auth_state(&http.id), AuthState::SignedIn(http.key_env.clone()));
            unsafe { std::env::remove_var(&http.key_env) };
        });
    });
}

#[test]
fn row_status_labels() {
    assert_eq!(AuthState::SignedIn("ChatGPT Plus".into()).row_status().as_deref(), Some("Authenticated · ChatGPT Plus"));
    assert_eq!(AuthState::SignedIn(String::new()).row_status().as_deref(), Some("Authenticated"));
    assert_eq!(AuthState::SignedOut.row_status().as_deref(), Some("Not signed in"));
    assert_eq!(AuthState::Unknown.row_status(), None);
    assert_eq!(AuthState::NotRequired.row_status(), None);
}
