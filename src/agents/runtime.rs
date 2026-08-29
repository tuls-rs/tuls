use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rmcp::task_manager::TaskContext;
use schemars::JsonSchema;
use serde::Serialize;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use super::{
    activity::{ActivityPhase, ActivityReporter, AgentActivityEvent},
    child_mcp::ChildMcpManager,
    definition::{
        AgentDefinition, MAX_BUILT_CONTEXT_BYTES, MAX_SEND_MESSAGE_BYTES, MAX_SPAWN_TASK_BYTES,
    },
    discovery::AgentRegistry,
    provider::{ConversationState, ProviderClient, ProviderCredential, ProviderRun},
};
use crate::{skills::SkillRegistry, support::truncate_text};

const RUNTIME_CAPACITY: usize = 8;
const MAX_RETAINED_IDLE_SESSIONS: usize = 64;
const MAX_AGENT_RESULT_BYTES: usize = 24 * 1024;
const AGENT_TURN_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub(crate) const AGENT_TASK_TTL_MS: u64 = 35 * 60 * 1000;

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTurnResult {
    #[serde(rename = "agentId")]
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) result: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTurnError {
    #[serde(rename = "agentId")]
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) resumable: bool,
}

pub(crate) enum TurnOutcome {
    Completed(AgentTurnResult),
    Failed(AgentTurnError),
    Cancelled,
}

struct SessionData {
    conversation: ConversationState,
    resumable: bool,
    last_accessed_at: Instant,
}

struct Session {
    definition: Arc<AgentDefinition>,
    context: String,
    data: Mutex<SessionData>,
    busy: AtomicBool,
}

struct TurnLease {
    session: Arc<Session>,
}

impl Drop for TurnLease {
    fn drop(&mut self) {
        self.session.busy.store(false, Ordering::Release);
    }
}

pub(crate) struct AgentTurn {
    id: String,
    message: String,
    initial: bool,
    lease: TurnLease,
    _permit: OwnedSemaphorePermit,
}

