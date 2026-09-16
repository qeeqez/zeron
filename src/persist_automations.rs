//! Scheduled-prompt persistence — `<project>/automations.json`. Split
//! from `persist.rs` for the SLOC cap; re-exported from `persist` so
//! callers keep `crate::persist::*` paths.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::automations::Automation;

/// On-disk wrapper so a future format bump can reject unknown versions.
#[derive(Serialize, Deserialize)]
struct StoredAutomations {
    v: u32,
    automations: Vec<Automation>,
}

/// Write the automations to `dir/automations.json` (atomic tmp+rename).
/// An empty list removes the file so a cleared schedule stays cleared.
pub fn save_automations(dir: &Path, automations: &[Automation]) {
    let path = dir.join("automations.json");
    if automations.is_empty() {
        let _ = fs::remove_file(path);
        return;
    }
    let stored = StoredAutomations { v: 1, automations: automations.to_vec() };
    let Ok(json) = serde_json::to_string_pretty(&stored) else { return };
    // Skip the write when nothing changed — the scheduler persists on
    // every fire, but the file stays byte-identical across unrelated saves.
    if fs::read_to_string(&path).is_ok_and(|old| old == json) {
        return;
    }
    let _ = fs::create_dir_all(dir);
    let tmp = dir.join("automations.json.tmp");
    let _ = fs::write(&tmp, json);
    let _ = fs::rename(&tmp, &path);
}

/// Read `dir/automations.json`; an empty list on any error or unknown
/// version.
pub fn load_automations(dir: &Path) -> Vec<Automation> {
    fs::read_to_string(dir.join("automations.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<StoredAutomations>(&s).ok())
        .filter(|s| s.v == 1)
        .map_or_else(Vec::new, |s| s.automations)
}
