use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use schemars::JsonSchema;
use serde::Serialize;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::{
    activity::{
        ActivityPhase, ActivityReporter, ActivitySnapshot, AgentActivity, AgentActivityEvent,
        millis,
    },
    child_mcp::ChildMcpManager,
    definition::{
        AgentDefinition, MAX_BUILT_CONTEXT_BYTES, MAX_SEND_MESSAGE_BYTES, MAX_SPAWN_TASK_BYTES,
        MAX_WAIT_TARGETS,
    },
    discovery::AgentRegistry,
    provider::{ConversationState, ProviderClient, ProviderCredential, ProviderRun},
    timeouts::{MAX_WAIT_AGENT_TIMEOUT_MS, RUNTIME_SHUTDOWN_TIMEOUT},
};
use crate::skills::SkillRegistry;

const QUEUE_LIMIT: usize = 16;
pub(crate) const MAX_RETAINED_TERMINAL_SESSIONS: usize = 64;

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeError {
    pub(crate) kind: String,
    pub(crate) message: String,
}
impl RuntimeError {
    pub(crate) fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentState {
    Running,
    Completed,
    Failed,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentResult {
    #[serde(rename = "agentId")]
    pub(crate) id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(rename = "status")]
    pub(crate) state: AgentState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<RuntimeError>,
    pub(crate) total_elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) activity: Option<ActivitySnapshot>,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpawnResult {
    #[serde(rename = "agentId")]
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(rename = "status")]
    pub(crate) state: AgentState,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InputAck {
    #[serde(rename = "agentId")]
    pub(crate) id: String,
    pub(crate) accepted: bool,
    #[serde(rename = "status")]
    pub(crate) state: AgentState,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WaitResult {
    pub(crate) agents: Vec<AgentResult>,
    pub(crate) timed_out: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct WaitObservation {
    pub(crate) result: WaitResult,
    pub(crate) wait_timeout_remaining_ms: u64,
}

struct SessionData {
    conversation: Option<ConversationState>,
    queue: VecDeque<String>,
    cancel: Option<CancellationToken>,
    interrupt_pending: bool,
    accepting_input: bool,
    state: AgentState,
    result: Option<String>,
    error: Option<RuntimeError>,
    resumable: bool,
    revision: u64,
    activity: Option<AgentActivity>,
    terminal_at: Option<Instant>,
    last_accessed_at: Instant,
}
struct Session {
    definition: Arc<AgentDefinition>,
    context: String,
    created_at: Instant,
    data: Mutex<SessionData>,
}
struct Inner {
    workspace: PathBuf,
    registry: Arc<AgentRegistry>,
    skills: Arc<SkillRegistry>,
    provider: ProviderClient,
    sessions: Mutex<BTreeMap<String, Arc<Session>>>,
    capacity: Arc<Semaphore>,
    version: watch::Sender<u64>,
}
#[derive(Clone)]
pub(crate) struct AgentRuntime {
    inner: Arc<Inner>,
}

impl AgentRuntime {
    pub(crate) fn new(workspace: PathBuf) -> Result<Self, RuntimeError> {
        let registry = AgentRegistry::discover(&workspace).map_err(|_| {
            RuntimeError::new(
                "configuration_error",
                "unable to discover agent definitions",
            )
        })?;
        let skills = SkillRegistry::discover(&workspace)
            .map_err(|_| RuntimeError::new("configuration_error", "unable to discover skills"))?;
        let provider = ProviderClient::new().map_err(|_| {
            RuntimeError::new(
                "configuration_error",
                "unable to initialize provider client",
            )
        })?;
        let workspace = registry.workspace().to_path_buf();
        let (version, _) = watch::channel(0u64);
        Ok(Self {
            inner: Arc::new(Inner {
                workspace,
                registry: Arc::new(registry),
                skills: Arc::new(skills),
                provider,
                sessions: Mutex::new(BTreeMap::new()),
                capacity: Arc::new(Semaphore::new(8)),
                version,
            }),
        })
    }
    pub(crate) fn registry(&self) -> &AgentRegistry {
        &self.inner.registry
    }
    pub(crate) async fn spawn(&self, name: &str, task: &str) -> Result<SpawnResult, RuntimeError> {
        if task.trim().is_empty() {
            return Err(RuntimeError::new(
                "invalid_request",
                "task must not be empty",
            ));
        }
        if task.len() > MAX_SPAWN_TASK_BYTES {
            return Err(RuntimeError::new(
                "invalid_request",
                format!("task exceeds the {MAX_SPAWN_TASK_BYTES}-byte limit"),
            ));
        }
        let definition = self
            .inner
            .registry
            .get(name)
            .ok_or_else(|| RuntimeError::new("unknown_agent", "unknown agent"))?;
        let credential = ProviderCredential::resolve(&definition)
            .map_err(|e| RuntimeError::new(e.kind, e.message))?;
        let context = self.context(&definition)?;
        cleanup_terminal_sessions(&self.inner).await;
        let permit = self
            .inner
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError::new("capacity_exceeded", "agent runtime is at capacity"))?;
        let id = format!("agt_{}", uuid::Uuid::now_v7().simple());
        let now = Instant::now();
        let session = Arc::new(Session {
            definition: definition.clone(),
            context,
            data: Mutex::new(SessionData {
                conversation: Some(ConversationState::new(&definition.wire_api)),
                queue: VecDeque::new(),
                cancel: None,
                interrupt_pending: false,
                accepting_input: true,
                state: AgentState::Running,
                result: None,
                error: None,
                resumable: true,
                revision: 1,
                activity: Some(AgentActivity::new(AgentActivityEvent::new(
                    ActivityPhase::Starting,
                    "Starting agent",
                ))),
                terminal_at: None,
                last_accessed_at: now,
            }),
            created_at: now,
        });
        self.inner
            .sessions
            .lock()
            .await
            .insert(id.clone(), session.clone());
        self.inner.version.send_modify(|v| *v = v.wrapping_add(1));
        tracing::info!(agent_id = %id, agent = %definition.name, event = "spawned", "agent activity");
        self.launch(id.clone(), session, task.to_owned(), credential, permit);
        Ok(SpawnResult {
            id,
            name: definition.name.clone(),
            state: AgentState::Running,
        })
    }
    fn context(&self, definition: &AgentDefinition) -> Result<String, RuntimeError> {
        let mut out = format!(
            "{}\n\nWorkspace: {}",
            definition.instructions,
            self.inner.workspace.display()
        );
        for name in &definition.skills {
            let skill = self.inner.skills.load(name).map_err(|_| {
                RuntimeError::new("configuration_error", "configured skill is unavailable")
            })?;
            out.push_str("\n\n");
            out.push_str(&skill.instructions);
            if out.len() > MAX_BUILT_CONTEXT_BYTES {
                return Err(RuntimeError::new(
                    "configuration_error",
                    format!("agent context exceeds the {MAX_BUILT_CONTEXT_BYTES}-byte limit"),
                ));
            }
        }
        if out.len() > MAX_BUILT_CONTEXT_BYTES {
            return Err(RuntimeError::new(
                "configuration_error",
                format!("agent context exceeds the {MAX_BUILT_CONTEXT_BYTES}-byte limit"),
            ));
        }
        Ok(out)
    }
    fn launch(
        &self,
        id: String,
        session: Arc<Session>,
        message: String,
        credential: ProviderCredential,
        permit: OwnedSemaphorePermit,
    ) {
        let inner = self.inner.clone();
        tokio::spawn(
            async move { run_worker(inner, id, session, message, credential, permit).await },
        );
    }
    pub(crate) async fn send_input(
        &self,
        target: &str,
        message: &str,
        interrupt: bool,
    ) -> Result<InputAck, RuntimeError> {
        if message.trim().is_empty() {
            return Err(RuntimeError::new(
                "invalid_request",
                "message must not be empty",
            ));
        }
        if message.len() > MAX_SEND_MESSAGE_BYTES {
            return Err(RuntimeError::new(
                "invalid_request",
                format!("message exceeds the {MAX_SEND_MESSAGE_BYTES}-byte limit"),
            ));
        }
        let session = self.session(target).await?;
        {
            let mut data = session.data.lock().await;
            data.last_accessed_at = Instant::now();
            if data.state == AgentState::Running {
                queue_input(&mut data, message, interrupt)?;
                return Ok(InputAck {
                    id: target.into(),
                    accepted: true,
                    state: AgentState::Running,
                });
            }
            ensure_resumable(&data)?;
        }
        let credential = ProviderCredential::resolve(&session.definition)
            .map_err(|e| RuntimeError::new(e.kind, e.message))?;
        let permit = self
            .inner
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError::new("capacity_exceeded", "agent runtime is at capacity"))?;
        let sessions = self.inner.sessions.lock().await;
        if sessions
            .get(target)
            .is_none_or(|retained| !Arc::ptr_eq(retained, &session))
        {
            return Err(RuntimeError::new("unknown_agent", "unknown agent session"));
        }
        let mut data = session.data.lock().await;
        data.last_accessed_at = Instant::now();
        if data.state == AgentState::Running {
            queue_input(&mut data, message, interrupt)?;
            return Ok(InputAck {
                id: target.into(),
                accepted: true,
                state: AgentState::Running,
            });
        }
        ensure_resumable(&data)?;
        let launch_message = if let Some(pending) = data.queue.pop_front() {
            data.queue.push_back(message.to_owned());
            pending
        } else {
            message.to_owned()
        };
        data.state = AgentState::Running;
        data.accepting_input = true;
        data.result = None;
        data.error = None;
        data.terminal_at = None;
        data.activity = Some(AgentActivity::new(AgentActivityEvent::new(
            ActivityPhase::Starting,
            "Starting agent",
        )));
        data.revision = data.revision.wrapping_add(1);
        drop(data);
        drop(sessions);
        self.inner.version.send_modify(|v| *v = v.wrapping_add(1));
        self.launch(target.into(), session, launch_message, credential, permit);
        cleanup_terminal_sessions(&self.inner).await;
        Ok(InputAck {
            id: target.into(),
            accepted: true,
            state: AgentState::Running,
        })
    }
    async fn session(&self, id: &str) -> Result<Arc<Session>, RuntimeError> {
        self.inner
            .sessions
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeError::new("unknown_agent", "unknown agent session"))
    }
    pub(crate) async fn wait(
        &self,
        targets: &[String],
        timeout_ms: u64,
    ) -> Result<WaitResult, RuntimeError> {
        self.wait_inner(targets, timeout_ms, None).await
    }
    pub(crate) async fn wait_observing(
        &self,
        targets: &[String],
        timeout_ms: u64,
        updates: mpsc::Sender<WaitObservation>,
    ) -> Result<WaitResult, RuntimeError> {
        self.wait_inner(targets, timeout_ms, Some(updates)).await
    }
    async fn wait_inner(
        &self,
        targets: &[String],
        timeout_ms: u64,
        updates: Option<mpsc::Sender<WaitObservation>>,
    ) -> Result<WaitResult, RuntimeError> {
        if targets.is_empty()
            || targets.len() > MAX_WAIT_TARGETS
            || timeout_ms > MAX_WAIT_AGENT_TIMEOUT_MS
        {
            return Err(RuntimeError::new(
                "invalid_request",
                format!(
                    "targets must contain 1 to {MAX_WAIT_TARGETS} entries and timeoutMs must be at most 300000"
                ),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        if !targets.iter().all(|x| seen.insert(x)) {
            return Err(RuntimeError::new(
                "invalid_request",
                "targets must be unique",
            ));
        }
        let mut rx = self.inner.version.subscribe();
        let (initial, mut revisions) = self.snapshot_with_revisions(targets).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        send_observation(&updates, initial.clone(), deadline);
        if initial
            .agents
            .iter()
            .all(|a| a.state != AgentState::Running)
        {
            return Ok(WaitResult {
                agents: initial.agents,
                timed_out: false,
            });
        }
        if timeout_ms == 0 {
            return Ok(WaitResult {
                agents: initial.agents,
                timed_out: true,
            });
        }
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => { let (s, _) = self.snapshot_with_revisions(targets).await?; return Ok(WaitResult { timed_out: s.agents.iter().any(|a| a.state == AgentState::Running), agents: s.agents }); }
                changed = rx.changed() => {
                    let (s, next_revisions) = self.snapshot_with_revisions(targets).await?;
                    if changed.is_err() { return Ok(WaitResult { timed_out: s.agents.iter().any(|a| a.state == AgentState::Running), agents: s.agents }); }
                    let changed_agents = WaitResult { agents: s.agents.iter().zip(&next_revisions).zip(&revisions).filter(|((_, next), previous)| next != previous).map(|((agent, _), _)| agent.clone()).collect(), timed_out: false };
                    revisions = next_revisions;
                    if !changed_agents.agents.is_empty() { send_observation(&updates, changed_agents, deadline); }
                    if s.agents.iter().all(|a| a.state != AgentState::Running) { return Ok(WaitResult { agents: s.agents, timed_out: false }); }
                }
            }
        }
    }
    async fn snapshot_with_revisions(
        &self,
        targets: &[String],
    ) -> Result<(WaitResult, Vec<u64>), RuntimeError> {
        let sessions = self.inner.sessions.lock().await;
        let mut agents = Vec::with_capacity(targets.len());
        let mut revisions = Vec::with_capacity(targets.len());
        for id in targets {
            let session = sessions
                .get(id)
                .ok_or_else(|| RuntimeError::new("unknown_agent", "unknown agent session"))?;
            let mut d = session.data.lock().await;
            d.last_accessed_at = Instant::now();
            revisions.push(d.revision);
            let now = d.terminal_at.unwrap_or_else(Instant::now);
            agents.push(AgentResult {
                id: id.clone(),
                name: Some(session.definition.name.clone()),
                state: d.state.clone(),
                result: d.result.clone(),
                error: d.error.clone(),
                total_elapsed_ms: millis(now.saturating_duration_since(session.created_at)),
                activity: (d.state == AgentState::Running)
                    .then(|| d.activity.as_ref().map(|a| a.snapshot(now)))
                    .flatten(),
            });
        }
        Ok((
            WaitResult {
                agents,
                timed_out: false,
            },
            revisions,
        ))
    }
    pub(crate) async fn shutdown(&self) {
        let sessions: Vec<_> = self.inner.sessions.lock().await.values().cloned().collect();
        for s in &sessions {
            if let Some(c) = &s.data.lock().await.cancel {
                c.cancel();
            }
        }
        let deadline = tokio::time::Instant::now() + RUNTIME_SHUTDOWN_TIMEOUT;
        let mut updates = self.inner.version.subscribe();
        loop {
            let running = {
                let mut running = false;
                for session in &sessions {
                    if session.data.lock().await.state == AgentState::Running {
                        running = true;
                        break;
                    }
                }
                running
            };
            if !running || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::select! { _ = tokio::time::sleep_until(deadline) => break, changed = updates.changed() => { if changed.is_err() { break; } } }
        }
    }
}

fn send_observation(
    updates: &Option<mpsc::Sender<WaitObservation>>,
    result: WaitResult,
    deadline: tokio::time::Instant,
) {
    if let Some(updates) = updates {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let _ = updates.try_send(WaitObservation {
            result,
            wait_timeout_remaining_ms: millis(remaining),
        });
    }
}

fn queue_input(data: &mut SessionData, message: &str, interrupt: bool) -> Result<(), RuntimeError> {
    if !data.accepting_input {
        return Err(RuntimeError::new(
            "not_accepting_input",
            "agent run is finishing and cannot accept more input",
        ));
    }
    if data.queue.len() == QUEUE_LIMIT {
        return Err(RuntimeError::new("queue_full", "agent input queue is full"));
    }
    data.queue.push_back(message.to_owned());
    if interrupt {
        if let Some(cancel) = &data.cancel {
            cancel.cancel();
        } else {
            data.interrupt_pending = true;
        }
    }
    Ok(())
}

fn pop_input_for_execution(data: &mut SessionData) -> Option<String> {
    let next = data.queue.pop_front();
    if next.is_some() {
        data.interrupt_pending = false;
    }
    next
}

fn install_turn_cancel(
    data: &mut SessionData,
    startup_cancel: Option<&CancellationToken>,
) -> CancellationToken {
    let turn_cancel = CancellationToken::new();
    if startup_cancel.is_some_and(CancellationToken::is_cancelled)
        || std::mem::take(&mut data.interrupt_pending)
    {
        turn_cancel.cancel();
    }
    data.cancel = Some(turn_cancel.clone());
    turn_cancel
}

fn ensure_resumable(data: &SessionData) -> Result<(), RuntimeError> {
    if data.state == AgentState::Failed && !data.resumable {
        Err(RuntimeError::new(
            "non_resumable",
            "agent session cannot be resumed",
        ))
    } else {
        Ok(())
    }
}

fn set_terminal(data: &mut SessionData, outcome: Result<String, super::provider::ProviderError>) {
    let now = Instant::now();
    data.cancel = None;
    data.terminal_at = Some(now);
    data.last_accessed_at = now;
    data.revision = data.revision.wrapping_add(1);
    match outcome {
        Ok(result) => {
            data.state = AgentState::Completed;
            data.result = Some(result);
            data.error = None;
            data.resumable = true;
            data.accepting_input = true;
            data.activity = Some(AgentActivity::new(AgentActivityEvent::new(
                ActivityPhase::Completed,
                "Completed",
            )));
        }
        Err(error) => {
            data.state = AgentState::Failed;
            data.result = None;
            data.error = Some(RuntimeError::new(error.kind, error.message));
            data.resumable = error.resumable;
            data.accepting_input = error.resumable;
            data.activity = Some(AgentActivity::new(AgentActivityEvent::new(
                ActivityPhase::Failed,
                "Failed",
            )));
        }
    }
}

async fn finish_failed(session: &Session, error: RuntimeError, resumable: bool) {
    let mut data = session.data.lock().await;
    let now = Instant::now();
    data.cancel = None;
    data.state = AgentState::Failed;
    data.result = None;
    data.error = Some(error);
    data.resumable = resumable;
    data.accepting_input = resumable;
    data.activity = Some(AgentActivity::new(AgentActivityEvent::new(
        ActivityPhase::Failed,
        "Failed",
    )));
    data.terminal_at = Some(now);
    data.last_accessed_at = now;
    data.revision = data.revision.wrapping_add(1);
}

async fn cleanup_terminal_sessions(inner: &Arc<Inner>) {
    let entries: Vec<_> = inner
        .sessions
        .lock()
        .await
        .iter()
        .map(|(id, session)| (id.clone(), session.clone()))
        .collect();
    let mut terminal = Vec::new();
    for (id, session) in entries {
        let data = session.data.lock().await;
        if data.state != AgentState::Running {
            terminal.push((data.last_accessed_at, id, session.clone()));
        }
    }
    if terminal.len() <= MAX_RETAINED_TERMINAL_SESSIONS {
        return;
    }
    terminal.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    let remove = terminal.len() - MAX_RETAINED_TERMINAL_SESSIONS;
    for (accessed, id, candidate) in terminal.into_iter().take(remove) {
        let mut sessions = inner.sessions.lock().await;
        let Some(retained) = sessions.get(&id).cloned() else {
            continue;
        };
        if !Arc::ptr_eq(&retained, &candidate) {
            continue;
        }
        let data = retained.data.lock().await;
        let evict = data.state != AgentState::Running && data.last_accessed_at == accessed;
        drop(data);
        if evict {
            sessions.remove(&id);
        }
    }
}

async fn run_worker(
    inner: Arc<Inner>,
    id: String,
    session: Arc<Session>,
    mut message: String,
    credential: ProviderCredential,
    permit: OwnedSemaphorePermit,
) {
    let reporter = {
        let inner = inner.clone();
        let session = session.clone();
        let id = id.clone();
        ActivityReporter::new(move |event| {
            let inner = inner.clone();
            let session = session.clone();
            let id = id.clone();
            Box::pin(async move {
                let mut data = session.data.lock().await;
                let prior = data
                    .activity
                    .as_ref()
                    .map(|activity| millis(activity.started_at.elapsed()));
                let prior_phase = data
                    .activity
                    .as_ref()
                    .map(|activity| activity.phase.clone());
                let prior_tool = data
                    .activity
                    .as_ref()
                    .and_then(|activity| activity.tool.clone());
                let prior_deadline_ms = data
                    .activity
                    .as_ref()
                    .and_then(|activity| activity.deadline)
                    .map(|deadline| millis(deadline.saturating_duration_since(Instant::now())));
                data.activity = Some(AgentActivity::new(event.clone()));
                data.revision = data.revision.wrapping_add(1);
                drop(data);
                inner.version.send_modify(|v| *v = v.wrapping_add(1));
                match event.kind {
                    "model_started" => {
                        tracing::info!(agent_id = %id, event = "model_started", "agent activity")
                    }
                    "tool_started" => {
                        tracing::info!(agent_id = %id, event = "tool_started", tool = ?event.tool, target = ?event.target, "agent activity")
                    }
                    "tool_completed" => {
                        tracing::info!(agent_id = %id, event = "tool_completed", tool = ?prior_tool, prior_activity_ms = ?prior, prior_deadline_ms = ?prior_deadline_ms, prior_phase = ?prior_phase, "agent activity")
                    }
                    "tool_failed" => {
                        tracing::info!(agent_id = %id, event = "tool_failed", tool = ?prior_tool, prior_activity_ms = ?prior, "agent activity")
                    }
                    "tool_timed_out" => {
                        tracing::info!(agent_id = %id, event = "tool_timed_out", tool = ?prior_tool, prior_activity_ms = ?prior, "agent activity")
                    }
                    _ => {}
                }
            })
        })
    };
    'run: loop {
        let cancel = {
            let mut d = session.data.lock().await;
            let cancel = CancellationToken::new();
            if std::mem::take(&mut d.interrupt_pending) {
                cancel.cancel();
            }
            d.cancel = Some(cancel.clone());
            cancel
        };
        reporter
            .report(AgentActivityEvent {
                phase: ActivityPhase::Starting,
                summary: "Starting child MCP servers".into(),
                target: None,
                tool: None,
                deadline: None,
                kind: "child_mcp_starting",
            })
            .await;
        let mut child =
            match ChildMcpManager::connect(&session.definition, &inner.workspace, &cancel).await {
                Ok(child) => child,
                Err(_) => {
                    if cancel.is_cancelled() {
                        let mut data = session.data.lock().await;
                        if let Some(next) = pop_input_for_execution(&mut data) {
                            data.cancel = None;
                            data.activity = Some(AgentActivity::new(AgentActivityEvent::new(
                                ActivityPhase::Starting,
                                "Starting agent",
                            )));
                            data.revision = data.revision.wrapping_add(1);
                            drop(data);
                            inner.version.send_modify(|v| *v = v.wrapping_add(1));
                            message = next;
                            continue 'run;
                        }
                    }
                    finish_failed(
                        &session,
                        RuntimeError::new(
                            "child_mcp_startup_error",
                            "unable to start configured child MCP servers",
                        ),
                        true,
                    )
                    .await;
                    inner.version.send_modify(|v| *v = v.wrapping_add(1));
                    break 'run;
                }
            };
        if cancel.is_cancelled() {
            let mut data = session.data.lock().await;
            if let Some(next) = pop_input_for_execution(&mut data) {
                data.cancel = None;
                data.activity = Some(AgentActivity::new(AgentActivityEvent::new(
                    ActivityPhase::Starting,
                    "Starting agent",
                )));
                data.revision = data.revision.wrapping_add(1);
                drop(data);
                child.shutdown().await;
                inner
                    .version
                    .send_modify(|value| *value = value.wrapping_add(1));
                message = next;
                continue 'run;
            }
            drop(data);
            child.shutdown().await;
            finish_failed(
                &session,
                RuntimeError::new("run_interrupted", "agent run was interrupted"),
                true,
            )
            .await;
            inner
                .version
                .send_modify(|value| *value = value.wrapping_add(1));
            break 'run;
        }
        let mut first_turn = true;
        let outcome = loop {
            let (conversation, turn_cancel) = {
                let mut d = session.data.lock().await;
                let startup_cancel = if first_turn {
                    first_turn = false;
                    Some(&cancel)
                } else {
                    None
                };
                let turn_cancel = install_turn_cancel(&mut d, startup_cancel);
                (d.conversation.clone(), turn_cancel)
            };
            let Some(mut candidate) = conversation else {
                break Err(super::provider::ProviderError {
                    kind: "internal_error",
                    message: "agent conversation is unavailable".into(),
                    resumable: false,
                });
            };
            let outcome = inner
                .provider
                .run(
                    ProviderRun {
                        definition: &session.definition,
                        credential: &credential,
                        system_context: &session.context,
                        child: &child,
                        cancel: &turn_cancel,
                        reporter: &reporter,
                        workspace: &inner.workspace,
                    },
                    &message,
                    &mut candidate,
                )
                .await;
            let mut d = session.data.lock().await;
            d.conversation = Some(candidate);
            d.cancel = None;
            if let Err(error) = &outcome {
                if error.kind == "run_interrupted"
                    && let Some(next) = pop_input_for_execution(&mut d)
                {
                    d.activity = Some(AgentActivity::new(AgentActivityEvent::new(
                        ActivityPhase::Waiting,
                        "Waiting for next input",
                    )));
                    d.revision = d.revision.wrapping_add(1);
                    drop(d);
                    inner.version.send_modify(|v| *v = v.wrapping_add(1));
                    message = next;
                    continue;
                }
                if !error.resumable {
                    d.queue.clear();
                }
                break outcome;
            }
            if let Some(next) = pop_input_for_execution(&mut d) {
                d.activity = Some(AgentActivity::new(AgentActivityEvent::new(
                    ActivityPhase::Waiting,
                    "Waiting for next input",
                )));
                d.revision = d.revision.wrapping_add(1);
                drop(d);
                inner.version.send_modify(|v| *v = v.wrapping_add(1));
                message = next;
                continue;
            }
            d.cancel = None;
            break outcome;
        };
        {
            let mut data = session.data.lock().await;
            data.accepting_input = false;
        }
        child.shutdown().await;
        let mut d = session.data.lock().await;
        if outcome.is_ok()
            && let Some(next) = pop_input_for_execution(&mut d)
        {
            d.accepting_input = true;
            d.activity = Some(AgentActivity::new(AgentActivityEvent::new(
                ActivityPhase::Starting,
                "Starting agent",
            )));
            d.revision = d.revision.wrapping_add(1);
            drop(d);
            inner.version.send_modify(|v| *v = v.wrapping_add(1));
            message = next;
            continue 'run;
        }
        set_terminal(&mut d, outcome);
        drop(d);
        // Error text is deliberately not logged: provider errors may be externally supplied.
        let terminal = session.data.lock().await;
        let state = terminal.state.clone();
        let error_kind = terminal.error.as_ref().map(|error| error.kind.clone());
        let total = terminal
            .terminal_at
            .unwrap_or_else(Instant::now)
            .saturating_duration_since(session.created_at);
        drop(terminal);
        match state {
            AgentState::Completed => {
                tracing::info!(agent_id = %id, event = "completed", total_ms = millis(total), "agent activity")
            }
            AgentState::Failed => {
                tracing::info!(agent_id = %id, event = "failed", total_ms = millis(total), error_kind = ?error_kind, "agent activity")
            }
            AgentState::Running => {}
        }
        inner.version.send_modify(|v| *v = v.wrapping_add(1));
        break 'run;
    }
    drop(permit);
    cleanup_terminal_sessions(&inner).await;
}

#[cfg(test)]
mod tests;
