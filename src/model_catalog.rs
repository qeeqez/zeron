//! Model catalog: per-provider model lists for the composer picker.
//!
//! Each provider's list is seeded from `AgentBackend::models()` (static
//! catalogs), overlaid with the persisted cache, then refreshed in the
//! background by `ProviderInfo::fetch` (codex's `model/list`). The picker
//! always prepends a synthetic `default` entry — it never lives in a
//! catalog.

use std::collections::HashMap;

use gpui_kit::*;

use crate::model::{ModelInfo, PROVIDERS, ProviderInfo};
use crate::workspace::Workspace;

/// The picker's synthetic first entry for every provider — `send` maps it
/// to the provider's own default.
pub(crate) fn default_model() -> ModelInfo {
    ModelInfo {
        id: "default".into(),
        label: "default".into(),
        description: SharedString::default(),
    }
}

/// Resolve a persisted provider id to a registry entry: the id must exist
/// and be enabled; otherwise the first enabled provider wins.
pub(crate) fn pick_provider(desired: &str, disabled: &[String]) -> &'static str {
    let usable = |p: &&ProviderInfo| !disabled.iter().any(|d| d == p.id);
    PROVIDERS
        .iter()
        .find(|p| p.id == desired && usable(p))
        .or_else(|| PROVIDERS.iter().find(|p| usable(p)))
        .map_or("codex-cli", |p| p.id)
}
/// Seed every provider's catalog: static `models()` first, then the
/// persisted cache overlays providers that fetch at runtime. A provider
/// whose backend can't be built (http without an endpoint) seeds empty —
/// the picker still offers `default`.
pub(crate) fn seed_catalog(s: &crate::persist::Settings) -> HashMap<String, Vec<ModelInfo>> {
    let cache = crate::persist::load_model_cache();
    PROVIDERS
        .iter()
        .map(|p| {
            let backend = crate::backend::backend_for(p.id, &s.http_url, &s.http_key_env);
            let statics = if backend.provider_id() == p.id { backend.models() } else { Vec::new() };
            (p.id.to_string(), cache.get(p.id).cloned().unwrap_or(statics))
        })
        .collect()
}

