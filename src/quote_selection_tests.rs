//! "Quote selection" on the message context menu seeds the composer with a
//! `>` block of just the selected text — the per-message counterpart of
//! "Quote". The item only exists while the message body has an active
//! selection, and an existing draft survives below the quote.

use gpui_kit::base::TextSelection;
use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext, point, px};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-quote-sel-test-{}", std::process::id()));
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
            role: Role::Assistant,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

fn composer(ws: &Entity<Workspace>, cx: &VisualTestContext) -> String {
    ws.read_with(cx, |ws, app| ws.composer.read(app).value().to_string())
}

/// The window's current text selection.
fn selected_text(cx: &mut VisualTestContext) -> String {
    cx.update(TextSelection::selected_text)
}

/// Drag across the start of message `ix`'s first text line — from just
/// inside the text to ~100px in — so a prefix of the message is selected.
fn select_message_prefix(ix: usize, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let body = window.find(("md-body", ix)).bounds();
        // The body has px_4/py_2 padding; the text starts one line-height in.
        let from = point(body.origin.x + px(17.), body.origin.y + px(9.));
        window.drag(from, from + point(px(100.), px(0.)), cx);
    });
}

fn menu_labels(window: &gpui_kit::Window) -> Vec<String> {
    let mut labels: Vec<String> = snapshots(window)
        .iter()
        .filter(|s| s.role() == Some(A11yRole::MenuItem))
        .filter_map(|s| s.label().map(str::to_string))
        .collect();
    labels.sort();
    labels
}

/// Click the open menu's item with `label`.
fn click_item(label: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let item = snapshots(window)
            .iter()
            .find(|s| s.label() == Some(label))
            .unwrap_or_else(|| panic!("menu should offer {label}"))
            .clone();
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").click(id, cx);
    });
}

/// Mirrors the `>` block the composer receives: one `> ` prefix per line.
fn expected_quote(text: &str) -> String {
    text.trim_end()
        .lines()
        .map(|line| if line.is_empty() { ">".to_string() } else { format!("> {line}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A drag selection inside the message body adds "Quote selection" to the
/// menu; choosing it seeds the composer with just the selected text.
#[test]
fn quote_selection_seeds_composer_with_selected_text() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "alpha beta gamma delta epsilon zeta eta theta", cx);

    select_message_prefix(0, cx);
    let selected = selected_text(cx);
    assert!(!selected.trim().is_empty(), "drag should select text");
    assert!(
        selected.trim().len() < "alpha beta gamma delta epsilon zeta eta theta".len(),
        "expected a sub-selection, got the whole message: {selected:?}"
    );

    cx.update(|window, cx| {
        window.right_click(("msg", 0usize), cx);
    });
    // The menu entity is built in a deferred callback after this update.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the message menu");
        assert!(menu_labels(window).contains(&"Quote selection".to_string()), "selection should add the item: {:?}", menu_labels(window));
    });
    click_item("Quote selection", cx);

    assert_eq!(composer(&ws, cx), format!("{}\n", expected_quote(&selected)), "composer should hold just the selection as a quote block");
}

/// Without a selection the menu offers "Quote" but not "Quote selection".
#[test]
fn quote_selection_hidden_without_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "nothing selected here", cx);

    cx.update(|window, cx| {
        window.right_click(("msg", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the message menu");
        let labels = menu_labels(window);
        assert!(labels.contains(&"Quote".to_string()), "Quote should always be listed: {labels:?}");
        assert!(!labels.contains(&"Quote selection".to_string()), "no selection means no Quote selection item: {labels:?}");
    });
}

/// An existing composer draft is preserved below the quoted selection.
#[test]
fn quote_selection_keeps_an_existing_draft_below_the_quote() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "alpha beta gamma delta epsilon zeta eta theta", cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value("my reply", window, cx));
        });
    });

    select_message_prefix(0, cx);
    let selected = selected_text(cx);
    assert!(!selected.trim().is_empty(), "drag should select text");

    cx.update(|window, cx| {
        window.right_click(("msg", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    click_item("Quote selection", cx);

    assert_eq!(composer(&ws, cx), format!("{}\n\nmy reply", expected_quote(&selected)), "draft should survive after the quote");
}
