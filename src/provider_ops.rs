//! Provider-instance mutations: selection, add/remove, enable flags and
//! per-model ordering — the Workspace API the picker and settings drive.

use gpui_kit::*;

use crate::model::ModelInfo;
use crate::model_catalog::{resolve_model, resolve_provider};
use crate::providers::{ModelConfig, ProviderInstance, ProviderKind};
use crate::workspace::Workspace;

impl Workspace {
    /// Select `model_id` under instance `instance_id` — the picker's single
    /// entry point. Switches the active backend when the instance changes.
    /// Returns false for unknown/disabled instances and unknown model ids.
    pub fn select_model(&mut self, instance_id: &str, model_id: &str, cx: &mut Context<Self>) -> bool {
        let Some(p) = self.providers.iter().find(|p| p.id == instance_id) else { return false };
        if !p.enabled || !self.models_for(instance_id).iter().any(|m| m.id.as_ref() == model_id) {
            return false;
        }
        if self.selected_provider != instance_id {
            self.select_instance(instance_id);
        }
        self.model = model_id.into();
        // Keep the effort pick when the new model advertises it, else
        // fall back to the new model's `default_effort`.
        self.reconcile_effort();
        self.save_settings();
        cx.notify();
        true
    }

    /// Set the provider+model new threads start on and persist it —
    /// `Settings.default_model`. Unlike `select_model` this never touches
    /// the active thread's selection or backend.
    pub fn set_default_model(&mut self, instance_id: &str, model_id: &str, cx: &mut Context<Self>) {
        self.default_model = crate::persist::DefaultModel {
            provider_instance_id: instance_id.to_string(),
            model_id: model_id.to_string(),
        };
        self.save_settings();
        cx.notify();
    }

    /// Add a provider instance of `kind` and return its unique id. The new
    /// instance is enabled but not selected; its catalog seeds from the
    /// kind's static `models()`.
    pub fn add_provider(&mut self, kind: ProviderKind, name: String, cx: &mut Context<Self>) -> String {
        let id = self.next_instance_id(kind);
        // Infallible: `next_instance_id` guarantees `id` is free and slugs
        // are already valid id characters.
        self.add_provider_with_id(kind, ProviderDraft { name, id, accent: None }, cx).unwrap_or_default()
    }

    /// The kind's slug with a `-N` suffix until it's free — the id the
    /// wizard seeds and `add_provider` lands on.
    pub fn next_instance_id(&self, kind: ProviderKind) -> String {
        let mut id = kind.slug().to_string();
        for n in 2.. {
            if !self.providers.iter().any(|p| p.id == id) {
                break;
            }
            id = format!("{}-{n}", kind.slug());
        }
        id
    }

    /// Add a provider instance with a caller-chosen id (the add-provider
    /// wizard's Instance ID field). Returns `Some(id)` on success; `None`
    /// when the id is empty, contains characters outside `[a-zA-Z0-9_-]`,
    /// or is already taken.
    pub fn add_provider_with_id(&mut self, kind: ProviderKind, draft: ProviderDraft, cx: &mut Context<Self>) -> Option<String> {
        let id = draft.id.trim();
        if id.is_empty()
            || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || self.providers.iter().any(|p| p.id == id)
        {
            return None;
        }
        let mut p = ProviderInstance::new(kind, draft.name);
        p.id = id.to_string();
        p.accent = draft.accent;
        self.model_catalog.insert(id.to_string(), crate::backend::backend_for(&p).models());
        self.providers.push(p);
        self.save_settings();
        cx.notify();
        Some(id.to_string())
    }

    /// Rename an instance's display label. Empty/whitespace names and
    /// no-ops are ignored so the field can be cleared while editing.
    pub fn rename_provider(&mut self, instance_id: &str, name: String, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let Some(p) = self.providers.iter_mut().find(|p| p.id == instance_id) else { return };
        if p.name == name {
            return;
        }
        p.name = name.to_string();
        self.save_settings();
        cx.notify();
    }

