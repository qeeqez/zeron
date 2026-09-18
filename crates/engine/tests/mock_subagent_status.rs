//! Sidebar working-status on the paths fake-claude.sh can't express:
//! TRUE nested `Subagent{outer, Subagent{leaf, …}}` wrappers (the claude
//! wire never produces them — only a harness emitting the real nested
//! shape exercises the leaf-owner bookkeeping end to end), and the
//! quiesce-watchdog park with a live subagent (a lost turn-end needs a
//! stream that stays open and silent, which a fixture's stdout cannot
//! pace).
//!
//! `ZERON_MOCK_DELAY_MS` paces the scripted run so each transition is
//! observable to the polling asserts; `ZERON_TURN_QUIESCE_MS` shrinks the
//! watchdog window. Both are process-global, set once before any engine
//! assembles (this file is its own test binary).

use std::path::Path;
use std::sync::{Arc, Once};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::{Mutex, mpsc};

use zeron_doc::{MessagePart, SessionMessageEntry, SubagentStatus};
use zeron_engine::{EngineCore, EngineProfile, HarnessRegistry};
use zeron_harness::mock::MockHarness;
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SessionStatus, SteeringMode, ToolCall,
};

const CHAT: &str = "mock-sub-status";
/// Watchdog window for the quiesce park test.
const QUIESCE_MS: u64 = 300;

fn init_env() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: called before any engine (and thus any reader of the
        // vars) exists in this test process; all tests share the values.
        unsafe {
            std::env::set_var("ZERON_MOCK_DELAY_MS", "60");
            std::env::set_var("ZERON_TURN_QUIESCE_MS", QUIESCE_MS.to_string());
        }
    });
}