struct Inner {
    workspace: PathBuf,
    registry: Arc<AgentRegistry>,
    skills: Arc<SkillRegistry>,
    provider: ProviderClient,
    sessions: Mutex<BTreeMap<String, Arc<Session>>>,
    capacity: Arc<Semaphore>,
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
        Ok(Self {
            inner: Arc::new(Inner {
                workspace,
                registry: Arc::new(registry),
                skills: Arc::new(skills),
                provider,
                sessions: Mutex::new(BTreeMap::new()),
                capacity: Arc::new(Semaphore::new(RUNTIME_CAPACITY)),
            }),
        })
    }

    pub(crate) fn registry(&self) -> &AgentRegistry {
        &self.inner.registry
    }

    pub(crate) async fn prepare_spawn(
        &self,
        name: &str,
        task: &str,
    ) -> Result<AgentTurn, RuntimeError> {
        validate_message(task, MAX_SPAWN_TASK_BYTES, "task")?;
        let definition = self
            .inner
            .registry
            .get(name)
            .ok_or_else(|| RuntimeError::new("unknown_agent", "unknown agent"))?;
        let context = self.context(&definition)?;
        let permit = self.acquire_capacity()?;
        cleanup_idle_sessions(&self.inner).await;

        let id = format!("agt_{}", uuid::Uuid::now_v7().simple());
        let session = Arc::new(Session {
            definition: definition.clone(),
            context,
            data: Mutex::new(SessionData {
                conversation: ConversationState::new(&definition.wire_api),
                resumable: false,
                last_accessed_at: Instant::now(),
            }),
            busy: AtomicBool::new(true),
        });
        self.inner
            .sessions
            .lock()
            .await
            .insert(id.clone(), session.clone());
        tracing::info!(agent_id = %id, agent = %definition.name, event = "created", "agent session");

        Ok(AgentTurn {
            id,
            message: task.to_owned(),
            initial: true,
            lease: TurnLease { session },
            _permit: permit,
        })
    }

    pub(crate) async fn prepare_input(
        &self,
        target: &str,
        message: &str,
    ) -> Result<AgentTurn, RuntimeError> {
        validate_message(message, MAX_SEND_MESSAGE_BYTES, "message")?;
        cleanup_idle_sessions(&self.inner).await;
        let session = self
            .inner
            .sessions
            .lock()
            .await
            .get(target)
            .cloned()
            .ok_or_else(|| RuntimeError::new("unknown_agent", "unknown agent session"))?;

        if session
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(RuntimeError::new(
                "agent_busy",
                "agent session already has a running task",
            ));
        }
        let lease = TurnLease {
            session: session.clone(),
        };
        let permit = self.acquire_capacity()?;
        {
            let mut data = session.data.lock().await;
            data.last_accessed_at = Instant::now();
            if !data.resumable {
                return Err(RuntimeError::new(
                    "non_resumable",
                    "agent session cannot be resumed",
                ));
            }
            // A turn is resumable only after its outcome is known. If the task
            // is hard-aborted by its TTL or server shutdown, the session stays
            // fail-closed rather than replaying from an uncertain boundary.
            data.resumable = false;
        }

        Ok(AgentTurn {
            id: target.to_owned(),
            message: message.to_owned(),
            initial: false,
            lease,
            _permit: permit,
        })
    }

    pub(crate) async fn execute(&self, turn: AgentTurn, task_context: TaskContext) -> TurnOutcome {
        let AgentTurn {
            id,
            message,
            initial,
            lease,
            _permit,
        } = turn;
        let session = lease.session.clone();
        let name = session.definition.name.clone();

        if task_context.is_cancel_requested() {
            drop(lease);
            if initial {
                self.remove_session(&id, &session).await;
            } else {
                // No provider request or child tool execution has started yet,
                // so the conversation boundary is unchanged: restore the
                // reserved follow-up turn.
                let mut data = session.data.lock().await;
                data.resumable = true;
                data.last_accessed_at = Instant::now();
            }
            return TurnOutcome::Cancelled;
        }

        task_context.set_status_message("Starting agent");
        let credential = match ProviderCredential::resolve(&session.definition) {
            Ok(credential) => credential,
            Err(error) => {
                let failure = ExecutionFailure::from_provider(error);
                self.record_failure(&session, &failure).await;
                drop(lease);
                return self
                    .finish_failure(&id, &name, initial, &session, failure)
                    .await;
            }
        };

        let cancel = CancellationToken::new();
        let _cancel_on_drop = CancelOnDrop(cancel.clone());
        let reporter = activity_reporter(task_context.clone(), id.clone());
        let mut execution =
            Box::pin(self.run_turn(&session, &message, &credential, &cancel, &reporter));
        let deadline = tokio::time::sleep(AGENT_TURN_TIMEOUT);
        tokio::pin!(deadline);

        let outcome = tokio::select! {
            result = &mut execution => map_execution_result(&id, &name, result),
            _ = task_context.cancelled() => {
                cancel.cancel();
                match execution.await {
                    Err(error) if error.kind == "run_interrupted" => TurnOutcome::Cancelled,
                    result => map_execution_result(&id, &name, result),
                }
            }
            _ = &mut deadline => {
                cancel.cancel();
                match execution.await {
                    Err(error) if error.kind == "run_interrupted" => TurnOutcome::Failed(AgentTurnError {
                        id: id.clone(),
                        name: name.clone(),
                        kind: "turn_timeout".into(),
                        message: "agent turn exceeded the 30-minute execution limit".into(),
                        resumable: true,
                    }),
                    result => map_execution_result(&id, &name, result),
                }
            }
        };

        drop(lease);
        if initial && matches!(outcome, TurnOutcome::Cancelled) {
            self.remove_session(&id, &session).await;
        }
        cleanup_idle_sessions(&self.inner).await;

        match &outcome {
            TurnOutcome::Completed(_) => {
                tracing::info!(agent_id = %id, event = "completed", "agent turn")
            }
            TurnOutcome::Failed(error) => {
                tracing::info!(agent_id = %id, event = "failed", error_kind = %error.kind, resumable = error.resumable, "agent turn")
            }
            TurnOutcome::Cancelled => {
                tracing::info!(agent_id = %id, event = "cancelled", "agent turn")
            }
        }
        outcome
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

    fn acquire_capacity(&self) -> Result<OwnedSemaphorePermit, RuntimeError> {
        self.inner
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError::new("capacity_exceeded", "agent runtime is at capacity"))
    }

    async fn run_turn(
        &self,
        session: &Session,
        message: &str,
        credential: &ProviderCredential,
        cancel: &CancellationToken,
        reporter: &ActivityReporter,
    ) -> Result<String, ExecutionFailure> {
        reporter
            .report(AgentActivityEvent::new(
                ActivityPhase::Starting,
                "Starting child MCP servers",
            ))
            .await;
        let mut child = match ChildMcpManager::connect(
            &session.definition,
            &self.inner.workspace,
            cancel,
        )
        .await
        {
            Ok(child) => child,
            Err(_) => {
                let failure = if cancel.is_cancelled() {
                    ExecutionFailure::interrupted()
                } else {
                    ExecutionFailure::new(
                        "child_mcp_startup_error",
                        "unable to start configured child MCP servers",
                        true,
                    )
                };
                self.record_failure(session, &failure).await;
                return Err(failure);
            }
        };

        let mut candidate = {
            let data = session.data.lock().await;
            data.conversation.clone()
        };
        let result = self
            .inner
            .provider
            .run(
                ProviderRun {
                    definition: &session.definition,
                    credential,
                    system_context: &session.context,
                    child: &child,
                    cancel,
                    reporter,
                    workspace: &self.inner.workspace,
                },
                message,
                &mut candidate,
            )
            .await;
        child.shutdown().await;

        let mut data = session.data.lock().await;
        data.conversation = candidate;
        data.last_accessed_at = Instant::now();
        match result {
            Ok(result) => {
                data.resumable = true;
                Ok(truncate_text(
                    &result,
                    MAX_AGENT_RESULT_BYTES,
                    "\n[truncated: agent result exceeded the output limit]",
                ))
            }
            Err(error) => {
                data.resumable = error.resumable;
                Err(ExecutionFailure::from_provider(error))
            }
        }
    }

    async fn record_failure(&self, session: &Session, failure: &ExecutionFailure) {
        let mut data = session.data.lock().await;
        data.resumable = failure.resumable;
        data.last_accessed_at = Instant::now();
    }

    async fn finish_failure(
        &self,
        id: &str,
        name: &str,
        initial: bool,
        session: &Arc<Session>,
        failure: ExecutionFailure,
    ) -> TurnOutcome {
        if initial && !failure.resumable {
            self.remove_session(id, session).await;
        }
        TurnOutcome::Failed(AgentTurnError {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: failure.kind,
            message: failure.message,
            resumable: failure.resumable,
        })
    }

    async fn remove_session(&self, id: &str, candidate: &Arc<Session>) {
        let mut sessions = self.inner.sessions.lock().await;
        if sessions
            .get(id)
            .is_some_and(|retained| Arc::ptr_eq(retained, candidate))
        {
            sessions.remove(id);
        }
    }
}

