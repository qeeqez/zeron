//! Tests for the chat info popover — clicking the titlebar title (or the
//! ⋯ menu's "Chat info") opens a read-only details card under it: created
//! date, message count, provider/model/access, folded token totals + cost
//! estimate, worktree path, ephemeral flag and resumed thread id. Optional
//! rows (worktree, thread id) hide when the chat has none; the thread id
//! copies on click, the worktree reveals in Finder, Esc closes.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::open_in::{ISSUED, reveal_command};
use crate::usage::UsageReport;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-chat-info-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process.
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

/// Push a text message without starting a turn, growing the scroller.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, text: &str) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        let count = this.chats[this.active].messages.len();
        this.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    });
}

/// Click the popup-menu item with `label` — panics when it isn't offered.
fn click_menu_item(window: &mut gpui_kit::Window, label: &str, cx: &mut gpui_kit::App) {
    let item = snapshots(window)
        .iter()
        .find(|s| s.label() == Some(label))
        .unwrap_or_else(|| panic!("menu should offer {label}"))
        .clone();
    let id = item.path().last().unwrap().clone();
    window.within("popup-menu").click(id, cx);
}

/// Click the titlebar title and redraw until the deferred popover surface
/// exists — the first frame after opening only captures the anchor bounds.
fn open_chat_info(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-title", cx);
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        assert!(window.find("chat-info").visible(), "clicking the title should open the details popover");
    });
}

/// A row's aria label — "Label: value" with the untruncated value.
fn row_label(window: &gpui_kit::Window, id: &'static str) -> String {
    window.find(id).label().unwrap_or_else(|| panic!("{id} should carry an aria label")).to_string()
}

/// Wait for `n` recorded open commands — `run_open_command` hops through
/// the background executor before `run` lands in `ISSUED`.
fn until_issued(app: &mut TestAppContext, n: usize) {
    for _ in 0..200 {
        app.run_until_parked();
        if ISSUED.lock().len() >= n {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("expected {n} issued commands, got {:?}", ISSUED.lock());
}

#[test]
fn chat_info_popover_lists_chat_details() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let created = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let workdir = std::env::temp_dir().join("rixlcode-info-worktree");
    let thread_id = "019a4f2e-7c3b-7d1e-9f0a-1234567890ab";
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            let chat = &mut this.chats[this.active];
            chat.title = "Fixture chat".into();
            chat.created_at = created;
            chat.provider = "codex-cli".into();
            chat.model = "gpt-5-codex".into();
            chat.access = Some(crate::backend::AccessMode::FullAccess);
            chat.worktree = true;
            chat.workdir = workdir.to_string_lossy().into_owned();
            chat.thread_id = thread_id.into();
            chat.usage.record(UsageReport {
                input: 1234,
                output: 567,
                cached: 89,
                ..UsageReport::default()
            });
        });
    });
    push(&ws, cx, Role::User, "first question");
    push(&ws, cx, Role::Assistant, "first answer");

    open_chat_info(cx);

    let expected_created = chrono::DateTime::<chrono::Local>::from(created).format("%b %-d, %Y, %-I:%M %p").to_string();
    let expected_cost =
        ws.read_with(cx, |this, _| format!("~{}", crate::pricing::fmt_cost(this.chats[this.active].usage.cost("gpt-5-codex").unwrap())));
    cx.update(|window, _cx| {
        assert_eq!(window.find("chat-info-title").label(), Some("Fixture chat"), "the popover is titled with the chat title");
        assert_eq!(row_label(window, "chat-info-created"), format!("Created: {expected_created}"));
        assert_eq!(row_label(window, "chat-info-messages"), "Messages: 2");
        assert_eq!(row_label(window, "chat-info-model"), "Model: gpt-5-codex");
        assert_eq!(row_label(window, "chat-info-provider"), "Provider: Codex");
        assert_eq!(row_label(window, "chat-info-access"), "Access mode: Full access");
        assert_eq!(row_label(window, "chat-info-tokens"), "Tokens: 1.2k in · 567 out · 89 cached");
        assert_eq!(row_label(window, "chat-info-cost"), format!("Est. cost: {expected_cost}"));
        assert_eq!(row_label(window, "chat-info-worktree"), format!("Worktree: {}", workdir.display()));
        assert_eq!(row_label(window, "chat-info-temporary"), "Temporary: No");
        // The aria label carries the full id even though the row ellipsizes.
        assert_eq!(row_label(window, "chat-info-thread"), format!("Thread id: {thread_id}"));
    });
}

#[test]
fn chat_info_popover_marks_temporary_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.new_temp_chat(cx));
    });

    open_chat_info(cx);

    cx.update(|window, _cx| {
        assert_eq!(window.find("chat-info-title").label(), Some("Temporary chat"));
        assert_eq!(row_label(window, "chat-info-temporary"), "Temporary: Yes");
    });
}

#[test]
fn chat_info_popover_hides_absent_rows() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);

    open_chat_info(cx);

    cx.update(|window, _cx| {
        // A plain checkout chat with no resumed thread: both optional rows
        // stay out of the list entirely.
        assert!(window.try_find("chat-info-worktree").is_none(), "no worktree → no Worktree row");
        assert!(window.try_find("chat-info-thread").is_none(), "no thread id → no Thread id row");
        assert_eq!(row_label(window, "chat-info-temporary"), "Temporary: No");
    });
}

#[test]
fn chat_info_thread_id_copies_to_clipboard() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let thread_id = "019a4f2e-7c3b-7d1e-9f0a-1234567890ab";
    cx.update(|_, cx| {
        ws.update(cx, |this, _| this.chats[this.active].thread_id = thread_id.into());
    });

    open_chat_info(cx);

    cx.update(|window, cx| {
        window.click("chat-info-thread", cx);
    });
    let clipboard = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default());
    assert_eq!(clipboard, thread_id, "clicking the thread id row copies the full id");
}

#[test]
fn chat_info_worktree_reveals_in_finder() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = std::env::temp_dir().join("rixlcode-info-worktree");
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            let chat = &mut this.chats[this.active];
            chat.worktree = true;
            chat.workdir = workdir.to_string_lossy().into_owned();
        });
    });

    open_chat_info(cx);

    cx.update(|window, cx| {
        window.click("chat-info-worktree", cx);
    });
    until_issued(&mut app, 1);
    assert_eq!(ISSUED.lock().as_slice(), &[reveal_command(&workdir)], "clicking the worktree row reveals the checkout");
}

#[test]
fn chat_info_escape_closes_popover() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);

    open_chat_info(cx);

    cx.update(|window, cx| {
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("chat-info").is_none(), "Esc should close the popover");
    });
    assert!(!ws.read_with(cx, |this, _| this.chat_info_open), "Esc writes the closed state back to the flag");
}

#[test]
fn chat_info_menu_item_opens_popover() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        click_menu_item(window, "Chat info", cx);
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        assert!(window.find("chat-info").visible(), "the ⋯ menu's Chat info should open the same popover");
    });
}
