//! Unit tests for the copy/quote operations: Copy writes rendered text,
//! Copy as Markdown writes the raw source, Copy Code collects fenced
//! blocks, and Quote seeds the composer with a `>` reply block.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-copy-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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

/// Push an assistant text message without starting a turn.
fn seed(ws: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role: Role::Assistant,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        cx.notify();
    });
}

fn clipboard(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default())
}

fn composer(ws: &Entity<Workspace>, cx: &VisualTestContext) -> String {
    ws.read_with(cx, |ws, app| ws.composer.read(app).value().to_string())
}

#[test]
fn copy_writes_rendered_text_not_markup() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "# Title\n\n**Bold** and `code`.\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n", cx);
    cx.update(|_, cx| ws.update(cx, |this, cx| this.copy_message(0, cx)));
    assert_eq!(clipboard(cx), "Title\nBold and code.\none\ntwo\nfn main() {}", "copy should strip markup");
}

#[test]
fn copy_as_markdown_writes_raw_source() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let source = "# Title\n\n**Bold** and `code`.\n\n- one\n- two\n";
    seed(&ws, source, cx);
    cx.update(|_, cx| ws.update(cx, |this, cx| this.copy_message_markdown(0, cx)));
    assert_eq!(clipboard(cx), source, "copy-as-markdown should keep the source verbatim");
}

#[test]
fn copy_code_collects_fenced_blocks() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "intro\n\n```rust\nfn a() {}\n```\n\nbetween\n\n```sh\nls -la\n```\n", cx);
    cx.update(|_, cx| ws.update(cx, |this, cx| this.copy_message_code(0, cx)));
    assert_eq!(clipboard(cx), "fn a() {}\n\nls -la", "copy-code should join every fenced block");
}

#[test]
fn quote_inserts_blockquote_into_empty_composer() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "first line\n\nsecond para", cx);
    cx.update(|window, cx| ws.update(cx, |this, cx| this.quote_message(0, window, cx)));
    assert_eq!(composer(&ws, cx), "> first line\n>\n> second para\n", "quote should prefix every line");
}

#[test]
fn quote_keeps_an_existing_draft_below_the_quote() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "quoted words", cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value("my reply", window, cx));
            this.quote_message(0, window, cx);
        });
    });
    assert_eq!(composer(&ws, cx), "> quoted words\n\nmy reply", "draft should survive after the quote");
}
