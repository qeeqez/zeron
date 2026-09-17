//! Tests for `git_watch` — the `fingerprint` helper against real temp repos
//! (skipped when git is unavailable) and the poll/debounce gates on
//! `GitWatch`. Sibling file so `git_watch.rs` stays under the SLOC cap.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use crate::git::watch::{GitWatch, POLL_TICKS, REFRESH_DEBOUNCE, fingerprint};

    /// Run `git` in `dir`; true on exit 0. Real-repo tests skip when it fails.
    fn git_ok(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// A fresh `git init` temp dir, or `None` when git won't run — callers
    /// return early, matching the other real-repo tests.
    fn repo(name: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git_ok(&dir, &["init", "-q"]).then_some(dir)
    }

    fn fp(dir: &Path) -> Option<u64> {
        fingerprint(&[dir.to_path_buf()])
    }

    fn commit(dir: &Path, message: &str) {
        assert!(git_ok(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", message]));
    }

    #[test]
    fn fingerprint_is_none_outside_a_repo() {
        let dir = std::env::temp_dir().join(format!("rixlcode-watch-norepo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(fp(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fingerprint_is_stable_when_the_tree_is_idle() {
        let Some(dir) = repo("idle") else { return };
        assert_eq!(fp(&dir), fp(&dir));
        assert!(fp(&dir).is_some(), "an empty repo still fingerprints");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every kind of dirty the panel renders — untracked, staged, committed
    /// (HEAD oid) and unstaged — must move the fingerprint.
    #[test]
    fn fingerprint_moves_on_each_change_kind() {
        let Some(dir) = repo("kinds") else { return };
        let base = fp(&dir);

        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        let untracked = fp(&dir);
        assert_ne!(base, untracked, "untracked file");

        assert!(git_ok(&dir, &["add", "a.txt"]));
        let staged = fp(&dir);
        assert_ne!(untracked, staged, "staged add");

        commit(&dir, "init");
        let committed = fp(&dir);
        assert_ne!(staged, committed, "commit moved HEAD");

        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        let unstaged = fp(&dir);
        assert_ne!(committed, unstaged, "unstaged edit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `refs/stash` is part of the fingerprint so the panel's stash count
    /// refreshes on an external `git stash` too.
    #[test]
    fn fingerprint_moves_on_stash() {
        let Some(dir) = repo("stash") else { return };
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        assert!(git_ok(&dir, &["add", "a.txt"]));
        commit(&dir, "init");
        let clean = fp(&dir);
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        assert!(git_ok(&dir, &["-c", "user.email=t@t", "-c", "user.name=t", "stash", "push", "-qm", "wip"]));
        assert_ne!(clean, fp(&dir), "stash push");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dirs are mixed into the hash, so two identical clean checkouts still
    /// differ — switching chats re-baselines instead of comparing across
    /// worktrees.
    #[test]
    fn fingerprint_includes_the_dir() {
        let (Some(a), Some(b)) = (repo("dir-a"), repo("dir-b")) else { return };
        assert_ne!(fp(&a), fp(&b));
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    /// A multi-dir poll answers `Some` while at least one dir is a repo —
    /// a worktree chat fingerprints its checkout plus the project root.
    #[test]
    fn fingerprint_is_some_when_any_dir_is_a_repo() {
        let Some(dir) = repo("mixed") else { return };
        let norepo = std::env::temp_dir().join(format!("rixlcode-watch-mixed-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&norepo);
        std::fs::create_dir_all(&norepo).unwrap();
        assert!(fingerprint(&[norepo.clone(), dir.clone()]).is_some());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&norepo);
    }

    #[test]
    fn due_fires_on_cadence_and_never_while_in_flight() {
        let mut w = GitWatch::default();
        for _ in 0..POLL_TICKS - 1 {
            assert!(!w.due());
        }
        assert!(w.due(), "poll starts on the cadence beat");
        assert!(!w.due(), "a second poll doesn't stack while in flight");
        w.land(Some(1), Instant::now());
        assert!(w.due(), "the skipped beats count toward the next poll");
    }

    #[test]
    fn land_baselines_silently_then_reports_moves() {
        let mut w = GitWatch::default();
        let t0 = Instant::now();
        assert!(!w.land(Some(7), t0), "first landing only sets the baseline");
        assert!(w.land(Some(8), t0), "a moved fingerprint refreshes");
        assert!(!w.land(Some(8), t0 + Duration::from_secs(1)), "identical state stays quiet");
    }

    /// `None` means git couldn't answer — it must not clear the baseline or
    /// trigger a refresh, so transient failures don't flicker the panel.
    #[test]
    fn land_ignores_failed_fingerprints() {
        let mut w = GitWatch::default();
        let t0 = Instant::now();
        w.land(Some(1), t0);
        assert!(!w.land(None, t0));
        assert!(w.land(Some(2), t0 + Duration::from_secs(1)), "the baseline survives a failed poll");
    }

    /// The first land can be a non-repo (`None`); a repo appearing later —
    /// `git init` in the project dir — still counts as a change.
    #[test]
    fn land_refreshes_when_a_repo_appears() {
        let mut w = GitWatch::default();
        let t0 = Instant::now();
        assert!(!w.land(None, t0));
        assert!(w.land(Some(9), t0));
    }

    /// A second move inside the debounce window is suppressed, but the old
    /// baseline is kept so the next landing retries the refresh rather than
    /// dropping it.
    #[test]
    fn land_debounces_then_retries() {
        let mut w = GitWatch::default();
        let t0 = Instant::now();
        w.land(Some(1), t0);
        assert!(w.land(Some(2), t0));
        assert!(!w.land(Some(3), t0 + Duration::from_millis(100)), "inside the debounce window");
        assert!(w.land(Some(3), t0 + REFRESH_DEBOUNCE + Duration::from_millis(1)), "retry fires it");
    }

    /// `note_refresh` feeds manual refreshes into the same gate, so a watch
    /// refresh doesn't pile onto a refresh the user just ran.
    #[test]
    fn note_refresh_debounces_the_watch() {
        let mut w = GitWatch::default();
        w.land(Some(1), Instant::now() - Duration::from_secs(10));
        w.note_refresh();
        assert!(!w.land(Some(2), Instant::now()), "a fresh manual refresh suppresses the watch");
    }

    /// While nothing watches, `reset` keeps the watch armed: the first beat
    /// after a view opens polls immediately and re-baselines silently.
    #[test]
    fn reset_rearms_the_first_beat() {
        let mut w = GitWatch::default();
        w.land(Some(1), Instant::now());
        w.reset();
        assert!(w.due(), "armed to poll on the next beat");
        assert!(!w.land(Some(1), Instant::now()), "same state re-baselines, no refresh");
    }
}
