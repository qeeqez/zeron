//! Sidebar working-status for live subagents (fake-claude.sh drives the real
//! ClaudeHarness stream-json pipeline; fixtures run offline).
//!
//! The eager-done policy parks the parent turn on `result` while a spawned
//! background subagent keeps running — and its tagged traffic keeps landing
//! on the parked session. The sidebar must read Working until the LAST
//! live subagent's tagged Done (a `task_notification` on the wire) lands,
//! not flip Idle with the parent's own turn.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use zeron_doc::{MessagePart, MessageStatus, SessionMessageEntry, SubagentStatus};
use zeron_engine::{EngineCore, EngineProfile, HarnessRegistry};
use zeron_harness::ClaudeHarness;
use zeron_proto::{HarnessId, RunRequest, SandboxLevel, SessionStatus};

const CHAT: &str = "claude-sub-status";

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../harness/tests/fixtures/fake-claude.sh")
}

fn assemble(dir: &Path) -> EngineCore {
    let registry = Arc::new(HarnessRegistry::new());
    registry.register(Arc::new(
        ClaudeHarness::new().with_executable(fixture_path()),
    ));
    let profile = EngineProfile::development(dir, "test-org", "test-user");
    EngineCore::assemble_with_profile(profile, registry, HarnessId::ClaudeCode, None)
        .expect("engine core assembles")
}

fn run_request(dir: &Path, scenario: &str) -> RunRequest {
    RunRequest {
        prompt: format!("scenario:{scenario}"),
        harness: None,
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: dir.display().to_string(),
        sandbox: SandboxLevel::ReadOnly,
        auto_approve: true,
        attachments: vec![],
        worktree: None,
        resume: None,
    }
}

fn status(core: &EngineCore) -> Option<SessionStatus> {
    core.sessions.session_status(CHAT).map(|s| s.status)
}

fn entries(core: &EngineCore, doc: &str) -> Vec<SessionMessageEntry> {
    core.doc_host
        .open(doc)
        .ok()
        .and_then(|h| h.doc().read_entries().ok())
        .unwrap_or_default()
}

/// The spawn chip's settled subagent status, by parent tool-use id.
fn chip_status(core: &EngineCore, spawn_id: &str) -> Option<SubagentStatus> {
    entries(core, CHAT)
        .iter()
        .flat_map(|e| &e.parts)
        .find_map(|p| match p {
            MessagePart::Tool {
                id,
                subagent_status,
                ..
            } if id == spawn_id => *subagent_status,
            _ => None,
        })
}

