//! Headless tests for the first-run onboarding card: it replaces the empty
//! state while no provider is usable, Skip persists the dismissal, the
//! setup button opens the same wizard Settings → Providers launches, and a
//! provider landing hides the card on its own. Same harness as
//! `settings_providers_tests.rs` — declared via `#[path]` in `views::mod`
//! so `main.rs` stays under the SLOC cap.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::providers::ProviderKind;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-onboarding-test-{}", std::process::id()));
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

/// Drop every provider instance — the state a first-run user is in.
fn remove_all_providers(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let ids: Vec<String> = w.provider_instances().iter().map(|p| p.id.clone()).collect();
            for id in ids {
                w.remove_provider(&id, cx);
            }
        });
    });
}

#[test]
fn card_shows_only_without_provider() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Fresh profile seeds the built-in providers — no card.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("onboarding-card").is_none(), "configured providers hide the card");
        assert!(window.find("open-project").visible(), "regular empty state shows");
    });
    // Remove them all — the card takes over the empty state.
    remove_all_providers(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("onboarding-card").visible(), "no usable provider shows the card");
        assert!(window.try_find("open-project").is_none(), "card replaces the regular empty state");
    });
}

#[test]
fn skip_persists_and_hides_card() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    remove_all_providers(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("onboarding-skip", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("onboarding-card").is_none(), "skip hides the card");
        assert!(window.find("open-project").visible(), "regular empty state returns");
        assert!(ws.read(cx).onboarding_dismissed);
    });
    assert!(crate::persist::load_settings().onboarding_dismissed, "skip must persist to settings.json");
}

#[test]
fn setup_button_opens_provider_wizard() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    remove_all_providers(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("onboarding-setup", cx);
        window.draw(cx).clear(cx);
    });
    // The dialog animates on a real-time clock — wait it out like
    // `open_wizard` in settings_providers_tests.
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("wizard-driver").visible(), "setup opens the provider wizard");
        assert!(ws.read(cx).settings_panel.read(cx).provider_wizard.is_some(), "same wizard-open path as Settings → Providers");
    });
}

#[test]
fn provider_landing_hides_card_without_skip() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    remove_all_providers(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("onboarding-card").visible());
    });
    // A provider lands — the card disappears on its own (live condition).
    cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            w.add_provider(ProviderKind::Sim, "Sim".to_string(), cx);
        });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("onboarding-card").is_none(), "a usable provider hides the card");
        assert!(window.find("open-project").visible());
        assert!(!ws.read(cx).onboarding_dismissed, "no skip was needed");
    });
}
