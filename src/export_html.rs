//! Chat export as a self-contained, printable HTML document — the ⋯ menu's
//! "Export HTML…" item. Split from `export.rs` for the SLOC cap; reached as
//! `crate::export::html`. The document inlines its CSS and carries a
//! `@media print` block so ⌘P in a browser produces a clean PDF.

use gpui_kit::*;

use crate::export::export_stem;
use crate::model::{ChatMessage, Role};
use crate::workspace::Workspace;

/// Inline stylesheet — self-contained on purpose: the file must render
/// identically detached from the app and survive "Save as PDF".
const EXPORT_CSS: &str = "\
:root { color-scheme: light dark; }
body { font-family: -apple-system, 'Helvetica Neue', sans-serif; max-width: 46rem; margin: 2rem auto; padding: 0 1rem; line-height: 1.5; }
h1 { font-size: 1.4rem; }
.msg { margin: 1.25rem 0; }
.role { font-size: .75rem; font-weight: 600; text-transform: uppercase; letter-spacing: .05em; opacity: .6; margin: 0 0 .25rem; }
.body { white-space: pre-wrap; overflow-wrap: break-word; }
code { font-family: ui-monospace, Menlo, monospace; font-size: .9em; background: rgba(127,127,127,.15); border-radius: 4px; padding: .1em .3em; }
pre { background: rgba(127,127,127,.12); border: 1px solid rgba(127,127,127,.25); border-radius: 8px; padding: .75rem 1rem; overflow-x: auto; }
pre code { background: none; padding: 0; font-size: inherit; }
@media print {
  @page { margin: 2cm; }
  body { max-width: none; margin: 0; color: #000; background: #fff; }
  .msg { break-inside: avoid; }
  pre { white-space: pre-wrap; border-color: #ccc; background: #f5f5f5; }
}";

impl Workspace {
    /// Export chat `ix` as a printable HTML document via the native save
    /// dialog, then reveal the file in Finder. Temporary chats can't be
    /// exported — nothing about them persists.
    pub fn export_chat_html(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.ensure_messages(ix);
        let Some(chat) = self.chats.get(ix) else { return };
        if chat.ephemeral {
            self.push_note("Temporary chats can't be exported.".into(), cx);
            return;
        }
        let out = chat_html(&chat.title, &chat.messages);
        let name = format!("{}.html", export_stem(&chat.title));
        let home = std::env::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&home, Some(&name));
        let ws = cx.entity();
        cx.spawn(async move |_this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            ws.update(cx, |this, cx| this.write_html_export(&path, out, cx));
        })
        .detach();
    }

    /// Export the active chat as HTML — the ⋯ menu and command palette's
    /// entry point.
    pub fn export_active_html(&mut self, cx: &mut Context<Self>) {
        let ix = self.active;
        self.export_chat_html(ix, cx);
    }

    /// Write the rendered document and reveal it in Finder — the user just
    /// picked a path, so showing the file confirms where it landed. Split
    /// from `export_chat_html` so tests can drive it without a save dialog.
    fn write_html_export(&mut self, path: &std::path::Path, html: String, cx: &mut Context<Self>) {
        match std::fs::write(path, html) {
            Ok(()) => {
                self.push_note(format!("Exported to `{}`", path.display()), cx);
                self.reveal_path_in_finder(path, cx);
            },
            Err(_) => self.push_note(format!("Export failed — could not write `{}`", path.display()), cx),
        }
    }
}

/// The whole document: `<h1>` title, then one `.msg` block per message with
/// a `.role` label and the message body. Bodies reuse the same Markdown
/// shape the markdown export writes — `body_html` renders its fenced code
/// blocks and inline code/bold, escaping everything else.
pub(crate) fn chat_html(title: &str, messages: &[ChatMessage]) -> String {
    let mut out = format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>\n{EXPORT_CSS}\n</style>\n</head>\n<body>\n<h1>{}</h1>\n",
        escape_html(title),
        escape_html(title)
    );
    for msg in messages {
        let (label, class) = match msg.role {
            Role::User => ("User", "user"),
            Role::Assistant => ("Assistant", "assistant"),
        };
        out.push_str(&format!(
            "<div class=\"msg {class}\">\n<p class=\"role\">{label}</p>\n<div class=\"body\">{}</div>\n</div>\n",
            body_html(&msg.markdown())
        ));
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// Markdown-ish body → HTML: fenced code blocks become `<pre><code>`,
/// everything else is escaped text with inline `code` and `**bold**`
/// tagged. Newlines stay literal — `.body` renders `white-space: pre-wrap`.
fn body_html(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in markdown.lines() {
        match (in_code, line.strip_prefix("```")) {
            (true, Some(_)) => {
                close_code(&mut out);
                in_code = false;
            },
            (false, Some(lang)) => {
                open_code(&mut out, lang.trim());
                in_code = true;
            },
            (true, None) => {
                out.push_str(&escape_html(line));
                out.push('\n');
            },
            (false, None) => {
                out.push_str(&inline_html(line));
                out.push('\n');
            },
        }
    }
    if in_code {
        out.push_str("</code></pre>");
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// `<pre><code>` opener — a bare fence gets no class, a language tag gets
/// `class="language-…"` (escaped: the value lands inside an attribute).
fn open_code(out: &mut String, lang: &str) {
    if lang.is_empty() {
        out.push_str("<pre><code>");
    } else {
        out.push_str(&format!("<pre><code class=\"language-{}\">", escape_html(lang)));
    }
}

/// `</code></pre>` closer — drops the trailing newline so the block doesn't
/// render a phantom blank line.
fn close_code(out: &mut String) {
    if out.ends_with('\n') {
        out.pop();
    }
    out.push_str("</code></pre>\n");
}

/// One text line: escape, then tag `code spans` and `**bold**`. A delimiter
/// without a closer stays literal.
fn inline_html(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        out.push_str(&bold_html(&rest[..start]));
        let after = &rest[start + 1..];
        match after.find('`') {
            Some(end) => {
                out.push_str("<code>");
                out.push_str(&escape_html(&after[..end]));
                out.push_str("</code>");
                rest = &after[end + 1..];
            },
            None => {
                out.push('`');
                rest = after;
            },
        }
    }
    out.push_str(&bold_html(rest));
    out
}

/// Escape, then tag `**bold**` pairs — same unmatched-delimiter rule as
/// `inline_html`.
fn bold_html(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("**") {
        out.push_str(&escape_html(&rest[..start]));
        let after = &rest[start + 2..];
        match after.find("**") {
            Some(end) => {
                out.push_str("<strong>");
                out.push_str(&escape_html(&after[..end]));
                out.push_str("</strong>");
                rest = &after[end + 2..];
            },
            None => {
                out.push_str("**");
                rest = after;
            },
        }
    }
    out.push_str(&escape_html(rest));
    out
}

/// `& < > " '` — the quote escapes keep `escape_html` output safe inside
/// the `class="language-…"` attribute too.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

// Declared here, not in `main.rs` — that file is at the SLOC cap.
#[cfg(test)]
#[path = "export_html_tests.rs"]
mod export_html_tests;
