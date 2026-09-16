//! Tests for `git::pr` — `parse_pr_status` fixtures plus `pr_status` against
//! a PATH-stubbed `gh` script. Declared from `git_pr.rs` so `git_tests.rs`
//! stays under the SLOC cap.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::git::{self, CheckVerdict, PrState};
    use crate::git_parse::parse_pr_status;

    /// A `bin` dir under `parent` whose `gh` runs `body` — a shell script
    /// fragment (e.g. `echo '{…}'` or `exit 1`). Returns the PATH value that
    /// puts the stub first.
    fn fake_gh_path(parent: &Path, body: &str) -> String {
        let bin = parent.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("gh"), format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default())
    }

    /// A scratch dir for `gh`'s `current_dir` — `pr_status` never runs `git`,
    /// so a plain directory is enough.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rixlcode-git-pr-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn pr_status_parses_state_and_check_rollup() {
        let raw = r#"{"number":42,"url":"https://example.test/pr/42","state":"OPEN","statusCheckRollup":[
            {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS"},
            {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"FAILURE"},
            {"__typename":"CheckRun","name":"test","status":"IN_PROGRESS","conclusion":null},
            {"__typename":"StatusContext","context":"ci/merge","state":"SUCCESS"},
            {"__typename":"StatusContext","context":"ci/expect","state":"EXPECTED"}
        ]}"#;
        let pr = parse_pr_status(raw).expect("fixture parses");
        assert_eq!(pr.number, 42);
        assert_eq!(pr.url, "https://example.test/pr/42");
        assert_eq!(pr.state, PrState::Open);
        assert_eq!((pr.checks.pass, pr.checks.fail, pr.checks.pending), (2, 1, 2));
        assert_eq!(pr.checks.failures, ["lint"], "the failing check is named");
    }

    /// The chip's verdict: fail beats pending beats pass; failing names
    /// come from `name` (CheckRun) or `context` (StatusContext).
    #[test]
    fn pr_checks_verdict_and_failure_names() {
        let raw = r#"{"number":1,"url":"https://u","state":"OPEN","statusCheckRollup":[
            {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"FAILURE"},
            {"__typename":"StatusContext","context":"ci/merge","state":"FAILURE"},
            {"__typename":"CheckRun","name":"test","status":"IN_PROGRESS","conclusion":null}
        ]}"#;
        let checks = parse_pr_status(raw).unwrap().checks;
        assert_eq!(checks.verdict(), Some(CheckVerdict::Fail), "a failure beats pending");
        assert_eq!(checks.verdict_count(), 2);
        assert_eq!(checks.failures, ["build", "ci/merge"]);
        assert_eq!(checks.detail(), "Failed: build, ci/merge\n2 failed · 1 pending");

        let pending = r#"{"number":1,"url":"https://u","state":"OPEN","statusCheckRollup":[
            {"status":"COMPLETED","conclusion":"SUCCESS"},{"status":"QUEUED","conclusion":null}]}"#;
        let checks = parse_pr_status(pending).unwrap().checks;
        assert_eq!(checks.verdict(), Some(CheckVerdict::Pending));
        assert_eq!(checks.detail(), "1 pending · 1 passed");

        let green = r#"{"number":1,"url":"https://u","state":"OPEN","statusCheckRollup":[
            {"status":"COMPLETED","conclusion":"SUCCESS"}]}"#;
        assert_eq!(parse_pr_status(green).unwrap().checks.verdict(), Some(CheckVerdict::Pass));

        let bare = r#"{"number":1,"url":"https://u","state":"OPEN"}"#;
        assert_eq!(parse_pr_status(bare).unwrap().checks.verdict(), None, "no checks → no chip");
    }

    /// The stubbed `gh` path carries failure names through — a mixed
    /// rollup lands as a Fail verdict with the check's name.
    #[test]
    fn pr_status_rolls_up_failing_checks() {
        let dir = scratch("mixed");
        let body = format!(
            "echo '{}'",
            r#"{"number":9,"url":"https://example.test/pr/9","state":"OPEN","statusCheckRollup":[{"name":"build","status":"COMPLETED","conclusion":"FAILURE"},{"name":"test","status":"IN_PROGRESS","conclusion":null}]}"#
        );
        let path = fake_gh_path(&dir, &body);
        let pr = git::pr_status(&dir, &[("PATH", path.as_str())]).expect("stub answered");
        assert_eq!(pr.checks.verdict(), Some(CheckVerdict::Fail));
        assert_eq!(pr.checks.failures, ["build"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pr_status_maps_states_and_accepts_nodes_shape() {
        for (raw, state) in [("MERGED", PrState::Merged), ("CLOSED", PrState::Closed), ("open", PrState::Open)] {
            let json = format!(r#"{{"number":1,"url":"https://u","state":"{raw}","statusCheckRollup":[]}}"#);
            assert_eq!(parse_pr_status(&json).unwrap().state, state, "{raw}");
        }
        // Older `gh` wraps the rollup in `{nodes: […]}` — same counts.
        let nodes = r#"{"number":1,"url":"https://u","state":"OPEN","statusCheckRollup":{"nodes":[
            {"status":"COMPLETED","conclusion":"SUCCESS"},{"status":"QUEUED","conclusion":null}]}}"#;
        let pr = parse_pr_status(nodes).unwrap();
        assert_eq!((pr.checks.pass, pr.checks.fail, pr.checks.pending), (1, 0, 1));
    }

    #[test]
    fn pr_status_tolerates_missing_rollup_and_rejects_missing_url() {
        let bare = r#"{"number":3,"url":"https://u","state":"OPEN"}"#;
        let pr = parse_pr_status(bare).expect("rollup is optional");
        assert_eq!((pr.checks.pass, pr.checks.fail, pr.checks.pending), (0, 0, 0));
        assert!(parse_pr_status(r#"{"number":3,"state":"OPEN"}"#).is_none(), "no URL → no row");
        assert!(parse_pr_status("not json").is_none(), "garbage → None");
    }

    /// A stub `gh` answering `pr view` with a fixture — `pr_status` returns
    /// the parsed row and `gh` saw the json flag.
    #[test]
    fn pr_status_reads_the_stubbed_gh() {
        let dir = scratch("ok");
        let log = dir.join("gh-args.log");
        let body = format!(
            "echo \"$@\" >> {}\necho '{}'",
            log.display(),
            r#"{"number":7,"url":"https://example.test/pr/7","state":"OPEN","statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"}]}"#
        );
        let path = fake_gh_path(&dir, &body);
        let pr = git::pr_status(&dir, &[("PATH", path.as_str())]).expect("stub answered");
        assert_eq!((pr.number, pr.state, pr.checks.pass), (7, PrState::Open, 1));
        let argv = std::fs::read_to_string(&log).unwrap_or_default();
        assert_eq!(argv.trim(), "pr view --json number,url,state,statusCheckRollup", "gh got the view args");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `gh` exiting non-zero — no PR for the branch, not logged in — hides
    /// the row rather than surfacing an error.
    #[test]
    fn pr_status_is_none_when_gh_fails() {
        let dir = scratch("nopr");
        let path = fake_gh_path(&dir, "echo 'no pull requests found' >&2\nexit 1");
        assert!(git::pr_status(&dir, &[("PATH", path.as_str())]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Without `gh` on PATH the row hides — PATH holds only an empty dir.
    #[test]
    fn pr_status_is_none_without_gh() {
        let dir = scratch("nogh");
        let empty = dir.join("empty-bin");
        std::fs::create_dir_all(&empty).unwrap();
        let path = format!("{}:/usr/bin:/bin", empty.display());
        assert!(git::pr_status(&dir, &[("PATH", path.as_str())]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
