//! Assistant message bodies render as Markdown through `TextView` — headings,
//! emphasis, inline code, lists, tables, links and fenced code blocks. The
//! state is keyed per message so streaming deltas append incrementally
//! (`push_str`) instead of re-parsing the whole reply, and an in-progress
//! text selection survives the stream.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::base::text::{CodeBlock, TextViewState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// Per-message Markdown document state: the `TextViewState` plus the source
/// already rendered into it, so `sync` can tell a streaming append from a
/// replacement (edit, retry, chat switch). `raw` flips the body between the
/// rendered document and the Markdown source.
pub(super) struct MarkdownState {
    pub(super) view: Entity<TextViewState>,
    rendered: String,
    pub(super) raw: bool,
}
impl MarkdownState {
    pub(super) fn new(text: &str, cx: &mut Context<Self>) -> Self {
        Self {
            view: cx.new(|cx| TextViewState::markdown(text, cx)),
            rendered: text.to_string(),
            raw: false,
        }
    }

    pub(super) fn sync(&mut self, text: &str, cx: &mut Context<Self>) {
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
        if delta.is_none() {
            // Replaced content (edit, retry, chat switch) starts rendered —
            // a stale raw flag would show the new reply's source instead.
            self.raw = false;
        }
        self.rendered.clear();
        self.rendered.push_str(text);
    }
}

/// The keyed `MarkdownState` for message `ix` — created on first render and
/// shared by the body (rendering) and the footer (the view-raw toggle).
pub(super) fn markdown_state(ix: usize, text: &str, window: &mut Window, cx: &mut App) -> Entity<MarkdownState> {
    window.use_keyed_state(("md-state", ix), cx, |_, cx| MarkdownState::new(text, cx))
}

/// Render assistant Markdown: rich blocks plus a code-block affordance row
/// (language label + copy button) like Codex's. `raw` swaps the document for
/// the Markdown source in mono type. Links are clickable: http(s) opens in
/// the browser, file links reveal in Finder.
pub(super) fn assistant_markdown(
    ix: usize, text: &SharedString, state: &Entity<MarkdownState>, ws: &Entity<Workspace>, cx: &mut App,
) -> AnyElement {
    state.update(cx, |state, cx| state.sync(text, cx));
    if state.read(cx).raw {
        return raw_markdown(ix, text, cx);
    }
    let compact = ws.read(cx).compact_mode;
    let ws = ws.clone();
    TextView::new(&state.read(cx).view)
        .style(super::message::markdown_style(compact))
        .code_block_actions(move |block, window, cx| code_block_actions(ix, block, window, cx))
        .markdown_block_parser(super::mermaid::parse_block)
        .markdown_block_renderer("mermaid", move |node, window, cx| super::mermaid::render_block(ix, node, window, cx))
        .on_link_click(move |url, event, _, cx| open_link(url, event, &ws, cx))
        .into_any_element()
}

/// Where a clicked link goes: http(s) opens in the default browser; a
/// `file://` or relative path reveals in Finder (project-relative paths
/// resolve against the workspace root). Anything else — `mailto:`,
/// `javascript:`, bare `#anchors` — is inert. Mirrors the default handler's
/// click filter: left/middle mouse, keyboard, or a non-long-press touch.
pub(super) fn open_link(url: &SharedString, event: &ClickEvent, ws: &Entity<Workspace>, cx: &mut App) {
    let activate = match event {
        ClickEvent::Mouse(click) => matches!(click.up.button, MouseButton::Left | MouseButton::Middle),
        ClickEvent::Keyboard(_) => true,
        ClickEvent::Touch(click) => !click.long_press,
    };
    if !activate {
        return;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        cx.open_url(url);
        return;
    }
    let path = url.strip_prefix("file://").unwrap_or(url);
    // A `:` marks another scheme (mailto:, javascript:) — never a local path.
    if path.is_empty() || path.starts_with('#') || path.contains(':') {
        return;
    }
    ws.update(cx, |this, cx| this.reveal_in_finder(path, cx));
}

/// The unrendered Markdown source — same text the Copy action writes.
fn raw_markdown(ix: usize, text: &SharedString, cx: &App) -> AnyElement {
    div()
        .id(("md-raw", ix))
        .test_support()
        .font_family(cx.theme().mono_font_family.clone())
        .text_color(cx.theme().muted_foreground)
        .child(text.to_string())
        .into_any_element()
}

/// Top-right affordance for a fenced code block: the language tag, a Run
/// button on shell blocks (dispatches `RunShellCommand` — the workspace's
/// `on_action` runs it), an Apply button on non-shell blocks (dispatches
/// `ApplyCodeBlock` — writes the block to a project file), and a copy
/// button that flips to a check for a moment after copying.
fn code_block_actions(ix: usize, block: &CodeBlock, window: &mut Window, cx: &mut App) -> AnyElement {
    // Span start is unique per block in a message; unspanned blocks share 0.
    let key = block.span.as_ref().map(|s| s.start).unwrap_or(0);
    let code = block.code().to_string();
    let lang = block.lang();
    let shell = lang.as_deref().and_then(crate::run_cmd::shell_for);
    let apply_lang = lang.clone().map(|l| l.to_string());
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .when_some(lang, |d, lang| d.child(div().id(ElementId::Name(format!("code-lang-{ix}-{lang}").into())).test_support().child(lang)))
        .when_some(shell, |d, shell| {
            let command = code.clone();
            d.child(
                div()
                    .id(ElementId::Name(format!("run-code-{ix}-{key}").into()))
                    .test_support()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(IconName::Play)
                    .child("Run")
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(crate::run_cmd::RunShellCommand { command: command.clone(), shell }), cx);
                    }),
            )
        })
        .when(shell.is_none(), |d| {
            let code = code.clone();
            let lang = apply_lang.clone();
            d.child(
                div()
                    .id(ElementId::Name(format!("apply-code-{ix}-{key}").into()))
                    .test_support()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(IconName::FilePen)
                    .child("Apply")
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(crate::apply_code::ApplyCodeBlock { code: code.clone(), lang: lang.clone() }), cx);
                    }),
            )
        })
        .child(copy_code_button(ix, key as u64, code, window, cx))
        .into_any_element()
}

/// The copy button every fenced block carries — plain code blocks and
/// mermaid diagrams alike. Flips to a check for two seconds after copying.
/// `key` namespaces the copied flag per block (span start, or a content hash
/// for mermaid).
pub(super) fn copy_code_button(ix: usize, key: u64, code: String, window: &mut Window, cx: &mut App) -> ObservedElement<Stateful<Div>> {
    let copied = window.use_keyed_state(("code-copied", key), cx, |_, _| false);
    let is_copied = *copied.read(cx);
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
        })
}