async fn wait_for<F>(predicate: F, what: &str)
where
    F: FnMut() -> bool + Send,
{
    tokio::time::timeout(Duration::from_secs(10), async move {
        let mut predicate = predicate;
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// The parent's `result` parks the turn, but the spawned subagent is still
/// running: the session must read Working (turn completed, subagent live)
/// and only settle to Idle when the child's tagged Done lands.
#[tokio::test(flavor = "multi_thread")]
async fn parked_session_reads_working_while_a_subagent_runs() {
    let dir = tempfile::tempdir().unwrap();
    let core = assemble(dir.path());
    core.sessions
        .dispatch(
            CHAT,
            HarnessId::ClaudeCode,
            run_request(dir.path(), "bgwait"),
            Some("user-prompt".into()),
        )
        .await
        .expect("dispatch");

    // Turn completed (the park's completion marker landed) AND the session
    // still reads Working — the bug parked this to Idle while the child ran.
    wait_for(
        || {
            let Some(session) = core.sessions.session_status(CHAT) else {
                return false;
            };
            session.status == SessionStatus::Working && session.last_completed_turn.is_some()
        },
        "parked session with a live subagent to read Working",
    )
    .await;

    // The subagent settles (task_notification → tagged Done): Idle.
    wait_for(
        || status(&core) == Some(SessionStatus::Idle),
        "session to settle Idle once the subagent finishes",
    )
    .await;

    core.shutdown().await;
}

/// Two concurrent background subagents: settling the first must NOT drop the
/// session to Idle — Working holds until the LAST live subagent's Done.
#[tokio::test(flavor = "multi_thread")]
async fn working_holds_until_the_last_subagent_settles() {
    let dir = tempfile::tempdir().unwrap();
    let core = assemble(dir.path());
    core.sessions
        .dispatch(
            CHAT,
            HarnessId::ClaudeCode,
            run_request(dir.path(), "bgwait2"),
            Some("user-prompt".into()),
        )
        .await
        .expect("dispatch");

    wait_for(
        || {
            let Some(session) = core.sessions.session_status(CHAT) else {
                return false;
            };
            session.status == SessionStatus::Working && session.last_completed_turn.is_some()
        },
        "parked session with live subagents to read Working",
    )
    .await;

    // First child settles (its chip stamps Done): the second still runs —
    // the session must not have dropped to Idle with it.
    wait_for(
        || chip_status(&core, "toolu_ba") == Some(SubagentStatus::Done),
        "first subagent's chip to settle",
    )
    .await;
    assert_eq!(
        status(&core),
        Some(SessionStatus::Working),
        "a second live subagent must keep the parked session Working"
    );

    wait_for(
        || status(&core) == Some(SessionStatus::Idle),
        "session to settle Idle once the last subagent finishes",
    )
    .await;
    assert_eq!(chip_status(&core, "toolu_bb"), Some(SubagentStatus::Done));

    core.shutdown().await;
}

/// THREE levels: the child spawned a grandchild inside its OWN tagged
/// transcript (a grandchild gets no task_started on the top stream — the
/// tagged assistant frame alone registers it). The DIRECT child settles
/// first; the session must hold Working on the grandchild alone until its
/// task_notification lands — dropped, that notification wedges Working.
#[tokio::test(flavor = "multi_thread")]
async fn working_holds_while_a_grandchild_runs() {
    let dir = tempfile::tempdir().unwrap();
    let core = assemble(dir.path());
    core.sessions
        .dispatch(
            CHAT,
            HarnessId::ClaudeCode,
            run_request(dir.path(), "bgnested"),
            Some("user-prompt".into()),
        )
        .await
        .expect("dispatch");

    wait_for(
        || {
            let Some(session) = core.sessions.session_status(CHAT) else {
                return false;
            };
            session.status == SessionStatus::Working && session.last_completed_turn.is_some()
        },
        "parked session with a live grandchild to read Working",
    )
    .await;

    // The direct child settles first — but the grandchild still runs.
    wait_for(
        || chip_status(&core, "toolu_pa") == Some(SubagentStatus::Done),
        "child chip to settle",
    )
    .await;
    assert_eq!(
        status(&core),
        Some(SessionStatus::Working),
        "a live grandchild must keep the parked session Working"
    );

    wait_for(
        || status(&core) == Some(SessionStatus::Idle),
        "session to settle Idle once the grandchild finishes",
    )
    .await;

    core.shutdown().await;
}

/// FOUR levels, settling shallowest-first: after the child AND the
/// grandchild are both done, the great-grandchild alone must hold the
/// parked session Working until its own notification lands.
#[tokio::test(flavor = "multi_thread")]
async fn working_holds_through_out_of_order_deep_settles() {
    let dir = tempfile::tempdir().unwrap();
    let core = assemble(dir.path());
    core.sessions
        .dispatch(
            CHAT,
            HarnessId::ClaudeCode,
            run_request(dir.path(), "bgnested2"),
            Some("user-prompt".into()),
        )
        .await
        .expect("dispatch");

    wait_for(
        || {
            let Some(session) = core.sessions.session_status(CHAT) else {
                return false;
            };
            session.status == SessionStatus::Working && session.last_completed_turn.is_some()
        },
        "parked session with live descendants to read Working",
    )
    .await;

    // Child then grandchild settle while the great-grandchild still runs.
    // (gc's doc freezes with a Complete final entry on its tagged Done.)
    wait_for(
        || {
            entries(&core, &format!("{CHAT}--sub--toolu_gc"))
                .last()
                .is_some_and(|e| e.status == Some(MessageStatus::Complete))
        },
        "grandchild doc to freeze",
    )
    .await;
    assert_eq!(chip_status(&core, "toolu_pa"), Some(SubagentStatus::Done));
    assert_eq!(
        status(&core),
        Some(SessionStatus::Working),
        "a live great-grandchild must keep the parked session Working"
    );

    wait_for(
        || status(&core) == Some(SessionStatus::Idle),
        "session to settle Idle once the great-grandchild finishes",
    )
    .await;

    core.shutdown().await;
}
