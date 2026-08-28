use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use schemars::JsonSchema;
use serde::Serialize;

use super::timeouts::{CHILD_MCP_CALL_TIMEOUT, PROVIDER_REQUEST_TIMEOUT};

pub(crate) const SUMMARY_LIMIT: usize = 120;
pub(crate) const TARGET_LIMIT: usize = 160;

#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivityPhase {
    Starting,
    Model,
    Tool,
    Waiting,
    Completed,
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentActivity {
    pub(crate) phase: ActivityPhase,
    pub(crate) summary: String,
    pub(crate) tool: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) started_at: Instant,
    pub(crate) deadline: Option<Instant>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivitySnapshot {
    pub(crate) phase: ActivityPhase,
    pub(crate) summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<String>,
    pub(crate) activity_elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operation_timeout_remaining_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentActivityEvent {
    pub(crate) phase: ActivityPhase,
    pub(crate) summary: String,
    pub(crate) target: Option<String>,
    pub(crate) tool: Option<String>,
    pub(crate) deadline: Option<Instant>,
    pub(crate) kind: &'static str,
}
impl AgentActivityEvent {
    pub(crate) fn new(phase: ActivityPhase, summary: impl Into<String>) -> Self {
        Self {
            phase,
            summary: bound(summary.into(), SUMMARY_LIMIT),
            target: None,
            tool: None,
            deadline: None,
            kind: "activity",
        }
    }
    pub(crate) fn tool(summary: String, tool: String, target: Option<String>) -> Self {
        Self {
            phase: ActivityPhase::Tool,
            summary: bound(summary, SUMMARY_LIMIT),
            target: target.map(|value| bound(value, TARGET_LIMIT)),
            tool: Some(bound(tool, TARGET_LIMIT)),
            deadline: Some(Instant::now() + CHILD_MCP_CALL_TIMEOUT),
            kind: "tool_started",
        }
    }
    pub(crate) fn tool_completed() -> Self {
        Self {
            phase: ActivityPhase::Model,
            summary: "Waiting for model response".into(),
            target: None,
            tool: None,
            deadline: Some(Instant::now() + PROVIDER_REQUEST_TIMEOUT),
            kind: "tool_completed",
        }
    }
    pub(crate) fn tool_failed() -> Self {
        Self {
            phase: ActivityPhase::Model,
            summary: "Tool call failed".into(),
            target: None,
            tool: None,
            deadline: None,
            kind: "tool_failed",
        }
    }
    pub(crate) fn tool_timed_out() -> Self {
        Self {
            phase: ActivityPhase::Model,
            summary: "Tool call timed out".into(),
            target: None,
            tool: None,
            deadline: None,
            kind: "tool_timed_out",
        }
    }
}
impl AgentActivity {
    pub(crate) fn new(event: AgentActivityEvent) -> Self {
        let now = Instant::now();
        Self {
            phase: event.phase,
            summary: event.summary,
            target: event.target,
            tool: event.tool,
            started_at: now,
            deadline: event.deadline,
        }
    }
    pub(crate) fn snapshot(&self, now: Instant) -> ActivitySnapshot {
        ActivitySnapshot {
            phase: self.phase.clone(),
            summary: self.summary.clone(),
            target: self.target.clone(),
            tool: self.tool.clone(),
            activity_elapsed_ms: millis(now.saturating_duration_since(self.started_at)),
            operation_timeout_remaining_ms: self
                .deadline
                .map(|deadline| millis(deadline.saturating_duration_since(now))),
        }
    }
}
pub(crate) type ReportFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
#[derive(Clone)]
pub(crate) struct ActivityReporter(Arc<dyn Fn(AgentActivityEvent) -> ReportFuture + Send + Sync>);
impl ActivityReporter {
    pub(crate) fn new(
        callback: impl Fn(AgentActivityEvent) -> ReportFuture + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(callback))
    }
    pub(crate) async fn report(&self, event: AgentActivityEvent) {
        (self.0)(event).await
    }
}
pub(crate) fn millis(value: Duration) -> u64 {
    value.as_millis().min(u64::MAX as u128) as u64
}
pub(crate) fn bound(mut value: String, limit: usize) -> String {
    value.retain(|c| !c.is_control());
    if value.len() <= limit {
        return value;
    }
    let mut end = limit.saturating_sub('…'.len_utf8());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

#[cfg(test)]
mod tests;
