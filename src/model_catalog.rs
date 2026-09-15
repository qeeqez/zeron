//! Model catalog: per-instance model lists for the composer picker.
//!
//! Each instance's list is seeded from `AgentBackend::models()` (static
//! catalogs), overlaid with the persisted cache, then refreshed in the
//! background by `ProviderKindInfo::fetch` (codex's `model/list`). There is
//! no synthetic `default` entry — a provider with no catalog shows empty.

use std::collections::HashMap;

use gpui_kit::*;

use crate::model::ModelInfo;
use crate::providers::{ProviderInstance, apply_model_config};
use crate::workspace::Workspace;

/// Codex model ids the picker falls back to when the app-server catalog
/// can't be fetched (codex missing, offline, error).
pub const CODEX_FALLBACK: [&str; 3] = ["gpt-5-codex", "gpt-5", "gpt-5-mini"];

/// Codex's static catalog — used until `model/list` lands and whenever the
/// fetch fails. The efforts are a static approximation; the fetched
/// catalog's `supportedReasoningEfforts` replaces them.
pub fn codex_fallback_models() -> Vec<ModelInfo> {
    CODEX_FALLBACK
        .iter()
        .map(|id| ModelInfo {
            id: (*id).into(),
            label: (*id).into(),
            default_effort: "medium".into(),
            efforts: ["low", "medium", "high"].iter().map(|e| (*e).into()).collect(),
            ..ModelInfo::default()
        })
        .collect()
}

/// Seed every instance's catalog: static `models()` first, then the
/// persisted cache overlays kinds that fetch at runtime. An instance whose
/// backend has no static catalog (http, acp before its first session)
/// seeds empty — the picker shows it empty.
pub(crate) fn seed_catalog(s: &crate::persist::Settings) -> HashMap<String, Vec<ModelInfo>> {
    let cache = crate::persist::load_model_cache();
    s.providers
        .iter()
        .map(|p| {
            let statics = crate::backend::backend_for(p).models();
            (p.id.clone(), cache.get(&p.id).cloned().unwrap_or(statics))
        })
        .collect()
}

/// The instance id to select: `desired` when it exists and is enabled,
/// otherwise the first enabled instance. Empty when nothing is enabled.
pub(crate) fn resolve_provider<'a>(providers: &'a [ProviderInstance], desired: &str) -> Option<&'a str> {
    providers
        .iter()
        .find(|p| p.id == desired && p.enabled)
        .or_else(|| providers.iter().find(|p| p.enabled))
        .map(|p| p.id.as_str())
}

/// The model to select on an instance: `desired` when it's in the effective
/// list, otherwise the first entry. Empty when the catalog is empty.
pub(crate) fn resolve_model(catalog: &[ModelInfo], config: &[crate::providers::ModelConfig], desired: &str) -> String {
    let effective = apply_model_config(catalog, config);
    if effective.iter().any(|m| m.id.as_ref() == desired) {
        desired.to_string()
    } else {
        effective.first().map_or_else(String::new, |m| m.id.to_string())
    }
}

impl Workspace {
    /// Configured provider instances, in user order.
    pub fn provider_instances(&self) -> &[ProviderInstance] {
        &self.providers
    }

    /// The selected instance id — `None` when no instance exists.
    pub fn selected_provider(&self) -> Option<&str> {
        self.providers
            .iter()
            .find(|p| p.id == self.selected_provider)
            .map(|_| self.selected_provider.as_str())
    }

    /// The selected model id — empty when the provider has no catalog.
    pub fn selected_model(&self) -> &str {
        &self.model
    }

    /// The selected instance+model's catalog entry — `None` when the
    /// selection doesn't resolve to a catalog model.
    pub(crate) fn selected_model_info(&self) -> Option<ModelInfo> {
        self.models_for(&self.selected_provider).into_iter().find(|m| m.id.as_ref() == self.model.as_ref())
    }

    /// The effort picker's options for the current selection — the
    /// selected model's `supportedReasoningEfforts`. Empty hides the
    /// picker (non-codex catalogs and unfetched models advertise none).
    pub(crate) fn effort_options(&self) -> Vec<String> {
        self.selected_model_info()
            .map(|m| m.efforts.iter().map(ToString::to_string).collect())
            .unwrap_or_default()
    }

