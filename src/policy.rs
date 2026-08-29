use std::{collections::BTreeSet, fmt, str::FromStr};

/// A coarse-grained capability granted to a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    FilesystemRead,
    FilesystemWrite,
    NetworkFetch,
    MemoryRead,
    MemoryWrite,
    ProcessExecute,
    SkillsRead,
    AgentsRun,
}

impl Capability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FilesystemRead => "filesystem.read",
            Self::FilesystemWrite => "filesystem.write",
            Self::NetworkFetch => "network.fetch",
            Self::MemoryRead => "memory.read",
            Self::MemoryWrite => "memory.write",
            Self::ProcessExecute => "process.execute",
            Self::SkillsRead => "skills.read",
            Self::AgentsRun => "agents.run",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Capability {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "filesystem.read" => Ok(Self::FilesystemRead),
            "filesystem.write" => Ok(Self::FilesystemWrite),
            "network.fetch" => Ok(Self::NetworkFetch),
            "memory.read" => Ok(Self::MemoryRead),
            "memory.write" => Ok(Self::MemoryWrite),
            "process.execute" => Ok(Self::ProcessExecute),
            "skills.read" => Ok(Self::SkillsRead),
            "agents.run" => Ok(Self::AgentsRun),
            _ => Err(format!("unknown capability {value:?}")),
        }
    }
}

/// Static authorization metadata for one built-in MCP tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    pub server: &'static str,
    pub name: &'static str,
    pub capability: Capability,
}

impl ToolSpec {
    pub const fn new(server: &'static str, name: &'static str, capability: Capability) -> Self {
        Self {
            server,
            name,
            capability,
        }
    }

