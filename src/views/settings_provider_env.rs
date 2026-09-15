//! Variables-row input state for the provider detail panel: one `EnvRow`
//! (key + value `InputState`) per `ProviderInstance.env` entry, synced
//! lazily on render and writing edits back through
//! `Workspace::set_provider_env`. Split from `settings_provider_detail`
//! to stay under the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::providers::ProviderInstance;
use crate::views::settings::SettingsPanel;
use crate::views::settings_provider_detail::icon_btn;
use crate::views::settings_sections::{SettingsView, group_label};
use crate::workspace::Workspace;

/// One Variables row's inputs — key + value `InputState` entities that
/// mirror `ProviderInstance.env[ix]`. They live on `SettingsPanel` so
/// typed text survives re-renders; `SettingsView` carries clones.
#[derive(Clone)]
pub(crate) struct EnvRow {
    pub key: Entity<InputState>,
    pub value: Entity<InputState>,
}

/// Which input of a Variables row changed — the write-back needs the
/// sibling's text to rebuild the (key, value) pair.
#[derive(Clone, Copy)]
enum EnvField {
    Key,
    Value,
}

/// The row's instance id, which input fired, and the workspace handle —
/// bundled so the subscribe closure stays under the argument-count lint.
#[derive(Clone)]
struct EnvFieldCtx {
    field: EnvField,
    id: String,
    ws: WeakEntity<Workspace>,
}

impl SettingsPanel {
    /// Reconcile `provider_env_inputs` with the live instances: one input
    /// row per `env` entry, created lazily on render and dropped with the
    /// instance. Mirrors `sync_provider_inputs`.
    pub(crate) fn sync_provider_env_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let instances: Vec<ProviderInstance> = self.ws.upgrade().map(|ws| ws.read(cx).provider_instances().to_vec()).unwrap_or_default();
        let ids: std::collections::HashSet<&str> = instances.iter().map(|p| p.id.as_str()).collect();
        self.provider_env_inputs.retain(|id, _| ids.contains(id.as_str()));
        for p in &instances {
            let rows = self.provider_env_inputs.entry(p.id.clone()).or_default();
            rows.truncate(p.env.len());
            let start = rows.len();
            let new_rows: Vec<EnvRow> = (start..p.env.len()).map(|ix| self.new_env_row(&p.id, p.env[ix].clone(), window, cx)).collect();
            self.provider_env_inputs.get_mut(&p.id).expect("entry above").extend(new_rows);
        }
    }

    /// Create one Variables row's inputs, seed them from `pair`, and
    /// subscribe so edits write back through `set_provider_env`. The
    /// subscription finds the row's index at event time — row removals
    /// shift indices, so a captured index would write the wrong row.
    fn new_env_row(&mut self, id: &str, pair: (String, String), window: &mut Window, cx: &mut Context<Self>) -> EnvRow {
        let key = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("KEY");
            s.set_value(pair.0, window, cx);
            s
        });
        let value = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("value");
            s.set_value(pair.1, window, cx);
            s
        });
        for (input, field) in [(key.clone(), EnvField::Key), (value.clone(), EnvField::Value)] {
            let ctx = EnvFieldCtx { field, id: id.to_string(), ws: self.ws.clone() };
            cx.subscribe_in(&input, window, move |this, state, event: &InputEvent, _window, cx| {
                this.on_env_field(&ctx, state, event, cx);
            })
            .detach();
        }
        EnvRow { key, value }
    }

    /// Write a Variables edit onto the instance: locate the row by its
    /// input entity (indices shift on removal), read the sibling input
    /// for the other half of the pair, then `set_provider_env`.
    fn on_env_field(&mut self, ctx: &EnvFieldCtx, state: &Entity<InputState>, event: &InputEvent, cx: &mut Context<Self>) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let Some(rows) = self.provider_env_inputs.get(&ctx.id) else { return };
        let Some(ix) = rows.iter().position(|r| match ctx.field {
            EnvField::Key => &r.key == state,
            EnvField::Value => &r.value == state,
        }) else {
            return;
        };
        let other = match ctx.field {
            EnvField::Key => &rows[ix].value,
            EnvField::Value => &rows[ix].key,
        };
        let other_value = other.read(cx).value().to_string();
        let value = state.read(cx).value().to_string();
        let pair = match ctx.field {
            EnvField::Key => (value, other_value),
            EnvField::Value => (other_value, value),
        };
        let _ = ctx.ws.update(cx, |this, cx| this.set_provider_env(&ctx.id, ix, pair, cx));
    }
}

/// The Variables block: one editable KEY/VALUE row per `p.env` entry plus
/// an "Add variable" button. Rows mirror `p.env` by index — blank keys
/// are half-edited rows, ignored on save and at spawn.
pub(crate) fn variables_section(p: &ProviderInstance, rows: &[EnvRow], s: &SettingsView, cx: &App) -> impl IntoElement {
    let (pid_add, ws_add) = (p.id.clone(), s.ws.clone());
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(group_label("Variables", cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Environment variables for this instance's backend — a custom base URL or API key without touching your shell env."),
        )
        .children(rows.iter().enumerate().map(|(ix, row)| env_row(&p.id, ix, row, s)))
        .child(
            div().pt_1().child(
                Button::new(SharedString::from(format!("provider-env-add-{}", p.id)))
                    .label("Add variable")
                    .icon(IconName::Plus)
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| {
                        ws_add.update(cx, |this, cx| this.add_provider_env_row(&pid_add, cx));
                    }),
            ),
        )
}

/// One Variables row: key input, value input, and a remove ✕. Edits write
/// through the row's `InputEvent::Change` subscription (see
/// `SettingsPanel::new_env_row`); the ✕ drops `p.env[ix]`.
fn env_row(pid: &str, ix: usize, row: &EnvRow, s: &SettingsView) -> impl IntoElement {
    let (pid_rm, ws_rm) = (pid.to_string(), s.ws.clone());
    div()
        .id(SharedString::from(format!("provider-env-row-{pid}-{ix}")))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(180.))
                .child(Input::new(&row.key).id(SharedString::from(format!("provider-env-key-{pid}-{ix}"))).appearance(true)),
        )
        .child(
            div().flex_1().child(
                Input::new(&row.value)
                    .id(SharedString::from(format!("provider-env-value-{pid}-{ix}")))
                    .appearance(true),
            ),
        )
        .child(icon_btn(&format!("provider-env-rm-{pid}-{ix}"), IconName::X, move |_, _, cx| {
            ws_rm.update(cx, |this, cx| this.remove_provider_env_row(&pid_rm, ix, cx));
        }))
}
