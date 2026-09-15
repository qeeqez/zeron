//! Provider instances: the user-configured providers the picker and
//! settings operate on. A `ProviderKind` carries the static metadata every
//! instance of that kind shares (label, tagline, catalog fetch); a
//! `ProviderInstance` is one configured entry — its own id, name, enabled
//! flag, connection fields and per-model configuration.

use gpui_kit::assets::IconName;

use crate::model::ModelInfo;

/// A provider catalog refresh — runs on a background thread, returns the
/// real model list or an error the picker ignores (cache/statics remain).
pub type ModelFetch = fn() -> Result<Vec<ModelInfo>, String>;

/// One backend implementation's provider identity. The serde names match
/// the legacy `backend` setting so old files migrate cleanly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    /// `codex app-server` over stdio — the default provider.
    #[default]
    CodexCli,
    ClaudeCli,
    /// Agent Client Protocol subprocess (Zed-style agents).
    Acp,
    /// Custom NDJSON HTTP endpoint.
    Http,
    /// Built-in simulator — no subprocess.
    Sim,
}

/// Static per-kind metadata — the registry of *kinds*, not instances.
pub struct ProviderKindInfo {
    /// Persisted slug; also the default instance id.
    pub slug: &'static str,
    pub label: &'static str,
    /// One-line description shown in the Providers settings section.
    pub tagline: &'static str,
    /// Icon shown next to instances of this kind (picker, settings).
    pub icon: IconName,
    /// Refresh an instance's catalog from the provider itself; `None` for
    /// kinds whose `models()` list is already complete.
    pub fetch: Option<ModelFetch>,
}

impl ProviderKind {
    /// All kinds in picker display order — codex-cli first, it's the default.
    pub const ALL: [ProviderKind; 5] = [Self::CodexCli, Self::ClaudeCli, Self::Acp, Self::Http, Self::Sim];

    pub fn info(self) -> &'static ProviderKindInfo {
        const CODEX: ProviderKindInfo = ProviderKindInfo {
            slug: "codex-cli",
            label: "Codex",
            tagline: "codex app-server over stdio",
            icon: IconName::Bot,
            fetch: Some(crate::backend::fetch_codex_models),
        };
        const CLAUDE: ProviderKindInfo = ProviderKindInfo {
            slug: "claude-cli",
            label: "Claude",
            tagline: "claude CLI over stdio",
            icon: IconName::Sparkles,
            fetch: None,
        };
        const ACP: ProviderKindInfo = ProviderKindInfo {
            slug: "acp",
            label: "ACP",
            tagline: "Agent Client Protocol agent",
            icon: IconName::Network,
            fetch: None,
        };
        const HTTP: ProviderKindInfo = ProviderKindInfo {
            slug: "http",
            label: "HTTP",
            tagline: "custom NDJSON endpoint",
            icon: IconName::Globe,
            fetch: None,
        };
        const SIM: ProviderKindInfo = ProviderKindInfo {
            slug: "sim",
            label: "Sim",
            tagline: "built-in simulator (no backend)",
            icon: IconName::FlaskConical,
            fetch: None,
        };
        match self {
            Self::CodexCli => &CODEX,
            Self::ClaudeCli => &CLAUDE,
            Self::Acp => &ACP,
            Self::Http => &HTTP,
            Self::Sim => &SIM,
        }
    }

    /// The persisted slug — matches the serde rename and legacy `backend` ids.
    pub fn slug(self) -> &'static str {
        self.info().slug
    }

    /// Default `command` for a fresh instance: the ACP agent command, empty
    /// for kinds without a spawn command/endpoint.
    pub fn default_command(self) -> &'static str {
        match self {
            Self::Acp => crate::backend::AcpBackend::DEFAULT_COMMAND,
            _ => "",
        }
    }

    /// Default `key_env` for a fresh instance — only http reads it.
    pub fn default_key_env(self) -> &'static str {
        match self {
            Self::Http => "RIXL_API_KEY",
            _ => "",
        }
    }
}

/// One configured provider — a user-added instance of a `ProviderKind`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProviderInstance {
    /// Unique slug — persisted, stable; the picker's provider key.
    pub id: String,
    pub kind: ProviderKind,
    /// User-facing label (editable).
    pub name: String,
    pub enabled: bool,
    /// acp/claude spawn command or http endpoint url — kind-dependent.
    pub command: String,
    /// Env var holding the http bearer token; empty for other kinds.
    pub key_env: String,
    /// Extra environment for the backend subprocess (e.g. a custom base
    /// URL or API key for this instance only). Rows with a blank key are
    /// half-edited UI state — skipped on save and at spawn.
    #[serde(with = "env_map", skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
    /// Per-model enable + order; empty = all enabled, catalog order.
    pub models: Vec<ModelConfig>,
    /// Optional accent color (hex) shown as a marker in the picker.
    pub accent: Option<String>,
}

/// `env` serializes as a JSON object (`{"KEY": "value"}`) so the settings
/// file stays hand-editable; in memory it's an ordered row list matching
/// the Variables editor. Blank keys — a row the user added but hasn't
/// named yet — are dropped on save.
mod env_map {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(env: &[(String, String)], s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(env.iter().filter(|(k, _)| !k.trim().is_empty()).map(|(k, v)| (k, v)))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, String)>, D::Error> {
        Ok(<std::collections::BTreeMap<String, String>>::deserialize(d)?.into_iter().collect())
    }
}

impl ProviderInstance {
    /// A fresh instance of `kind` — id defaults to the kind slug (callers
    /// dedup), enabled, with the kind's default connection fields.
    pub fn new(kind: ProviderKind, name: String) -> Self {
        Self {
            id: kind.slug().to_string(),
            kind,
            name,
            enabled: true,
            command: kind.default_command().to_string(),
            key_env: kind.default_key_env().to_string(),
            env: Vec::new(),
            models: Vec::new(),
            accent: None,
        }
    }
}

/// Per-model enable + order inside a `ProviderInstance`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub id: String,
    pub enabled: bool,
    pub order: u32,
}

/// Apply an instance's `models` config to its catalog: configured models
/// come first in `order` with disabled ones dropped; catalog models with no
/// config entry keep catalog order after them. Empty config = catalog as-is.
pub fn apply_model_config(catalog: &[ModelInfo], config: &[ModelConfig]) -> Vec<ModelInfo> {
    if config.is_empty() {
        return catalog.to_vec();
    }
    let mut ordered: Vec<&ModelConfig> = config.iter().collect();
    ordered.sort_by_key(|c| c.order);
    let mut out: Vec<ModelInfo> = ordered
        .iter()
        .filter(|c| c.enabled)
        .filter_map(|c| catalog.iter().find(|m| m.id.as_ref() == c.id).cloned())
        .collect();
    out.extend(catalog.iter().filter(|m| !config.iter().any(|c| c.id == m.id.as_ref())).cloned());
    out
}
