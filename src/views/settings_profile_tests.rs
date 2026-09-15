//! Headless tests for the Profile settings section: the account rows mirror
//! the workspace's auth state, the signed-out state links to Providers,
//! "Reveal in Finder" opens the data dir's `file://` URL (the test platform
//! records `open_url` calls; `reveal_path` is unimplemented there), and
//! "Sign out all" signs out every flow-based provider.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use super::settings_profile::data_dir;
use crate::auth::{AuthState, logout_flow};
use crate::providers::ProviderKind;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats/auth reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-profile-test-{}", std::process::id()));
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

/// Open settings and switch to the Profile section.
fn open_profile(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        window.click("settings-nav-profile", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-profile").visible(), "profile section should show");
    });
}

/// The default codex instance's id — the seeded providers carry one per kind.
fn codex_id(ws: &Workspace) -> String {
    ws.providers.iter().find(|p| p.kind == ProviderKind::CodexCli).unwrap().id.clone()
}

fn claude_id(ws: &Workspace) -> String {
    ws.providers.iter().find(|p| p.kind == ProviderKind::ClaudeCli).unwrap().id.clone()
}

#[test]
fn signed_out_state_links_to_providers() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    // Nothing signed in — the section shows the empty state, not rows.
    open_profile(cx);
    cx.update(|window, cx| {
        let el = window.find("profile-signed-out");
        assert!(el.visible(), "signed-out state should render");
        assert_eq!(el.label(), Some("No accounts signed in"));
        assert!(window.find("profile-version").visible(), "version still renders signed out");
        window.click("profile-open-providers", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-providers").visible(), "the link lands on Providers");
    });
}

#[test]
fn signed_in_accounts_render_with_status() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (codex, claude) = cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let codex = codex_id(w);
            let claude = claude_id(w);
            w.land_auth(&codex, AuthState::SignedIn("dev@example.com · ChatGPT Plus".into()), cx);
            w.land_auth(&claude, AuthState::SignedIn("claude@example.com".into()), cx);
            (codex, claude)
        })
    });
    open_profile(cx);
    cx.update(|window, _| {
        assert!(window.try_find("profile-signed-out").is_none(), "rows replace the empty state");
        let codex_row = window.find(format!("profile-account-{codex}"));
        assert_eq!(codex_row.label(), Some("Codex — Authenticated · dev@example.com · ChatGPT Plus"));
        let claude_row = window.find(format!("profile-account-{claude}"));
        assert_eq!(claude_row.label(), Some("Claude — Authenticated · claude@example.com"));
        assert!(window.find("profile-sign-out-all").visible(), "sign-out-all shows when a session exists");
        // About block: version + data dir.
        assert_eq!(window.find("profile-version").label(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(window.find("profile-data-dir").label(), Some(data_dir().display().to_string().as_str()));
    });
}

#[test]
fn reveal_in_finder_opens_the_data_dir() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let id = codex_id(w);
            w.land_auth(&id, AuthState::SignedIn("dev@example.com".into()), cx);
        });
    });
    open_profile(cx);
    cx.update(|window, cx| {
        window.click("profile-reveal-data", cx);
    });
    let url = cx.opened_url().expect("reveal should open the data dir URL");
    assert_eq!(url, format!("file://{}", data_dir().display()), "reveal opens the data dir");
}

#[test]
fn sign_out_all_signs_out_every_flow_provider() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (codex, claude) = cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let codex = codex_id(w);
            let claude = claude_id(w);
            w.land_auth(&codex, AuthState::SignedIn("dev@example.com".into()), cx);
            w.land_auth(&claude, AuthState::SignedIn("claude@example.com".into()), cx);
            (codex, claude)
        })
    });
    open_profile(cx);
    cx.update(|window, cx| {
        window.click("profile-sign-out-all", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("profile-signed-out").visible(), "empty state returns once all sessions end");
    });
    cx.update(|_, cx| {
        ws.update(cx, |w, _| {
            assert_eq!(w.auth_state(&codex), AuthState::SignedOut, "codex signed out");
            assert_eq!(w.auth_state(&claude), AuthState::SignedOut, "claude signed out");
        });
    });
}

#[test]
fn sign_out_all_covers_every_logout_flow() {
    // `sign_out_all` iterates `logout_flow` — assert the seeded providers
    // that have one are exactly the flow-based kinds, so the loop can't
    // silently skip a kind that gains a logout later.
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |w, _| {
            let with_logout: Vec<ProviderKind> = w.providers.iter().filter(|p| logout_flow(p.kind).is_some()).map(|p| p.kind).collect();
            assert_eq!(with_logout, vec![ProviderKind::CodexCli, ProviderKind::ClaudeCli]);
        });
    });
}
