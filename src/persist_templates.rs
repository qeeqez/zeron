//! Prompt-template persistence — `<project>/templates.json`. Split from
//! `persist.rs` for the SLOC cap; re-exported from `persist` so callers
//! keep `crate::persist::*` paths.

use std::fs;
use std::path::Path;

use crate::prompts::{Template, TemplateStore};
use serde::{Deserialize, Serialize};

/// On-disk wrapper so a future format bump can reject unknown versions.
#[derive(Serialize, Deserialize)]
struct StoredTemplates {
    v: u32,
    templates: Vec<Template>,
}

/// Write the templates to `dir/templates.json` (atomic tmp+rename). An
/// empty store removes the file so a cleared list stays cleared.
pub fn save_templates(dir: &Path, store: &TemplateStore) {
    let path = dir.join("templates.json");
    if store.templates.is_empty() {
        let _ = fs::remove_file(path);
        return;
    }
    let stored = StoredTemplates { v: 1, templates: store.templates.clone() };
    let Ok(json) = serde_json::to_string_pretty(&stored) else { return };
    // Skip the write when nothing changed — the file stays byte-identical
    // across unrelated saves.
    if fs::read_to_string(&path).is_ok_and(|old| old == json) {
        return;
    }
    let _ = fs::create_dir_all(dir);
    let tmp = dir.join("templates.json.tmp");
    let _ = fs::write(&tmp, json);
    let _ = fs::rename(&tmp, &path);
}

/// Read `dir/templates.json`; an empty store on any error or unknown
/// version.
pub fn load_templates(dir: &Path) -> TemplateStore {
    fs::read_to_string(dir.join("templates.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<StoredTemplates>(&s).ok())
        .filter(|s| s.v == 1)
        .map_or_else(TemplateStore::default, |s| TemplateStore { templates: s.templates })
}
