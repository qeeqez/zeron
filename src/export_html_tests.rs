//! Tests for `export::html` — the ⋯ menu's "Export HTML…". Pure tests cover
//! the document render (title, roles, escaping, code blocks, print CSS);
//! the headless test drives `export_chat_html` through the simulated save
//! dialog and asserts the file lands on the picked path and gets revealed.
//! Narrow imports on purpose (see `composer_testutil`).

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::export::html::chat_html;
use crate::model::{ChatMessage, MessageKind, Role, ToolCall, ToolStatus};
use crate::open_in::{ISSUED, reveal_command};
use crate::workspace::Workspace;

fn msg(role: Role, kind: MessageKind) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role,
        kind,
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

fn text(role: Role, s: &str) -> ChatMessage {
    msg(role, MessageKind::Text(s.into()))
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-exporthtml-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
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

/// Wait for a background `run_open_command` task to record `n` commands.
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
fn html_contains_title_and_every_message() {
    let messages = vec![
        text(Role::User, "first question"),
        text(Role::Assistant, "an answer"),
        text(Role::User, "follow-up"),
    ];
    let html = chat_html("My Chat", &messages);
    assert!(html.contains("<h1>My Chat</h1>"), "title renders as the document heading");
    for body in ["first question", "an answer", "follow-up"] {
        assert!(html.contains(body), "every message's text is in the document: {body}");
    }
    assert!(html.contains(">User<"), "user messages are role-labeled");
    assert!(html.contains(">Assistant<"), "assistant messages are role-labeled");
}

#[test]
fn html_escapes_and_wraps_code_blocks() {
    let messages = vec![text(Role::Assistant, "use `Vec<T>` here\n```rust\nif a < b && c > d { &x }\n```")];
    let html = chat_html("T", &messages);
    assert!(html.contains("<pre><code class=\"language-rust\">"), "fenced block becomes pre/code");
    assert!(html.contains("if a &lt; b &amp;&amp; c &gt; d { &amp;x }"), "code contents are escaped");
    assert!(!html.contains("if a < b"), "raw markup never reaches the document");
    assert!(html.contains("<code>Vec&lt;T&gt;</code>"), "inline code is tagged and escaped");
}

#[test]
fn html_is_self_contained_and_printable() {
    let html = chat_html("T", &[text(Role::User, "hi")]);
    assert!(html.contains("<style>"), "CSS is inlined — no external assets");
    assert!(html.contains("@media print"), "print rules ship with the document");
    assert!(!html.contains("href="), "nothing external is referenced");
}

#[test]
fn export_writes_the_picked_path_and_reveals_it() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "write");
    let dir = std::env::temp_dir().join(format!("rixlcode-exporthtml-out-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let picked = dir.join("transcript.html");
    ISSUED.lock().clear();
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            std::rc::Rc::make_mut(&mut this.chats[0].messages).push(text(Role::User, "export me <please>"));
            this.export_chat_html(0, cx);
        });
    });
    // `cx`'s borrow of `app` ends here — the prompt helpers live on `app`.
    assert!(app.did_prompt_for_new_path(), "export asks where to save");
    app.simulate_new_path_selection(|_| Some(picked.clone()));
    app.run_until_parked();
    let written = std::fs::read_to_string(&picked).expect("the picked path gets the document");
    assert!(written.contains("export me &lt;please&gt;"), "the file carries the transcript, escaped");
    until_issued(&mut app, 1);
    assert_eq!(ISSUED.lock().as_slice(), &[reveal_command(&picked)], "the written file is revealed in Finder");
}

#[test]
fn export_html_refuses_a_temporary_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "temp");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_temp_chat(cx);
            let ix = this.active;
            this.export_chat_html(ix, cx);
            let chat = &this.chats[ix];
            assert!(
                chat.messages
                    .iter()
                    .any(|m| matches!(&m.kind, MessageKind::Text(t) if t.contains("can't be exported"))),
                "export leaves a refusal note"
            );
        });
    });
    assert!(!app.did_prompt_for_new_path(), "no save dialog for a temporary chat");
}

#[test]
fn tool_and_diff_cards_render_as_code() {
    let messages = vec![msg(
        Role::Assistant,
        MessageKind::Tool(ToolCall {
            tool_ix: 0,
            name: "bash".into(),
            detail: "ls".into(),
            output: "a.rs\nb.rs".into(),
            status: ToolStatus::Done,
            expanded: false,
        }),
    )];
    let html = chat_html("T", &messages);
    assert!(html.contains("<code>bash ls</code>"), "the tool header renders inline");
    assert!(html.contains("a.rs\nb.rs"), "tool output keeps its line breaks");
}
