//! The saved-prompts store — split from `persist.rs` to stay under the
//! SLOC cap. Same atomic tmp+rename writes and skip-if-unchanged reads.

use std::fs;

use serde::{Deserialize, Serialize};

/// On-disk wrapper for the saved-prompts store so a future format bump can
/// reject unknown versions.
#[derive(Serialize, Deserialize)]
struct StoredPrompts {
    v: u32,
    prompts: Vec<crate::prompts::SavedPrompt>,
}

/// Write the saved prompts to `dir/prompts.json` (atomic tmp+rename). An
/// empty store removes the file so a cleared list stays cleared.
pub fn save_prompts(dir: &std::path::Path, store: &crate::prompts::PromptStore) {
    let path = dir.join("prompts.json");
    if store.prompts.is_empty() {
        let _ = fs::remove_file(path);
        return;
    }
    let stored = StoredPrompts { v: 1, prompts: store.prompts.clone() };
    let Ok(json) = serde_json::to_string_pretty(&stored) else { return };
    // Skip the write when nothing changed — prompt mutations are rare but
    // the file stays byte-identical across unrelated saves.
    if fs::read_to_string(&path).is_ok_and(|old| old == json) {
        return;
    }
    let _ = fs::create_dir_all(dir);
    let tmp = dir.join("prompts.json.tmp");
    let _ = fs::write(&tmp, json);
    let _ = fs::rename(&tmp, &path);
}

/// Read `dir/prompts.json`; an empty store on any error or unknown version.
pub fn load_prompts(dir: &std::path::Path) -> crate::prompts::PromptStore {
    fs::read_to_string(dir.join("prompts.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<StoredPrompts>(&s).ok())
        .filter(|s| s.v == 1)
        .map_or_else(crate::prompts::PromptStore::default, |s| crate::prompts::PromptStore { prompts: s.prompts })
}