    /// Drop a selected effort the current model doesn't advertise —
    /// called after the selection or catalog changes so a stale effort
    /// can't reach the wire. Models with no advertised list keep any
    /// effort (nothing to validate against).
    pub(crate) fn reconcile_effort(&mut self) {
        let keep = self.effort.as_ref().is_none_or(|e| {
            self.selected_model_info()
                .is_none_or(|m| m.efforts.is_empty() || m.efforts.iter().any(|o| o.as_ref() == e))
        });
        if !keep {
            self.effort = None;
        }
    }

    /// The configured default provider+model for new threads —
    /// `Settings.default_model`. Empty fields mean "follow the current
    /// selection"; `apply_thread_defaults` resolves them at `new_chat`.
    pub fn default_model(&self) -> &crate::persist::DefaultModel {
        &self.default_model
    }

    /// The instance's effective model list: catalog filtered by its
    /// `ModelConfig` (disabled dropped, sorted by `order`). No synthetic
    /// default — an unfetched/empty catalog returns empty.
    pub fn models_for(&self, instance_id: &str) -> Vec<ModelInfo> {
        let Some(p) = self.providers.iter().find(|p| p.id == instance_id) else {
            return Vec::new();
        };
        apply_model_config(self.model_catalog.get(&p.id).map_or(&[], Vec::as_slice), &p.models)
    }

    /// The instance's full catalog with each model's enabled flag, in
    /// picker order — the settings section's per-model toggle list.
    /// Configured models lead in `order`; unconfigured follow in catalog
    /// order and are enabled by default.
    pub(crate) fn models_config_for(&self, instance_id: &str) -> Vec<(ModelInfo, bool)> {
        let Some(p) = self.providers.iter().find(|p| p.id == instance_id) else {
            return Vec::new();
        };
        let catalog = self.model_catalog.get(&p.id).map_or(&[][..], Vec::as_slice);
        let mut ordered: Vec<&crate::providers::ModelConfig> = p.models.iter().collect();
        ordered.sort_by_key(|c| c.order);
        let mut out: Vec<(ModelInfo, bool)> = ordered
            .iter()
            .filter_map(|c| catalog.iter().find(|m| m.id.as_ref() == c.id).map(|m| (m.clone(), c.enabled)))
            .collect();
        out.extend(
            catalog
                .iter()
                .filter(|m| !p.models.iter().any(|c| c.id == m.id.as_ref()))
                .map(|m| (m.clone(), true)),
        );
        out
    }

    /// Enabled instances, in user order — the picker's provider list.
    pub(crate) fn enabled_providers(&self) -> Vec<&ProviderInstance> {
        self.providers.iter().filter(|p| p.enabled).collect()
    }

    /// Refresh every enabled instance whose kind has a `fetch` — off the UI
    /// thread, one background task per instance. Test builds skip the
    /// spawn: tests inject catalogs via `land_catalog` instead of running
    /// a real `codex app-server`.
    pub(crate) fn refresh_model_catalogs(&self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        for p in &self.providers {
            let Some(fetch) = p.kind.info().fetch else { continue };
            if !p.enabled {
                continue;
            }
            Self::spawn_catalog_fetch(p.id.clone(), fetch, cx);
        }
    }

    /// One instance's catalog refresh: run `fetch` on the background
    /// executor, then land the result on the UI thread.
    fn spawn_catalog_fetch(instance: String, fetch: crate::providers::ModelFetch, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async move { fetch() });
        cx.spawn(async move |this, cx| {
            let Ok(models) = task.await else { return };
            let _ = this.update(cx, |this, cx| this.land_catalog(&instance, models, cx));
        })
        .detach();
    }

    /// Publish a fetched catalog: update the picker's list, persist it to
    /// the model cache, and re-resolve the selection if the active
    /// instance dropped the selected model.
    pub(crate) fn land_catalog(&mut self, instance_id: &str, models: Vec<ModelInfo>, cx: &mut Context<Self>) {
        if models.is_empty() {
            return; // an empty page means a broken fetch — keep the old list
        }
        crate::persist::save_model_cache(instance_id, &models);
        self.model_catalog.insert(instance_id.to_string(), models);
        if self.selected_provider == instance_id {
            let effective = self.models_for(instance_id);
            if !effective.iter().any(|m| m.id == self.model) {
                self.model = effective.first().map_or_else(String::new, |m| m.id.to_string()).into();
                self.save_settings();
            }
            // A refreshed catalog may advertise a different effort list —
            // drop a selection the model no longer supports.
            self.reconcile_effort();
        }
        cx.notify();
    }
}