    pub fn id(self) -> String {
        format!("{}/{}", self.server, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Selector {
    Capability(Capability),
    Tool(String),
}

/// Policy applied to a built-in server's advertised and callable tool routes.
///
/// With no `allow` selectors, all tools belonging to that server are allowed.
/// Once at least one `allow` selector is supplied, the policy becomes an
/// allowlist. `deny` always takes precedence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPolicy {
    allow: Option<BTreeSet<Selector>>,
    deny: BTreeSet<Selector>,
}

impl ToolPolicy {
    pub fn from_selectors(
        allow: &[String],
        deny: &[String],
        specs: &[ToolSpec],
    ) -> Result<Self, String> {
        let allow = if allow.is_empty() {
            None
        } else {
            Some(parse_selectors(allow, specs)?)
        };
        let deny = parse_selectors(deny, specs)?;
        Ok(Self { allow, deny })
    }

    pub fn allows(&self, spec: ToolSpec) -> bool {
        let capability = Selector::Capability(spec.capability);
        let tool = Selector::Tool(spec.id());
        if self.deny.contains(&capability) || self.deny.contains(&tool) {
            return false;
        }
        self.allow
            .as_ref()
            .is_none_or(|allow| allow.contains(&capability) || allow.contains(&tool))
    }

    /// Return whether a capability itself is granted, independent of any exact tool id.
    pub fn allows_capability(&self, capability: Capability) -> bool {
        let selector = Selector::Capability(capability);
        if self.deny.contains(&selector) {
            return false;
        }
        self.allow
            .as_ref()
            .is_none_or(|allow| allow.contains(&selector))
    }

    pub fn allows_auxiliary_surface(
        &self,
        capability: Capability,
        controlling_tool: ToolSpec,
    ) -> bool {
        self.allows_capability(capability)
            && !self.deny.contains(&Selector::Tool(controlling_tool.id()))
    }
}

fn parse_selectors(values: &[String], specs: &[ToolSpec]) -> Result<BTreeSet<Selector>, String> {
    values
        .iter()
        .map(|value| parse_selector(value, specs))
        .collect()
}

fn parse_selector(value: &str, specs: &[ToolSpec]) -> Result<Selector, String> {
    if let Ok(capability) = Capability::from_str(value) {
        if specs.iter().any(|spec| spec.capability == capability) {
            return Ok(Selector::Capability(capability));
        }
        return Err(format!(
            "capability {value:?} is not provided by this server"
        ));
    }
    if specs.iter().any(|spec| spec.id() == value) {
        return Ok(Selector::Tool(value.to_owned()));
    }
    Err(format!(
        "unknown tool policy selector {value:?}; use a capability or server/tool id"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPECS: &[ToolSpec] = &[
        ToolSpec::new("filesystem", "read_text_file", Capability::FilesystemRead),
        ToolSpec::new("filesystem", "write_file", Capability::FilesystemWrite),
    ];

    #[test]
    fn default_allows_and_deny_wins() {
        let policy = ToolPolicy::from_selectors(&[], &[], SPECS).unwrap();
        assert!(policy.allows(SPECS[0]));
        assert!(policy.allows(SPECS[1]));

        let policy = ToolPolicy::from_selectors(
            &["filesystem.read".into(), "filesystem.write".into()],
            &["filesystem/write_file".into()],
            SPECS,
        )
        .unwrap();
        assert!(policy.allows(SPECS[0]));
        assert!(!policy.allows(SPECS[1]));
    }

    #[test]
    fn allowlist_is_fail_closed_and_rejects_typos() {
        let policy = ToolPolicy::from_selectors(&["filesystem.read".into()], &[], SPECS).unwrap();
        assert!(policy.allows(SPECS[0]));
        assert!(!policy.allows(SPECS[1]));
        assert!(ToolPolicy::from_selectors(&["filesystem.reed".into()], &[], SPECS).is_err());
        assert!(ToolPolicy::from_selectors(&["filesystem/missing".into()], &[], SPECS).is_err());
    }

    #[test]
    fn every_capability_round_trips_and_displays() {
        for value in [
            "filesystem.read",
            "filesystem.write",
            "network.fetch",
            "memory.read",
            "memory.write",
            "process.execute",
            "skills.read",
            "agents.run",
        ] {
            let capability = Capability::from_str(value).expect(value);
            assert_eq!(capability.as_str(), value);
            assert_eq!(capability.to_string(), value);
        }
        assert!(Capability::from_str("filesystem-read").is_err());
        assert!(Capability::from_str("filesystem.read_text_file").is_err());
    }

    #[test]
    fn capability_from_another_server_is_rejected() {
        for foreign in [
            "network.fetch",
            "memory.read",
            "process.execute",
            "skills.read",
            "agents.run",
        ] {
            let error = ToolPolicy::from_selectors(&[foreign.into()], &[], SPECS)
                .expect_err("foreign capability must be rejected");
            assert!(
                error.contains("not provided by this server"),
                "{foreign}: {error}"
            );
        }
    }

    #[test]
    fn selectors_are_case_sensitive() {
        assert!(ToolPolicy::from_selectors(&["Filesystem.Read".into()], &[], SPECS).is_err());
        assert!(
            ToolPolicy::from_selectors(&["filesystem/READ_TEXT_FILE".into()], &[], SPECS).is_err()
        );
        // A mis-cased selector is invalid rather than silently matched.
        assert!(ToolPolicy::from_selectors(&[], &["FILESYSTEM.READ".into()], SPECS).is_err());
        // Case-exact selectors still work in both positions.
        let policy = ToolPolicy::from_selectors(
            &["filesystem.read".into()],
            &["filesystem/write_file".into()],
            SPECS,
        )
        .unwrap();
        assert!(policy.allows(SPECS[0]));
        assert!(!policy.allows(SPECS[1]));
    }

    #[test]
    fn deny_capability_wins_over_exact_tool_allow() {
        let policy = ToolPolicy::from_selectors(
            &["filesystem/read_text_file".into()],
            &["filesystem.read".into()],
            SPECS,
        )
        .unwrap();
        assert!(!policy.allows(SPECS[0]));
        assert!(!policy.allows_capability(Capability::FilesystemRead));
    }

    #[test]
    fn exact_tool_allow_does_not_grant_the_capability() {
        let policy =
            ToolPolicy::from_selectors(&["filesystem/read_text_file".into()], &[], SPECS).unwrap();
        assert!(policy.allows(SPECS[0]));
        assert!(!policy.allows(SPECS[1]));
        // The capability itself is not granted, so capability-scoped
        // surfaces (e.g. the fetch prompt, memory resources) stay disabled.
        assert!(!policy.allows_capability(Capability::FilesystemRead));
    }

    #[test]
    fn allows_capability_respects_deny_over_allow() {
        let policy = ToolPolicy::from_selectors(
            &["filesystem.read".into()],
            &["filesystem.read".into()],
            SPECS,
        )
        .unwrap();
        assert!(!policy.allows_capability(Capability::FilesystemRead));
        assert!(!policy.allows(SPECS[0]));
    }

    #[test]
    fn agents_tools_follow_the_same_selector_semantics() {
        let specs = &[
            ToolSpec::new("agents", "spawn_agent", Capability::AgentsRun),
            ToolSpec::new("agents", "send_input", Capability::AgentsRun),
        ];

        let policy = ToolPolicy::from_selectors(&[], &[], specs).unwrap();
        assert!(policy.allows(specs[0]));
        assert!(policy.allows_capability(Capability::AgentsRun));

        let policy = ToolPolicy::from_selectors(&["agents.run".into()], &[], specs).unwrap();
        assert!(policy.allows(specs[1]));
        assert!(policy.allows_capability(Capability::AgentsRun));

        let policy =
            ToolPolicy::from_selectors(&["agents/spawn_agent".into()], &[], specs).unwrap();
        assert!(policy.allows(specs[0]));
        assert!(!policy.allows(specs[1]), "exact grant is narrower");
        assert!(
            !policy.allows_capability(Capability::AgentsRun),
            "an exact tool grant must not enable the whole capability"
        );

        let policy = ToolPolicy::from_selectors(
            &["agents.run".into()],
            &["agents/send_input".into()],
            specs,
        )
        .unwrap();
        assert!(policy.allows(specs[0]));
        assert!(!policy.allows(specs[1]));

        assert!(ToolPolicy::from_selectors(&["agents/unknown".into()], &[], specs).is_err());
        assert!(
            ToolPolicy::from_selectors(&["shell/*".into()], &[], specs).is_err(),
            "child-style wildcard selectors are not valid built-in selectors"
        );
    }
}
