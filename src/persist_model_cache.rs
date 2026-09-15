//! Model-catalog cache persistence — split from `persist.rs` for the SLOC cap.
//! Re-exported from `persist` so callers keep `crate::persist::*` paths.

use std::fs;
use std::path::PathBuf;

/// Cache file for fetched model catalogs — lets the picker show last
/// session's real models before the refresh lands and when offline.
fn model_cache_path() -> PathBuf {
    crate::persist::dirs_home().join(".rixl/rixlcode/models.json")
}

/// Load cached catalogs keyed by provider id; empty on any error.
pub fn load_model_cache() -> std::collections::HashMap<String, Vec<crate::model::ModelInfo>> {
    fs::read_to_string(model_cache_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Merge a fetched catalog into the cache file (read-modify-write so
/// providers fetched at different times keep their entries).
pub fn save_model_cache(provider: &str, models: &[crate::model::ModelInfo]) {
    let mut cache = load_model_cache();
    cache.insert(provider.to_string(), models.to_vec());
    let path = model_cache_path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(&cache) {
        let _ = fs::write(&tmp, json);
        let _ = fs::rename(&tmp, &path);
    }
}
