use std::time::Duration;

pub(crate) const PROVIDER_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const PROVIDER_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) const CHILD_MCP_STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const CHILD_MCP_CALL_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const CHILD_MCP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