fn run_request(prompt: &str, dir: &Path) -> RunRequest {
    RunRequest {
        prompt: prompt.into(),
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

fn session_started() -> AgentEvent {
    AgentEvent::SessionStarted {
        harness: HarnessId::Mock,
        model: "mock-1".into(),
        tools: vec![],
        cwd: "/tmp".into(),
        session_id: "hs-sub".into(),
        assistant_message_id: "a-sub".into(),
    }
}

fn done(status: DoneStatus) -> AgentEvent {
    AgentEvent::Done {
        status,
        result: None,
        error: None,
        session_id: Some("hs-sub".into()),
    }
}

fn text(t: &str) -> AgentEvent {
    AgentEvent::TextDelta { text: t.into() }
}

/// An `Agent`-genus spawn call — the shape every driver's Task/Agent
/// decode produces, and the only call kind that mints subagent liveness.
fn spawn(id: &str, description: &str) -> AgentEvent {
    AgentEvent::ToolCall {
        id: id.into(),
        call: ToolCall::Unknown {
            name: format!("Agent: {description}"),
            input: Some(serde_json::json!({
                "description": description,
                "prompt": "probe",
            })),
        },
    }
}

/// Wrap an event as subagent-attributed traffic. Applied twice it builds
/// the TRUE nested wrapper (`Subagent{pa, Subagent{gc, ev}}`) no
/// flat-tagging adapter can emit.
fn tag(parent: &str, event: AgentEvent) -> AgentEvent {
    AgentEvent::Subagent {
        parent_tool_use_id: parent.into(),
        event: Box::new(event),
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

/// The parked-session predicate every test here shares: the turn's
/// completion marker landed AND the status reads Working — i.e. a live
/// subagent held the park rather than letting it settle Idle.
fn parked_working(core: &EngineCore) -> bool {
    let Some(session) = core.sessions.session_status(CHAT) else {
        return false;
    };
    session.status == SessionStatus::Working && session.last_completed_turn.is_some()
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

fn assemble_mock(dir: &Path, script: Vec<AgentEvent>) -> EngineCore {
    let registry = Arc::new(HarnessRegistry::new());
    registry.register(Arc::new(MockHarness { script }));
    let profile = EngineProfile::development(dir, "test-org", "test-user");
    EngineCore::assemble_with_profile(profile, registry, HarnessId::Mock, None)
        .expect("engine core assembles")
}

/// The leaf-owner proof no flat-tagging adapter can give: a grandchild's
/// events arrive DOUBLE-WRAPPED (`Subagent{pa, Subagent{gc, ev}}`). The
/// innermost id owns the liveness — `gc` mints from a nested leaf, holds
/// the parked session Working after `pa` itself settles (the wrapper only
/// relays), and releases it on its own nested Done.
#[tokio::test(flavor = "multi_thread")]
async fn nested_leaf_owner_holds_the_park_until_it_settles() {
    init_env();
    let script = vec![
        session_started(),
        spawn("pa", "child probe"),
        text("launched the probes"),
        // Parent turn ends; `pa` is live — the park must read Working.
        done(DoneStatus::Completed),
        tag("pa", text("child working")),
        // The grandchild's first event arrives nested inside pa's wrapper:
        // the leaf owner `gc` mints live here.
        tag("pa", tag("gc", text("gc working"))),
        // The DIRECT child settles while the grandchild streams on.
        tag("pa", done(DoneStatus::Completed)),
        // Nested content landing after pa froze still proves gc live.
        tag("pa", tag("gc", text("gc outlives pa"))),
        tag("pa", tag("gc", done(DoneStatus::Completed))),
    ];
    let dir = tempfile::tempdir().unwrap();
    let core = assemble_mock(dir.path(), script);
    core.sessions
        .dispatch(
            CHAT,
            HarnessId::Mock,
            run_request("nested probes", dir.path()),
            Some("user-prompt".into()),
        )
        .await
        .expect("dispatch");

    wait_for(
        || parked_working(&core),
        "parked session with live descendants to read Working",
    )
    .await;

    // `pa` settles (its chip stamps Done) while `gc` still streams — the
    // session must hold Working on the grandchild alone.
    wait_for(
        || chip_status(&core, "pa") == Some(SubagentStatus::Done),
        "child chip to settle",
    )
    .await;
    assert_eq!(
        status(&core),
        Some(SessionStatus::Working),
        "a live grandchild must keep the parked session Working"
    );

    // gc's own nested Done is the last leaf standing: Idle.
    wait_for(
        || status(&core) == Some(SessionStatus::Idle),
        "session to settle Idle once the grandchild finishes",
    )
    .await;

    core.shutdown().await;
}

/// Feed-by-hand harness (turn_quiesce.rs's shape): the test pushes events
/// through a channel, the only way to hold a stream OPEN and silent for
/// the watchdog and then resume it. Non-main prompts (the auto-titler)
/// get an instantly-completed empty stream. No `deterministic_turn_end`
/// override: the watchdog stays armed for prompted turns.
struct FeedHarness {
    main_prompt: String,
    feed: Mutex<Option<mpsc::UnboundedReceiver<AgentEvent>>>,
}

#[async_trait]
impl Harness for FeedHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Feed"
    }
    fn supports_steering(&self) -> bool {
        true
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::StepBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        if request.prompt != self.main_prompt {
            let events = vec![Ok(done(DoneStatus::Completed))];
            return Ok(futures::stream::iter(events).boxed());
        }
        let feed = self
            .feed
            .lock()
            .await
            .take()
            .expect("FeedHarness serves the main dispatch once per test");
        Ok(futures::stream::unfold(feed, |mut feed| async move {
            feed.recv().await.map(|event| (Ok(event), feed))
        })
        .boxed())
    }
}

/// The OTHER park path: when a turn's Done is lost upstream, the quiesce
/// watchdog parks on silence — and a live subagent must hold THAT park at
/// Working exactly like the eager-Done one, then flip Idle on its settle.
#[tokio::test(flavor = "multi_thread")]
async fn quiesce_park_holds_working_with_a_live_subagent() {
    init_env();
    let (feed, rx) = mpsc::unbounded_channel();
    let registry = Arc::new(HarnessRegistry::new());
    registry.register(Arc::new(FeedHarness {
        main_prompt: "quiesce probe".into(),
        feed: Mutex::new(Some(rx)),
    }));
    let dir = tempfile::tempdir().unwrap();
    let profile = EngineProfile::development(dir.path(), "test-org", "test-user");
    let core = EngineCore::assemble_with_profile(profile, registry, HarnessId::Mock, None)
        .expect("engine core assembles");
    core.sessions
        .dispatch(
            CHAT,
            HarnessId::Mock,
            run_request("quiesce probe", dir.path()),
            Some("user-prompt".into()),
        )
        .await
        .expect("dispatch");

    feed.send(session_started()).unwrap();
    feed.send(text("Kicking off a background probe.")).unwrap();
    feed.send(spawn("pa", "quiesce probe")).unwrap();
    // The launch ack resolves the chip in the fold (the watchdog's
    // in-flight gate opens) but does NOT settle the mint — only an ERROR
    // result does.
    feed.send(AgentEvent::ToolResult {
        id: "pa".into(),
        is_error: false,
        output: None,
        diff: None,
    })
    .unwrap();
    // …then the turn's Done is lost upstream: silence. The watchdog parks
    // the turn — the live subagent must hold the park at Working.
    wait_for(
        || parked_working(&core),
        "quiesce-parked session with a live subagent to read Working",
    )
    .await;

    // The child's tagged Done lands on the parked session: last settle →
    // Idle, no un-park.
    feed.send(tag("pa", done(DoneStatus::Completed))).unwrap();
    wait_for(
        || status(&core) == Some(SessionStatus::Idle),
        "session to settle Idle once the subagent finishes",
    )
    .await;

    drop(feed);
    core.shutdown().await;
}
