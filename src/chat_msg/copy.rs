//! Copy and quote operations on a chat message. "Copy" writes the rendered
//! text (what a select-all copy of the bubble produces), "Copy as Markdown"
//! writes the raw source, "Copy Code" writes every fenced block's contents,
//! and "Quote" seeds the composer with a `>` reply block.
//!
//! `markdown_to_plain` mirrors gpui-base's `BlockNode::text()` — the same
//! `markdown` crate and GFM options the renderer parses with, so the copied
//! text matches what the bubble displays (list markers dropped, soft breaks
//! collapsed, code fences unwrapped).

use gpui_kit::*;
use markdown::mdast::Node;

use crate::model::MessageKind;
use crate::workspace::Workspace;

impl Workspace {
    /// Copy the message's rendered text — no markup.
    pub fn copy_message(&self, ix: usize, cx: &mut Context<Self>) {
        let Some(msg) = self.chats[self.active].messages.get(ix) else { return };
        let text = match &msg.kind {
            MessageKind::Text(t) => markdown_to_plain(t),
            MessageKind::Tool(t) => format!("{}: {}\n{}", t.name, t.detail, t.output),
            MessageKind::Diff(d) => format!("{} (+{} -{})\n{}", d.path, d.added, d.removed, d.hunks),
            MessageKind::Plan(p) => p.markdown(),
            MessageKind::Approval(a) => {
                let outcome = a.decision.map_or("pending", |d| d.label());
                format!("{}: {} ({})", a.kind.label(), a.detail, outcome)
            },
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Copy the message's Markdown source verbatim.
    pub fn copy_message_markdown(&self, ix: usize, cx: &mut Context<Self>) {
        let Some(msg) = self.chats[self.active].messages.get(ix) else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(msg.markdown()));
    }

    /// Copy every fenced code block in the message, joined by a blank line —
    /// the message-level counterpart of each block's own copy button. Tool
    /// cards have no fences; their "code" is the output.
    pub fn copy_message_code(&self, ix: usize, cx: &mut Context<Self>) {
        let Some(msg) = self.chats[self.active].messages.get(ix) else { return };
        let code = match &msg.kind {
            MessageKind::Text(t) => code_blocks(t).join("\n\n"),
            MessageKind::Tool(t) => t.output.to_string(),
            _ => String::new(),
        };
        if !code.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(code));
        }
    }

    /// Seed the composer with the message as a `>` quote block, leaving room
    /// for the reply below it. An existing draft is kept after the quote.
    pub fn quote_message(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.chats[self.active].messages.get(ix) else { return };
        let quote = quote_block(&msg.markdown());
        if quote.is_empty() {
            return;
        }
        let draft = self.composer.read(cx).value().to_string();
        let value = if draft.is_empty() { format!("{quote}\n") } else { format!("{quote}\n\n{draft}") };
        self.composer.update(cx, |s, cx| {
            s.set_value(value, window, cx);
            s.focus(window, cx);
        });
    }
}