impl Workspace {
    /// Providers not disabled in settings, in registry order.
    pub(crate) fn enabled_providers(&self) -> Vec<&'static ProviderInfo> {
        PROVIDERS.iter().filter(|p| !self.disabled_providers.iter().any(|d| d == p.id)).collect()
    }

    /// The provider's catalog without the synthetic `default` entry.
    pub(crate) fn models_for(&self, provider: &str) -> Vec<ModelInfo> {
        self.model_catalog.get(provider).cloned().unwrap_or_default()
    }

    /// Rebuild the active backend for `provider`, honoring configured
    /// provider settings (acp's command, http's endpoint) via
    /// `make_backend` — `backend_for` alone would drop them.
    fn build_backend(&self, provider: &'static str) -> std::sync::Arc<dyn crate::backend::AgentBackend> {
        let mut s = crate::persist::load_settings();
        s.backend = provider.to_string();
        s.use_codex_cli = None;
        s.http_url = self.http_url.clone();
        s.http_key_env = self.http_key_env.clone();
        crate::backend::make_backend(&s)
    }

    /// Select `model` under `provider` — the picker's single entry point.
    /// Switches the active backend when the provider changes. Returns
    /// false for unknown/disabled providers and unknown model ids.
    pub(crate) fn select_model(&mut self, provider: &str, model: &str, cx: &mut Context<Self>) -> bool {
        let Some(p) = PROVIDERS.iter().find(|p| p.id == provider) else { return false };
        if self.disabled_providers.iter().any(|d| d == p.id) {
            return false;
        }
        if model != "default" && !self.models_for(p.id).iter().any(|m| m.id.as_ref() == model) {
            return false;
        }
        if self.provider != p.id {
            self.switch_provider(p.id);
        }
        self.model = model.into();
        self.save_settings();
        cx.notify();
        true
    }

    /// The picker's option list for a provider: synthetic `default` first,
    /// then the catalog.
    pub(crate) fn picker_options(&self, provider: &str) -> Vec<ModelInfo> {
        std::iter::once(default_model()).chain(self.models_for(provider)).collect()
    }

    /// Enable/disable a provider from the Providers settings section. The
    /// last enabled provider can't be disabled; disabling the active one
    /// moves the selection to the first remaining provider.
    pub(crate) fn set_provider_enabled(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        if !PROVIDERS.iter().any(|p| p.id == id) {
            return;
        }
        if enabled {
            self.disabled_providers.retain(|d| d != id);
        } else {
            if self.enabled_providers().iter().all(|p| p.id != id) {
                return; // already disabled
            }
            if self.enabled_providers().len() <= 1 {
                return; // never disable the last provider
            }
            self.disabled_providers.push(id.to_string());
            if self.provider == id {
                self.switch_provider(self.enabled_providers().first().map_or("codex-cli", |p| p.id));
            }
        }
        self.save_settings();
        cx.notify();
    }

    /// Refresh every enabled provider that has a `fetch` — off the UI
    /// thread, one background task per provider. Test builds skip the
    /// spawn: tests inject catalogs via `land_catalog` instead of running
    /// a real `codex app-server`.
    pub(crate) fn refresh_model_catalogs(&self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        for p in PROVIDERS {
            let Some(fetch) = p.fetch else { continue };
            if self.disabled_providers.iter().any(|d| d == p.id) {
                continue;
            }
            Self::spawn_catalog_fetch(p.id, fetch, cx);
        }
    }

    /// One provider's catalog refresh: run `fetch` on the background
    /// executor, then land the result on the UI thread.
    fn spawn_catalog_fetch(provider: &'static str, fetch: crate::model::ModelFetch, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async move { fetch() });
        cx.spawn(async move |this, cx| {
            let Ok(models) = task.await else { return };
            let _ = this.update(cx, |this, cx| this.land_catalog(provider, models, cx));
        })
        .detach();
    }

    /// Switch the active provider: rebuild the backend and reset the model
    /// selection when the new provider's catalog doesn't carry it.
    fn switch_provider(&mut self, next: &'static str) {
        self.provider = next;
        self.backend = self.build_backend(next);
        if self.model != "default" && !self.models_for(next).iter().any(|m| m.id == self.model) {
            self.model = "default".into();
        }
    }

    /// Publish a fetched catalog: update the picker's list, persist it to
    /// the model cache, and reset the selection to `default` if the active
    /// provider dropped the selected model.
    pub(crate) fn land_catalog(&mut self, provider: &'static str, models: Vec<ModelInfo>, cx: &mut Context<Self>) {
        if models.is_empty() {
            return; // an empty page means a broken fetch — keep the old list
        }
        crate::persist::save_model_cache(provider, &models);
        self.model_catalog.insert(provider.to_string(), models);
        if self.provider == provider && self.model != "default" && !self.models_for(provider).iter().any(|m| m.id == self.model) {
            self.model = "default".into();
            self.save_settings();
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Narrow imports only: `use super::*` would pull `gpui_kit::test` into
    // scope (the parent glob-imports gpui_kit), shadowing the builtin `test`
    // attribute with the proc macro - which re-emits `#[test]` and recurses
    // until rustc's stack blows.
    use super::{default_model, pick_provider, seed_catalog};
    use crate::model::PROVIDERS;

    #[test]
    fn pick_provider_falls_back_when_unknown_or_disabled() {
        assert_eq!(pick_provider("sim", &[]), "sim");
        assert_eq!(pick_provider("nope", &[]), "codex-cli");
        assert_eq!(pick_provider("sim", &["sim".to_string()]), "codex-cli");
        // All disabled → still a valid provider, never an empty picker.
        let all: Vec<String> = PROVIDERS.iter().map(|p| p.id.to_string()).collect();
        assert_eq!(pick_provider("sim", &all), "codex-cli");
    }

    #[test]
    fn seed_catalog_uses_statics_and_cache_overlay() {
        let s = crate::persist::Settings::default();
        let catalog = seed_catalog(&s);
        // codex seeds from its static fallback; http has no endpoint so
        // its backend can't be built and it seeds empty.
        assert_eq!(catalog["codex-cli"].len(), crate::model::CODEX_FALLBACK.len());
        assert!(catalog["http"].is_empty());
        assert!(catalog.contains_key("sim"));
    }

    #[test]
    fn default_model_is_not_in_catalogs() {
        let s = crate::persist::Settings::default();
        let catalog = seed_catalog(&s);
        for models in catalog.values() {
            assert!(models.iter().all(|m| m.id != "default"), "default is synthesized, never stored");
        }
        assert_eq!(default_model().id.as_ref(), "default");
    }
}