fn validate_message(value: &str, limit: usize, field: &str) -> Result<(), RuntimeError> {
    if value.trim().is_empty() {
        return Err(RuntimeError::new(
            "invalid_request",
            format!("{field} must not be empty"),
        ));
    }
    if value.len() > limit {
        return Err(RuntimeError::new(
            "invalid_request",
            format!("{field} exceeds the {limit}-byte limit"),
        ));
    }
    Ok(())
}

fn map_execution_result(
    id: &str,
    name: &str,
    result: Result<String, ExecutionFailure>,
) -> TurnOutcome {
    match result {
        Ok(result) => TurnOutcome::Completed(AgentTurnResult {
            id: id.to_owned(),
            name: name.to_owned(),
            result,
        }),
        Err(error) => TurnOutcome::Failed(AgentTurnError {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: error.kind,
            message: error.message,
            resumable: error.resumable,
        }),
    }
}

fn activity_reporter(task_context: TaskContext, agent_id: String) -> ActivityReporter {
    ActivityReporter::new(move |event| {
        let task_context = task_context.clone();
        let agent_id = agent_id.clone();
        Box::pin(async move {
            task_context.set_status_message(event.summary.clone());
            tracing::info!(
                agent_id = %agent_id,
                event = event.kind,
                phase = ?event.phase,
                tool = ?event.tool,
                target = ?event.target,
                "agent activity"
            );
        })
    })
}

#[derive(Debug)]
struct ExecutionFailure {
    kind: String,
    message: String,
    resumable: bool,
}

impl ExecutionFailure {
    fn new(kind: impl Into<String>, message: impl Into<String>, resumable: bool) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            resumable,
        }
    }

    fn interrupted() -> Self {
        Self::new("run_interrupted", "agent run was interrupted", true)
    }

    fn from_provider(error: super::provider::ProviderError) -> Self {
        Self::new(error.kind, error.message, error.resumable)
    }
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn cleanup_idle_sessions(inner: &Arc<Inner>) {
    let entries: Vec<_> = inner
        .sessions
        .lock()
        .await
        .iter()
        .map(|(id, session)| (id.clone(), session.clone()))
        .collect();
    let mut idle = Vec::new();
    for (id, session) in entries {
        if session.busy.load(Ordering::Acquire) {
            continue;
        }
        let last_accessed_at = session.data.lock().await.last_accessed_at;
        idle.push((last_accessed_at, id, session));
    }
    if idle.len() <= MAX_RETAINED_IDLE_SESSIONS {
        return;
    }
    idle.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    let remove = idle.len() - MAX_RETAINED_IDLE_SESSIONS;
    for (accessed, id, candidate) in idle.into_iter().take(remove) {
        let mut sessions = inner.sessions.lock().await;
        let Some(retained) = sessions.get(&id).cloned() else {
            continue;
        };
        if !Arc::ptr_eq(&retained, &candidate) || retained.busy.load(Ordering::Acquire) {
            continue;
        }
        let last_accessed_at = retained.data.lock().await.last_accessed_at;
        if last_accessed_at == accessed {
            sessions.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests;
