//! Assistant message bodies render as Markdown through `TextView` — headings,
//! emphasis, inline code, lists, tables, links and fenced code blocks. The
//! state is keyed per message so streaming deltas append incrementally
//! (`push_str`) instead of re-parsing the whole reply, and an in-progress
//! text selection survives the stream.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::text::{CodeBlock, TextViewState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Per-message Markdown document state: the `TextViewState` plus the source
/// already rendered into it, so `sync` can tell a streaming append from a
/// replacement (edit, retry, chat switch).
struct MarkdownState {
    view: Entity<TextViewState>,
    rendered: String,
}

impl MarkdownState {
    fn new(text: &str, cx: &mut Context<Self>) -> Self {
        Self {
            view: cx.new(|cx| TextViewState::markdown(text, cx)),
            rendered: text.to_string(),
        }
    }

    fn sync(&mut self, text: &str, cx: &mut Context<Self>) {
        if text == self.rendered {
            return;
        }
        // Streaming appends to the last assistant message — parse just the
        // delta and keep any in-progress selection. Anything else replaces.
        let delta = text.strip_prefix(self.rendered.as_str()).filter(|d| !d.is_empty());
        self.view.update(cx, |view, cx| match delta {
            Some(delta) => view.push_str(delta, cx),
            None => view.set_text(text, cx),
        });
        self.rendered.clear();
        self.rendered.push_str(text);
    }
}

/// Render assistant Markdown: rich blocks plus a code-block affordance row
/// (language label + copy button) like Codex's.
pub(super) fn assistant_markdown(ix: usize, text: &SharedString, window: &mut Window, cx: &mut App) -> AnyElement {
    let state = window.use_keyed_state(("md-state", ix), cx, |_, cx| MarkdownState::new(text, cx));
    state.update(cx, |state, cx| state.sync(text, cx));
    TextView::new(&state.read(cx).view)
        .code_block_actions(move |block, window, cx| code_block_actions(ix, block, window, cx))
        .into_any_element()
}

/// Top-right affordance for a fenced code block: the language tag and a copy
/// button that flips to a check for a moment after copying.
fn code_block_actions(ix: usize, block: &CodeBlock, window: &mut Window, cx: &mut App) -> AnyElement {
    // Span start is unique per block in a message; unspanned blocks share 0 —
    // a cosmetic collision on the copied flag only.
    let key = block.span.as_ref().map(|s| s.start).unwrap_or(0);
    let copied = window.use_keyed_state(("code-copied", key), cx, |_, _| false);
    let is_copied = *copied.read(cx);
    let code = block.code().to_string();
    let lang = block.lang();
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .when_some(lang, |d, lang| d.child(div().id(ElementId::Name(format!("code-lang-{ix}-{lang}").into())).test_support().child(lang)))
        .child(
            div()
                .id(ElementId::Name(format!("copy-code-{ix}-{key}").into()))
                .test_support()
                .cursor_pointer()
                .child(if is_copied { IconName::Check } else { IconName::Copy })
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                    copied.update(cx, |c, cx| {
                        *c = true;
                        cx.notify();
                    });
                    let weak = copied.downgrade();
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(Duration::from_secs(2)).await;
                        let _ = weak.update(cx, |c, cx| {
                            *c = false;
                            cx.notify();
                        });
                    })
                    .detach();
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui_kit::base::TextSelection;
    use gpui_kit::base::test_support::snapshots;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, ElementId, Entity, TestAppContext, VisualTestContext, point, px};

    use super::MarkdownState;
    use crate::workspace::Workspace;

    /// Mount a `Workspace` in a headless window with `HOME` redirected to a
    /// temp dir so settings/chats stay off the real profile.
    fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        let dir = std::env::temp_dir().join(format!("rixlcode-md-test-{}", std::process::id()));
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
            std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(crate::model::ChatMessage {
                role: crate::model::Role::User,
                kind: crate::model::MessageKind::Text("**not bold** ```sh\nx\n```".into()),
                rating: None,
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
}