/// The message source as a `>` quote block — one `> ` prefix per line, bare
/// `>` on blank lines, no trailing newline.
fn quote_block(source: &str) -> String {
    source
        .trim_end()
        .lines()
        .map(|line| if line.is_empty() { ">".to_string() } else { format!("> {line}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The contents of every fenced code block in `source`, in order. Math,
/// frontmatter and MDX flow expressions render as code blocks too, so they
/// count. Blocks nested in lists or quotes are found through `children()`.
pub(crate) fn code_blocks(source: &str) -> Vec<String> {
    let Ok(root) = markdown::to_mdast(source, &markdown::ParseOptions::gfm()) else {
        return vec![];
    };
    let mut blocks = Vec::new();
    collect_code(&root, &mut blocks);
    blocks
}

fn collect_code(node: &Node, out: &mut Vec<String>) {
    match node {
        Node::Code(c) => out.push(c.value.trim_end_matches('\n').to_string()),
        Node::Math(m) => out.push(m.value.trim_end_matches('\n').to_string()),
        Node::Yaml(y) => out.push(y.value.trim_end_matches('\n').to_string()),
        Node::Toml(t) => out.push(t.value.trim_end_matches('\n').to_string()),
        Node::MdxFlowExpression(e) => out.push(e.value.trim_end_matches('\n').to_string()),
        _ => {
            if let Some(children) = node.children() {
                for child in children {
                    collect_code(child, out);
                }
            }
        },
    }
}

/// The rendered text of a Markdown document: each block's text on its own
/// line, markup stripped. Falls back to the source when it fails to parse —
/// copying raw text beats copying nothing.
pub(crate) fn markdown_to_plain(source: &str) -> String {
    let Ok(Node::Root(root)) = markdown::to_mdast(source, &markdown::ParseOptions::gfm()) else {
        return source.to_string();
    };
    root.children.iter().map(block_text).collect::<String>().trim_end_matches('\n').to_string()
}

/// One block's rendered text — mirrors `BlockNode::text()`: paragraphs and
/// headings emit their inline text plus a newline, list items concatenate
/// without markers, tables join cells with spaces, and thematic breaks,
/// definitions and block-level breaks emit nothing.
fn block_text(node: &Node) -> String {
    match node {
        Node::Paragraph(_) | Node::Heading(_) => line(inline_text(node)),
        Node::Blockquote(_) | Node::List(_) | Node::ListItem(_) => {
            node.children().map(|cs| cs.iter().map(block_text).collect()).unwrap_or_default()
        },
        Node::Code(c) => line(c.value.clone()),
        Node::Math(m) => line(m.value.clone()),
        Node::Yaml(y) => line(y.value.clone()),
        Node::Toml(t) => line(t.value.clone()),
        Node::MdxFlowExpression(e) => line(e.value.clone()),
        Node::Table(t) => table_text(t),
        Node::Html(h) => line(html_text(&h.value)),
        Node::FootnoteDefinition(d) => {
            let body = d.children.iter().map(inline_text).collect::<String>();
            line(format!("[{}]: {body}", d.identifier))
        },
        _ => String::new(),
    }
}

/// `text` plus a newline when non-empty — the per-block separator.
fn line(text: String) -> String {
    if text.is_empty() { String::new() } else { format!("{text}\n") }
}

/// A table's rendered text: cells joined by spaces, one row per line, plus
/// the usual trailing block newline.
fn table_text(table: &markdown::mdast::Table) -> String {
    let mut out = String::new();
    for row in &table.children {
        let Node::TableRow(row) = row else { continue };
        let cells: Vec<String> = row.children.iter().map(inline_text).collect();
        if !cells.is_empty() {
            out.push_str(&cells.join(" "));
            out.push('\n');
        }
    }
    line(out.trim_end_matches('\n').to_string())
}

/// One inline node's text: markup dropped, soft breaks collapsed to spaces
/// (the renderer reflows them), hard breaks kept, images emit nothing.
fn inline_text(node: &Node) -> String {
    match node {
        Node::Text(t) => t.value.replace("\r\n", " ").replace(['\n', '\r'], " "),
        Node::InlineCode(c) => c.value.clone(),
        Node::InlineMath(m) => m.value.clone(),
        Node::MdxTextExpression(e) => e.value.clone(),
        Node::Break(_) => "\n".to_string(),
        Node::Html(h) => html_text(&h.value),
        Node::FootnoteReference(f) => format!("[{}]", f.identifier),
        Node::Image(_) | Node::ImageReference(_) => String::new(),
        _ => node.children().map(|cs| cs.iter().map(inline_text).collect()).unwrap_or_default(),
    }
}

/// The text inside an HTML fragment: tags dropped, `<br>` becomes a newline.
/// The renderer parses real inline HTML; this covers the common cases without
/// dragging in its HTML parser.
fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else {
            out.push_str(rest);
            break;
        };
        if rest[..gt].trim_start_matches('<').trim_end_matches('/').trim().eq_ignore_ascii_case("br") {
            out.push('\n');
        }
        rest = &rest[gt + 1..];
    }
    out.push_str(rest);
    out
}
