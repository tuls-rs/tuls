use std::{future::Future, pin::Pin, sync::Arc};

pub(crate) const SUMMARY_LIMIT: usize = 120;
pub(crate) const TARGET_LIMIT: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActivityPhase {
    Starting,
    Model,
    Tool,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentActivityEvent {
    pub(crate) phase: ActivityPhase,
    pub(crate) summary: String,
    pub(crate) target: Option<String>,
    pub(crate) tool: Option<String>,
    pub(crate) kind: &'static str,
}

impl AgentActivityEvent {
    pub(crate) fn new(phase: ActivityPhase, summary: impl Into<String>) -> Self {
        Self {
            phase,
            summary: bound(summary.into(), SUMMARY_LIMIT),
            target: None,
            tool: None,
            kind: "activity",
        }
    }

    pub(crate) fn tool(summary: String, tool: String, target: Option<String>) -> Self {
        Self {
            phase: ActivityPhase::Tool,
            summary: bound(summary, SUMMARY_LIMIT),
            target: target.map(|value| bound(value, TARGET_LIMIT)),
            tool: Some(bound(tool, TARGET_LIMIT)),
            kind: "tool_started",
        }
    }

    pub(crate) fn tool_completed() -> Self {
        Self {
            phase: ActivityPhase::Model,
            summary: "Waiting for model response".into(),
            target: None,
            tool: None,
            kind: "tool_completed",
        }
    }

    pub(crate) fn tool_failed() -> Self {
        Self {
            phase: ActivityPhase::Model,
            summary: "Tool call failed".into(),
            target: None,
            tool: None,
            kind: "tool_failed",
        }
    }

    pub(crate) fn tool_timed_out() -> Self {
        Self {
            phase: ActivityPhase::Model,
            summary: "Tool call timed out".into(),
            target: None,
            tool: None,
            kind: "tool_timed_out",
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

pub(crate) fn bound(mut value: String, limit: usize) -> String {
    value.retain(|character| !character.is_control());
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
