//! Headless tests for assistant Markdown rendering: structured blocks,
//! streaming appends, partial input mid-stream, code-block copy and the
//! view-raw toggle.

use gpui_kit::base::TextSelection;
use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Entity, TestAppContext, VisualTestContext, point, px};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::views::markdown::MarkdownState;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-md-test-{}", std::process::id()));
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

fn seed(ws: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| this.push_note(text.to_string(), cx));
}

/// Overwrite the last message's text — the path streaming deltas take.
fn grow_last(ws: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        let chat = &mut this.chats[this.active];
        let Some(last) = std::rc::Rc::make_mut(&mut chat.messages).last_mut() else { return };
        let MessageKind::Text(t) = &mut last.kind else { return };
        *t = text.into();
        this.scroller.update(cx, |s, cx| s.remeasure(cx));
        cx.notify();
    });
}

/// ElementIds registered by `.test_support()` in the last frame.
fn observed_ids(window: &gpui_kit::Window) -> Vec<ElementId> {
    snapshots(window).iter().filter_map(|s| s.path().last().cloned()).collect()
}

fn has_id_containing(ids: &[ElementId], needle: &str) -> bool {
    ids.iter().any(|id| format!("{id:?}").contains(needle))
}

#[test]
fn markdown_renders_structured_blocks_not_raw_markup() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "# Plan\n\n**Bold** and *italic* with `code`.\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "code-lang-0-rust"), "lang label missing: {ids:?}");
        assert!(has_id_containing(&ids, "copy-code-0-"), "copy button missing: {ids:?}");

        // Drag-select the whole message body: the copyable text must be
        // the rendered content, not the Markdown source.
        let b = window.find(("md-body", 0usize)).bounds();
        window.drag(point(b.origin.x + px(20.), b.origin.y + px(8.)), point(b.right() - px(4.), b.bottom() - px(2.)), cx);
        let selected = TextSelection::selected_text(window, cx);
        for raw in ["# Plan", "**Bold**", "*italic*", "`code`", "```"] {
            assert!(!selected.contains(raw), "raw markup leaked into selection: {selected:?}");
        }
        for rendered in ["Plan", "Bold", "italic", "code", "one", "two", "fn main()"] {
            assert!(selected.contains(rendered), "missing rendered text {rendered:?} in {selected:?}");
        }
    });
}

#[test]
fn code_block_copy_writes_code_to_clipboard() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "Run this:\n\n```rust\nfn main() {}\n```\n", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let copy_id = observed_ids(window)
            .into_iter()
            .find(|id| format!("{id:?}").contains("copy-code-0-"))
            .expect("copy button missing");
        window.click(copy_id, cx);
        let clip = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        assert!(clip.contains("fn main() {}"), "clipboard: {clip:?}");
        assert!(!clip.contains("```"), "clipboard copied the fence: {clip:?}");
    });
}

#[test]
fn user_message_stays_plain_text() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role: Role::User,
            kind: MessageKind::Text("**not bold** ```sh\nx\n```".into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("msg", 0usize)).visible());
        let ids = observed_ids(window);
        assert!(!has_id_containing(&ids, "copy-code") && !has_id_containing(&ids, "code-lang"), "{ids:?}");
    });
}

#[test]
fn streaming_deltas_append_incrementally() {
    let app = TestAppContext::single();
    let state = app.update(|cx| cx.new(|cx| MarkdownState::new("hello", cx)));
    app.update(|cx| {
        state.update(cx, |s, cx| s.sync("hello **wor", cx));
        state.update(cx, |s, cx| s.sync("hello **world**", cx));
        // A non-append (edit/retry) replaces instead.
        state.update(cx, |s, cx| s.sync("different", cx));
    });
    // The final document holds the replaced text — verified by selecting all.
    app.update(|cx| {
        state.update(cx, |s, cx| {
            s.view.update(cx, |v, cx| v.select_all(cx));
        });
    });
    app.read(|cx| {
        assert_eq!(state.read(cx).view.read(cx).selected_text().trim(), "different");
    });
}

