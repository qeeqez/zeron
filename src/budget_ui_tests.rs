//! Budget alert UI surfaces: the ⋯ menu's "Budget alert…" dialog (per-chat
//! override) and the General settings field (global default). Same mount
//! harness as `budget_tests.rs` — duplicated because sibling test files
//! can't share private helpers.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-budget-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
}

fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    sandbox_home();
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

/// Price the active chat on gpt-5 and fold `input` tokens into its usage.
fn seed_spend(ws: &Entity<Workspace>, input: u64, cx: &mut VisualTestContext) {
    ws.update(cx, |this, _| {
        this.chats[0].model = "gpt-5".into();
        this.chats[0].usage.record(crate::usage::UsageReport::tokens(input, 0));
    });
}

#[test]
fn budget_dialog_sets_and_clears_the_override() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = ws.read_with(cx, |ws, _| ws.chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        let item = snapshots(window)
            .iter()
            .find(|s| s.role() == Some(A11yRole::MenuItem) && s.label() == Some("Budget alert…"))
            .unwrap_or_else(|| panic!("chat menu should offer Budget alert…"))
            .clone();
        window.within("popup-menu").click(item.path().last().unwrap().clone(), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("dialog").is_some(), "the budget dialog should open");
        ws.read(cx).budget_input.clone().update(cx, |s, cx| s.set_value("2.50", window, cx));
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].budget_alert_usd, Some(2.5), "OK saves the override");
        assert!(window.try_find("dialog").is_none(), "dialog closes on OK");
    });
    // Reopen — the field seeds the override; empty input clears it.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.open_chat_budget(chat_id, window, cx));
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).budget_input.read(cx).value().as_ref(), "2.5", "the dialog seeds the current cap");
        ws.read(cx).budget_input.clone().update(cx, |s, cx| s.set_value("", window, cx));
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    assert_eq!(ws.read_with(cx, |ws, _| ws.chats[0].budget_alert_usd), None, "empty input clears back to the global default");
}

#[test]
fn settings_field_commits_the_global_cap() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_spend(&ws, 5_000_000, cx); // $6.25 spent
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.budget_cap_input.update(cx, |input, cx| input.set_value("3.50", window, cx));
            this.commit_budget_cap(window, cx);
        });
    });
    assert_eq!(ws.read_with(cx, |ws, _| ws.budget_alert_usd), Some(3.5));
    assert_eq!(crate::persist::load_settings().budget_alert_usd, Some(3.5), "the cap persists");
    // Spend already over the new cap alerts immediately — no turn needed.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("budget-banner").visible(), "a lowered cap alerts on commit");
    });
    // An invalid draft keeps the stored cap and snaps the field back.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.budget_cap_input.update(cx, |input, cx| input.set_value("abc", window, cx));
            this.commit_budget_cap(window, cx);
        });
    });
    assert_eq!(ws.read_with(cx, |ws, _| ws.budget_alert_usd), Some(3.5), "invalid input keeps the stored cap");
    assert_eq!(ws.read_with(cx, |ws, cx| ws.budget_cap_input.read(cx).value().to_string()), "3.5", "the field snaps back");
    // Empty clears the cap — and the banner.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.budget_cap_input.update(cx, |input, cx| input.set_value("", window, cx));
            this.commit_budget_cap(window, cx);
        });
        window.draw(cx).clear(cx);
        assert!(window.try_find("budget-banner").is_none(), "clearing the cap drops the banner");
    });
    assert_eq!(ws.read_with(cx, |ws, _| ws.budget_alert_usd), None);
}
