//! Prompt templates — named, reusable composer drafts, persisted per project
//! as `<project>/templates.json` (see
//! `crate::persist::{save_templates, load_templates}`).
//!
//! `/templates` opens the picker dialog (see `crate::views::templates`); a
//! pick loads the template's body into the composer draft without sending.
//! The composer ⋯ menu's "Save as template…" names the current draft; the
//! picker row's hover ✕ deletes.

use gpui_kit::component::WindowExt;
use gpui_kit::component::input::Input;
use gpui_kit::*;

use crate::workspace::Workspace;

/// One prompt template: a name the user picks plus the body a pick loads
/// into the composer.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Template {
    pub name: String,
    pub body: String,
}

/// The project's prompt templates, in save order — the picker's display
/// order.
#[derive(Debug, Default)]
pub struct TemplateStore {
    pub templates: Vec<Template>,
}

impl TemplateStore {
    /// Insert or overwrite the template named `name` (trimmed). Empty name
    /// or body saves nothing — callers surface that as a note.
    pub fn save(&mut self, name: &str, body: &str) -> bool {
        let (name, body) = (name.trim(), body.trim());
        if name.is_empty() || body.is_empty() {
            return false;
        }
        if let Some(t) = self.templates.iter_mut().find(|t| t.name == name) {
            t.body = body.to_string();
        } else {
            self.templates.push(Template { name: name.to_string(), body: body.to_string() });
        }
        true
    }

    pub fn get(&self, name: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.name == name)
    }

    /// Drop the template named `name`; returns whether one existed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.templates.len();
        self.templates.retain(|t| t.name != name);
        self.templates.len() != before
    }
}

impl Workspace {
    /// Store `body` under `name` and persist. Shared by the save dialog and
    /// tests; returns false on an empty name or body.
    pub(crate) fn save_template(&mut self, name: &str, body: &str) -> bool {
        if !self.templates.save(name, body) {
            return false;
        }
        crate::persist::save_templates(self.project.dir(), &self.templates);
        true
    }

    /// Load a template into the composer draft — the picker row's click.
    /// The dialog closes first so focus lands back on the composer; the
    /// body is never sent.
    pub(crate) fn load_template(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(body) = self.templates.get(name).map(|t| t.body.clone()) else { return };
        window.close_dialog(cx);
        self.composer.update(cx, |s, cx| {
            s.set_value(body, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// Delete a template and persist — the picker row's ✕. The picker's
    /// list is an open-time snapshot (the dialog builder can't read the
    /// workspace mid-render), so a delete from inside it reopens the
    /// dialog for a fresh list.
    pub(crate) fn delete_template(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.templates.remove(name) {
            return;
        }
        crate::persist::save_templates(self.project.dir(), &self.templates);
        cx.notify();
        if window.has_active_dialog(cx) {
            window.close_dialog(cx);
            self.open_template_picker(window, cx);
        }
    }

    /// The composer ⋯ menu's "Save as template…" — a small dialog naming
    /// the composer's current text. An empty draft opens nothing.
    pub(crate) fn open_save_template_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.composer.read(cx).value().to_string();
        if body.trim().is_empty() {
            return;
        }
        self.template_input.update(cx, |state, cx| state.set_value("", window, cx));
        let ws = cx.entity();
        let input = self.template_input.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ws_ok = ws.clone();
            let body = body.clone();
            dialog
                .title("Save as template")
                .overlay_closable(true)
                .child(Input::new(&input))
                .on_ok(move |_, _window, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_save_template(&body, cx));
                    true
                })
        });
        self.template_input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Dialog OK for "Save as template" — empty names save nothing.
    fn commit_save_template(&mut self, body: &str, cx: &mut Context<Self>) {
        let name = self.template_input.read(cx).value().trim().to_string();
        if self.save_template(&name, body) {
            self.push_note(format!("Saved template `{name}` — `/templates` can load it."), cx);
        }
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "templates_tests.rs"]
mod templates_tests;