    /// Remove a provider instance. Removing the selected instance moves the
    /// selection to the first enabled one; removing the last instance
    /// leaves the selection empty (sends error until a provider is added).
    pub fn remove_provider(&mut self, instance_id: &str, cx: &mut Context<Self>) {
        let Some(ix) = self.providers.iter().position(|p| p.id == instance_id) else { return };
        self.providers.remove(ix);
        self.model_catalog.remove(instance_id);
        if self.selected_provider == instance_id {
            self.selected_provider = String::new();
            self.model = String::new().into();
            self.select_first_enabled();
        }
        self.save_settings();
        cx.notify();
    }

    /// Enable/disable an instance from the Providers settings section.
    /// Disabling the selected instance moves the selection to the first
    /// remaining enabled one; disabling the last enabled instance leaves
    /// the selection empty.
    pub fn set_provider_enabled(&mut self, instance_id: &str, on: bool, cx: &mut Context<Self>) {
        let Some(p) = self.providers.iter_mut().find(|p| p.id == instance_id) else { return };
        if p.enabled == on {
            return;
        }
        p.enabled = on;
        if !on && self.selected_provider == instance_id {
            self.selected_provider = String::new();
            self.model = String::new().into();
            self.select_first_enabled();
        }
        self.save_settings();
        cx.notify();
    }

    /// Enable/disable one model in an instance's picker list. The first
    /// touch materializes the catalog order into `models` config so the
    /// toggle has stable positions to work on.
    pub fn set_model_enabled(&mut self, instance_id: &str, model_id: &str, on: bool, cx: &mut Context<Self>) {
        self.materialize_model_config(instance_id);
        let Some(p) = self.providers.iter_mut().find(|p| p.id == instance_id) else { return };
        let Some(c) = p.models.iter_mut().find(|c| c.id == model_id) else { return };
        c.enabled = on;
        self.reselect_model_if_dropped(instance_id);
        self.save_settings();
        cx.notify();
    }

    /// Move a model one step in the picker order — `dir` < 0 moves it
    /// earlier, > 0 later. Materializes the config like
    /// `set_model_enabled`.
    pub fn move_model(&mut self, instance_id: &str, model_id: &str, dir: i32, cx: &mut Context<Self>) {
        self.materialize_model_config(instance_id);
        let Some(p) = self.providers.iter_mut().find(|p| p.id == instance_id) else { return };
        p.models.sort_by_key(|c| c.order);
        let Some(ix) = p.models.iter().position(|c| c.id == model_id) else { return };
        let next = ix as i64 + dir.signum() as i64;
        if !(0..p.models.len() as i64).contains(&next) {
            return;
        }
        p.models.swap(ix, next as usize);
        for (i, c) in p.models.iter_mut().enumerate() {
            c.order = i as u32;
        }
        self.save_settings();
        cx.notify();
    }

    /// Update an instance's connection fields (acp command, http url +
    /// key env) and rebuild the backend when it's the selected one.
    pub(crate) fn configure_provider(&mut self, instance_id: &str, command: String, key_env: String) {
        let Some(p) = self.providers.iter_mut().find(|p| p.id == instance_id) else { return };
        p.command = command;
        p.key_env = key_env;
        self.rebuild_selected_backend(instance_id);
        self.save_settings();
    }

    /// Write one Variables row — `ix` indexes `p.env`, `key`/`value` are
    /// the row's current text. Blank keys stay in memory (the row is still
    /// being edited) but are skipped on save and at spawn.
    pub fn set_provider_env(&mut self, instance_id: &str, ix: usize, pair: (String, String), cx: &mut Context<Self>) {
        let Some(p) = self.providers.iter_mut().find(|p| p.id == instance_id) else { return };
        let Some(row) = p.env.get_mut(ix) else { return };
        if row.0 == pair.0 && row.1 == pair.1 {
            return;
        }
        *row = pair;
        self.rebuild_selected_backend(instance_id);
        self.save_settings();
        cx.notify();
    }