/// Mid-stream the reply is always partial Markdown: an unclosed fence, an
/// open `**`, a dangling `[` link. Every stage must render without panic,
/// and an unclosed fence still gets the code-block header + copy button.
#[test]
fn partial_markdown_streams_without_panic() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let stages = [
        "Working on it…\n\n```rust\nfn main() {",
        "Working on it…\n\n```rust\nfn main() {\n    let x = **unclosed\n",
        "Working on it…\n\n```rust\nfn main() {\n    let x = 1;\n}\n```\n\nDone — **bold",
        "Working on it…\n\n```rust\nfn main() {\n    let x = 1;\n}\n```\n\nDone — **bold** and [a link",
    ];
    seed(&ws, stages[0], cx);
    for stage in &stages[1..] {
        grow_last(&ws, stage, cx);
    }
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "copy-code-0-"), "copy button missing after closed fence: {ids:?}");
        assert!(has_id_containing(&ids, "code-lang-0-rust"), "lang label missing: {ids:?}");
    });
}

/// The unclosed-fence stage alone: the code block is still open, yet the
/// affordance row renders and copies the partial code.
#[test]
fn unclosed_fence_copies_partial_code() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "One moment:\n\n```sh\ncargo build --loc", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let copy_id = observed_ids(window)
            .into_iter()
            .find(|id| format!("{id:?}").contains("copy-code-0-"))
            .expect("copy button missing for unclosed fence");
        window.click(copy_id, cx);
        let clip = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        assert!(clip.contains("cargo build --loc"), "clipboard: {clip:?}");
    });
}

/// The footer's raw toggle swaps the rendered document for the Markdown
/// source and back.
#[test]
fn view_raw_toggle_shows_markdown_source() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "**Bold** reply\n\n```rust\nfn main() {}\n```\n", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("raw", 0usize)).visible(), "raw toggle hidden on hover");
        window.click(("raw", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("md-raw", 0usize)).visible(), "raw source not shown");
        let ids = observed_ids(window);
        assert!(!has_id_containing(&ids, "copy-code-0-"), "rendered code block still present: {ids:?}");
        window.click(("raw", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find(("md-raw", 0usize)).is_none(), "raw source still shown after toggle off");
        assert!(has_id_containing(&observed_ids(window), "copy-code-0-"), "rendered view not restored");
    });
}

/// Clicking a rendered `[text](https://…)` link opens the URL in the browser
/// — the test platform records `cx.open_url` as `opened_url`.
#[test]
fn markdown_link_click_opens_url() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "[the example link](https://example.com/some/page)", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The whole body is the link — a click just inside the text lands on it.
        window.click_at(("md-body", 0usize), point(px(30.), px(15.)), cx);
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/some/page"));
}

/// A bare `https://…` autolink (no `[text](…)` syntax) opens too.
#[test]
fn markdown_autolink_click_opens_url() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "<https://example.com/auto>", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click_at(("md-body", 0usize), point(px(30.), px(15.)), cx);
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/auto"));
}

/// Non-web links never reach `open_url`: `mailto:`/`javascript:` are inert,
/// while a relative path reveals in Finder through the `open_in` fake.
#[test]
fn markdown_link_guard_schemes_and_files() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "[mail](mailto:a@b.c) and [script](javascript:void)", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Click the first link ("mail") — near the text start.
        window.click_at(("md-body", 0usize), point(px(30.), px(15.)), cx);
    });
    assert_eq!(cx.opened_url(), None, "mailto: must not open in the browser");
    assert!(crate::open_in::ISSUED.lock().is_empty(), "mailto: must not reveal a file");

    grow_last(&ws, "[the file](src/main.rs)", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click_at(("md-body", 0usize), point(px(30.), px(15.)), cx);
    });
    assert_eq!(cx.opened_url(), None, "file links don't go to the browser");
    for _ in 0..200 {
        cx.run_until_parked();
        if !crate::open_in::ISSUED.lock().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let abs = ws.read_with(cx, |w, _| w.project.root().join("src/main.rs"));
    assert_eq!(crate::open_in::ISSUED.lock().as_slice(), &[crate::open_in::reveal_command(&abs)], "relative link should reveal in Finder");
}
