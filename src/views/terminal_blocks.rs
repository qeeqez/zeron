//! Command-block rendering for the terminal panel: the active session's
//! transcript is split at each submitted command — a muted header row
//! carries the command text (with its OSC 133 exit mark when the shell
//! reports one), the echoed line it replaces is hidden, and the output
//! below stays ordinary interactive text so find highlights and links
//! keep working. Hovering a header reveals re-run (writes the command
//! back to the PTY), copy-output, and send-to-chat (the output drops
//! into the composer draft as a fenced quote); exited sessions keep
//! their blocks read-only — copy and send stay, re-run is gone.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::terminal::blocks::BlockLayout;
use crate::terminal::links::{TermLink, TermMatch};
use crate::workspace::Workspace;

impl Workspace {
    /// The active session's transcript as block sections: text segments
    /// carry find highlights and Cmd-click links (byte ranges rebased
    /// per segment), headers carry the command and its hover actions.
    pub(crate) fn terminal_contents(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(session) = self.terminal.active_session() else {
            return div().into_any_element();
        };
        let t = session.transcript();
        let contents = t.lines.join("\n");
        // Byte offset of each line's start in `contents` — segment ranges
        // rebase find/link ranges by subtracting their segment's offset.
        let mut offs = Vec::with_capacity(t.lines.len() + 1);
        offs.push(0usize);
        for line in &t.lines {
            offs.push(offs.last().unwrap() + line.len() + 1);
        }
        *offs.last_mut().unwrap() = contents.len();
        let seg = Seg {
            contents: &contents,
            offs: &offs,
            matches: self.term_find_matches(cx),
            current: self.terminal.find.match_ix,
            links: self.term_links(&contents),
        };
        let layout = session.block_layout(&t);
        let exited = session.exited;
        let mut children: Vec<AnyElement> = Vec::new();
        let mut l = 0usize;
        for (ix, bl) in layout.iter().enumerate() {
            let text_end = bl.hide.start.max(l).min(t.lines.len());
            if text_end > l {
                children.push(seg.render(l..text_end, cx));
            }
            children.push(block_header(ix, bl, exited, cx));
            l = bl.hide.end.max(l);
        }
        if l < t.lines.len() {
            children.push(seg.render(l..t.lines.len(), cx));
        }
        div().flex().flex_col().children(children).into_any_element()
    }

    /// A block header's re-run button: submit the command again — the
    /// same path the input line takes, so the re-run opens a new block.
    pub(crate) fn terminal_rerun(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(session) = self.terminal.sessions.get_mut(self.terminal.active) {
            session.rerun(ix);
            self.terminal.scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    /// A block header's copy button: the block's output lines onto the
    /// clipboard — the command itself and the next prompt are excluded.
    pub(crate) fn terminal_copy_block(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(session) = self.terminal.active_session() {
            let text = session.block_output(ix);
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    /// A block header's send-to-chat button: the block's output drops
    /// into the composer as a `$ cmd` line over a fenced block — quoted
    /// context above any in-progress draft, the same shape `quote_text`
    /// gives message quotes. Empty output no-ops, same as copy.
    pub(crate) fn terminal_send_block(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.terminal.active_session() else {
            return;
        };
        let Some(command) = session.blocks.get(ix).map(|b| b.command.clone()) else {
            return;
        };
        let quote = block_quote(&command, &session.block_output(ix));
        if quote.is_empty() {
            return;
        }
        let draft = self.composer.read(cx).value().to_string();
        let value = if draft.is_empty() { quote } else { format!("{quote}\n\n{draft}") };
        self.composer.update(cx, |s, cx| {
            s.set_value(value, window, cx);
            s.focus(window, cx);
        });
        // `set_value` suppresses Change — nudge so the send button's
        // enabled state re-reads the new draft.
        cx.notify();
    }
}

/// The shared context a text segment needs: the whole contents, each
/// line's byte offset, and the find/link ranges to clip and rebase.
struct Seg<'a> {
    contents: &'a str,
    offs: &'a [usize],
    matches: Vec<TermMatch>,
    current: usize,
    links: Vec<TermLink>,
}

impl Seg<'_> {
    /// One text run between block boundaries: the same InteractiveText
    /// the flat renderer used, with find/link ranges clipped to the
    /// segment and rebased to its local string.
    fn render(&self, lines: std::ops::Range<usize>, cx: &mut Context<Workspace>) -> AnyElement {
        let b0 = self.offs[lines.start];
        let b1 = self.offs[lines.end];
        let text = &self.contents[b0..b1];
        let clip = |r: &std::ops::Range<usize>| (r.start >= b0 && r.end <= b1).then(|| r.start - b0..r.end - b0);
        let current = self.matches.get(self.current).map(|m| m.range.clone());
        let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
        for m in &self.matches {
            let Some(r) = clip(&m.range) else { continue };
            let bg = if current.as_ref() == Some(&m.range) {
                cx.theme().selection.alpha(0.6)
            } else {
                cx.theme().selection
            };
            highlights.push((r, HighlightStyle { background_color: Some(bg), ..Default::default() }));
        }
        let seg_links: Vec<TermLink> = self
            .links
            .iter()
            .filter_map(|l| clip(&l.range).map(|r| TermLink { range: r, ..l.clone() }))
            .collect();
        // Matches win the paint — a link under a hit stays clickable but
        // skips the underline so the highlight ranges never overlap.
        for link in &seg_links {
            if highlights.iter().any(|(r, _)| r.start < link.range.end && link.range.start < r.end) {
                continue;
            }
            highlights.push((
                link.range.clone(),
                HighlightStyle {
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(cx.theme().accent),
                        wavy: false,
                    }),
                    ..Default::default()
                },
            ));
        }
        let ws = cx.entity();
        InteractiveText::new(("term-seg", lines.start), StyledText::new(text.to_string()).with_highlights(highlights))
            .on_click(seg_links.iter().map(|l| l.range.clone()).collect(), move |ix, window, cx| {
                if !window.modifiers().platform {
                    return;
                }
                let link = seg_links[ix].clone();
                ws.update(cx, |this, cx| this.term_link_click(&link, window, cx));
            })
            .into_any_element()
    }
}