    /// Append a blank Variables row — the detail panel's "Add variable".
    pub fn add_provider_env_row(&mut self, instance_id: &str, cx: &mut Context<Self>) {
        let Some(p) = self.providers.iter_mut().find(|p| p.id == instance_id) else { return };
        p.env.push((String::new(), String::new()));
        self.save_settings();
        cx.notify();
    }

    /// Remove Variables row `ix`.
    pub fn remove_provider_env_row(&mut self, instance_id: &str, ix: usize, cx: &mut Context<Self>) {
        let Some(p) = self.providers.iter_mut().find(|p| p.id == instance_id) else { return };
        if ix >= p.env.len() {
            return;
        }
        p.env.remove(ix);
        self.rebuild_selected_backend(instance_id);
        self.save_settings();
        cx.notify();
    }

    /// Rebuild the live backend when `instance_id` is the selected one —
    /// env/connection edits must reach the next spawned subprocess.
    fn rebuild_selected_backend(&mut self, instance_id: &str) {
        if self.selected_provider == instance_id
            && let Some(p) = self.providers.iter().find(|p| p.id == instance_id)
        {
            self.backend = crate::backend::backend_for(p);
        }
    }

    /// Switch the active instance: rebuild its backend and re-resolve the
    /// model selection against the new catalog.
    fn select_instance(&mut self, instance_id: &str) {
        let Some(p) = self.providers.iter().find(|p| p.id == instance_id) else { return };
        self.backend = crate::backend::backend_for(p);
        self.selected_provider = p.id.clone();
        self.model = resolve_model(self.catalog_of(instance_id), &p.models, &self.model.clone()).into();
    }

    /// Point the selection at the first enabled instance (or clear it when
    /// none remain) and rebuild the backend.
    fn select_first_enabled(&mut self) {
        match resolve_provider(&self.providers, "") {
            Some(id) => {
                let id = id.to_string();
                self.select_instance(&id);
            },
            None => {
                self.backend = std::sync::Arc::new(crate::backend::SimBackend);
            },
        }
    }

    /// The instance's raw catalog — statics/cache/fetched, before the
    /// `models` config filter.
    fn catalog_of(&self, instance_id: &str) -> &[ModelInfo] {
        self.model_catalog.get(instance_id).map_or(&[], Vec::as_slice)
    }

    /// Expand an empty `models` config to cover the catalog in order, so
    /// enable/order edits have entries to land on. No-op when the config
    /// is already populated or the catalog is empty.
    fn materialize_model_config(&mut self, instance_id: &str) {
        let catalog = self.model_catalog.get(instance_id).cloned().unwrap_or_default();
        let Some(p) = self.providers.iter_mut().find(|p| p.id == instance_id) else { return };
        if !p.models.is_empty() {
            return;
        }
        p.models = catalog
            .iter()
            .enumerate()
            .map(|(i, m)| ModelConfig { id: m.id.to_string(), enabled: true, order: i as u32 })
            .collect();
    }

    /// After a model-config change, re-resolve the selection when the
    /// selected instance dropped the selected model.
    fn reselect_model_if_dropped(&mut self, instance_id: &str) {
        if self.selected_provider != instance_id {
            return;
        }
        if !self.models_for(instance_id).iter().any(|m| m.id == self.model) {
            let p = self.providers.iter().find(|p| p.id == instance_id);
            let config = p.map_or(&[][..], |p| p.models.as_slice());
            self.model = resolve_model(self.catalog_of(instance_id), config, &self.model.clone()).into();
        }
    }
}

/// The wizard's instance fields bundled for `add_provider_with_id` — keeps
/// the signature under the argument-count lint.
pub struct ProviderDraft {
    pub name: String,
    pub id: String,
    pub accent: Option<String>,
}
