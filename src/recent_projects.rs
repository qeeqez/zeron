//! The recent-projects list — `~/.rixl/rixlcode/recent-projects.json`, most
//! recent first. The empty state and the project switcher render it; every
//! `Workspace` records its project on mount.
//!
//! Kept out of `settings.json` on purpose: recents change on every window
//! open, and two windows writing whole-settings snapshots would lose each
//! other's entries. A dedicated file keeps the read-modify-write small and
//! independent — same shape as `models.json`.

use std::fs;
use std::path::{Path, PathBuf};

/// Cap on the persisted list — the switcher shows a handful, not a history.
const RECENT_PROJECTS_MAX: usize = 10;

fn path() -> PathBuf {
    crate::persist::dirs_home().join(".rixl/rixlcode/recent-projects.json")
}

/// Recent project roots, most recent first. Entries whose folder no longer
/// exists are dropped — opening one would silently fall back to the cwd
/// (`Project::open`), which reads as a no-op.
pub fn list() -> Vec<PathBuf> {
    fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<PathBuf>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
}

/// Record `path` as the most recently opened project. Canonicalizes so the
/// same folder reached via `~` vs an absolute path dedupes to one entry;
/// re-opening an existing entry just bumps it to the front.
pub fn record(dir: &Path) {
    let root = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut recents = list();
    recents.retain(|p| p != &root);
    recents.insert(0, root);
    recents.truncate(RECENT_PROJECTS_MAX);
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(&recents) {
        let _ = fs::write(&tmp, json);
        let _ = fs::rename(&tmp, &path);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{list, record};

    /// Redirect `~` into a throwaway dir; nextest runs each test in its own
    /// process, so no other thread can observe HOME mid-write.
    fn sandbox_home() {
        let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("HOME", &dir) };
    }

    /// A fresh folder under the system temp dir, canonicalized like
    /// `record` leaves it.
    fn temp_dir(leaf: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rixlcode-recents-{}", std::process::id())).join(leaf);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn recents_dedupe_and_order_by_recency() {
        sandbox_home();
        let a = temp_dir("alpha");
        let b = temp_dir("beta");
        record(&a);
        record(&b);
        record(&a);
        assert_eq!(list(), vec![a, b], "re-opening should bump, not duplicate");
    }

    #[test]
    fn recents_drop_missing_folders() {
        sandbox_home();
        let gone = temp_dir("gone");
        record(&gone);
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(list().is_empty(), "a deleted folder must not stay openable");
    }

    #[test]
    fn recents_cap_at_ten() {
        sandbox_home();
        for i in 0..12 {
            record(&temp_dir(&format!("p{i}")));
        }
        let recents = list();
        assert_eq!(recents.len(), 10);
        assert_eq!(recents[0], temp_dir("p11"), "newest first");
    }
}