/// The muted row standing in for a command's echo: the command text, an
/// exit mark when OSC 133 reported one, and hover-revealed re-run /
/// copy-output / send-to-chat buttons. Exited sessions drop the re-run
/// affordance.
fn block_header(ix: usize, bl: &BlockLayout, exited: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let group = SharedString::from(format!("term-block-{ix}"));
    let mut actions = div()
        .invisible()
        .group_hover(group.clone(), |style| style.visible())
        .flex()
        .items_center()
        .gap_2()
        .text_color(cx.theme().muted_foreground);
    if !exited {
        actions = actions.child(
            div()
                .id(("term-rerun", ix))
                .test_support()
                .cursor_pointer()
                .child(IconName::RotateCcw)
                .on_click(cx.listener(move |this, _, _, cx| this.terminal_rerun(ix, cx))),
        );
    }
    actions = actions.child(
        div()
            .id(("term-copy", ix))
            .test_support()
            .cursor_pointer()
            .child(IconName::Copy)
            .on_click(cx.listener(move |this, _, _, cx| this.terminal_copy_block(ix, cx))),
    );
    actions = actions.child(
        div()
            .id(("term-send", ix))
            .test_support()
            .cursor_pointer()
            .child(IconName::MessageSquareShare)
            .on_click(cx.listener(move |this, _, window, cx| this.terminal_send_block(ix, window, cx))),
    );
    div()
        .id(("term-block", ix))
        .test_support()
        .group(group)
        .flex()
        .items_center()
        .gap_2()
        .py_0p5()
        .text_color(cx.theme().muted_foreground)
        .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(bl.command.clone()))
        .when_some(bl.exit, |d, code| {
            d.child(if code == 0 {
                div().text_color(cx.theme().success).child(IconName::CircleCheck).into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(cx.theme().danger)
                    .child(IconName::CircleX)
                    .child(code.to_string())
                    .into_any_element()
            })
        })
        .child(actions)
        .into_any_element()
}

/// The composer snippet a block's send button inserts: a `$ cmd`
/// provenance line (dropped when the command is blank) over the output
/// in a fenced `text` block whose fence outruns any backtick run inside
/// it. Whitespace-only output yields an empty string — callers no-op.
pub(crate) fn block_quote(command: &str, output: &str) -> String {
    let output = output.trim_end();
    if output.is_empty() {
        return String::new();
    }
    let fence = code_fence(output);
    match command.trim() {
        "" => format!("{fence}text\n{output}\n{fence}"),
        cmd => format!("$ {cmd}\n{fence}text\n{output}\n{fence}"),
    }
}

/// A backtick fence `content` can't close: one tick longer than its
/// longest backtick run, three at minimum.
fn code_fence(content: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for c in content.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat(longest.max(2) + 1)
}
