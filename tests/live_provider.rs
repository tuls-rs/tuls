//! Live end-to-end tests against the real `tuls` binary and a real external
//! provider (OpenRouter).
//!
//! These tests spawn the actual compiled binary as stdio MCP servers and
//! drive real model requests. They exercise only the external provider path;
//! deterministic offline coverage of the same servers lives in the README
//! conformance tests (`tests/readme.rs`).
//!
//! Gating: a live test runs only when the process environment sets `TULS_LIVE`
//! to exactly `1`. A `.env` file next to `Cargo.toml` may supply credential
//! *values* but never enables the suite by itself.
//!
//! Credentials and configuration (process environment first, then `.env`):
//!   OPENROUTER_API_KEY - required; no OpenAI fallback is accepted
//!   TULS_LIVE_MODEL    - required; expected value: `openai/gpt-5.6-luna`
//!
//! The provider endpoint is fixed to `https://openrouter.ai/api/v1`; agent
//! definitions use `model_provider = "openrouter"` with no base_url, env_key,
//! or wire_api overrides. Secrets are never printed; skipped tests only
//! report a marker line.

#[path = "common/client.rs"]
mod common;

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use common::{TulsServer, read_file, structured_of, toml_tuls_bin};
use serde_json::{Value, json};
use tempfile::TempDir;

const SKIP_MARKER: &str = "SKIPPED (live tests disabled)";
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const OPENROUTER_MODEL: &str = "openai/gpt-5.6-luna";

// ---------------------------------------------------------------------------
// gating, .env loading, and the pure selection helper
// ---------------------------------------------------------------------------

struct LiveConfig {
    api_key: String,
    base_url: String,
    model: String,
}

/// Pure selection of the live configuration from a process-environment
/// resolver and the parsed `.env` values. Side-effect free so the gating
/// semantics are unit-testable without mutating the process environment.
fn select_config(
    process: &dyn Fn(&str) -> Option<String>,
    dotenv: &BTreeMap<String, String>,
) -> Option<LiveConfig> {
    if process("TULS_LIVE").as_deref() != Some("1") {
        return None;
    }
    let api_key = resolve_value(process, dotenv, "OPENROUTER_API_KEY")?;
    let model = resolve_value(process, dotenv, "TULS_LIVE_MODEL")?;
    if model != OPENROUTER_MODEL {
        return None;
    }
    Some(LiveConfig {
        api_key,
        base_url: OPENROUTER_BASE_URL.to_string(),
        model,
    })
}

fn resolve_value(
    process: &dyn Fn(&str) -> Option<String>,
    dotenv: &BTreeMap<String, String>,
    key: &str,
) -> Option<String> {
    process(key)
        .or_else(|| dotenv.get(key).cloned())
        .filter(|value| !value.trim().is_empty())
}

fn read_dotenv() -> BTreeMap<String, String> {
    let path = project_root().join(".env");
    let Ok(text) = fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        values.insert(key.trim().to_string(), trim_quotes(value.trim()));
    }
    values
}

fn load_config() -> Option<LiveConfig> {
    select_config(&|key| env::var(key).ok(), &read_dotenv())
}

fn trim_quotes(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
        .to_string()
}

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn skip() {
    eprintln!(
        "{SKIP_MARKER}: {}",
        std::thread::current().name().unwrap_or("?")
    );
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn resolver(
        entries: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<String> {
        move |key| {
            entries
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        }
    }

    fn dotenv(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn configured() -> LiveConfig {
        let process = resolver(&[
            ("TULS_LIVE", "1"),
            ("OPENROUTER_API_KEY", "sk-test-key"),
            ("TULS_LIVE_MODEL", "openai/gpt-5.6-luna"),
        ]);
        select_config(&process, &BTreeMap::new()).expect("exact TULS_LIVE=1 enables")
    }

    #[test]
    fn disabled_unless_tuls_live_is_exactly_one() {
        for value in ["", "0", "true", "yes", "2", "on"] {
            let process = |key: &str| match key {
                "TULS_LIVE" => Some(value.to_string()),
                "OPENROUTER_API_KEY" => Some("sk-test-key".to_string()),
                "TULS_LIVE_MODEL" => Some("openai/gpt-5.6-luna".to_string()),
                _ => None,
            };
            assert!(
                select_config(&process, &BTreeMap::new()).is_none(),
                "{value:?} must not enable the live suite"
            );
        }
    }

    #[test]
    fn dotenv_never_opts_in() {
        let process = resolver(&[]);
        let dotenv = dotenv(&[
            ("OPENROUTER_API_KEY", "sk-dotenv-key"),
            ("TULS_LIVE_MODEL", "openai/gpt-5.6-luna"),
        ]);
        assert!(
            select_config(&process, &dotenv).is_none(),
            "a .env file must not enable the live suite"
        );
    }

    #[test]
    fn enabled_only_by_exact_one() {
        let process = resolver(&[
            ("TULS_LIVE", "1"),
            ("OPENROUTER_API_KEY", "sk-test-key"),
            ("TULS_LIVE_MODEL", "openai/gpt-5.6-luna"),
        ]);
        let config = select_config(&process, &BTreeMap::new()).expect("exact 1 enables");
        assert_eq!(config.api_key, "sk-test-key");
        assert_eq!(config.model, "openai/gpt-5.6-luna");
    }

    #[test]
    fn dotenv_supplies_values_when_process_lacks_them() {
        let process = resolver(&[("TULS_LIVE", "1")]);
        let dotenv = dotenv(&[
            ("OPENROUTER_API_KEY", "sk-dotenv-key"),
            ("TULS_LIVE_MODEL", OPENROUTER_MODEL),
        ]);
        let config = select_config(&process, &dotenv).expect("dotenv values apply");
        assert_eq!(config.api_key, "sk-dotenv-key");
        assert_eq!(config.model, OPENROUTER_MODEL);
    }

    #[test]
    fn process_values_win_over_dotenv() {
        let process = resolver(&[
            ("TULS_LIVE", "1"),
            ("OPENROUTER_API_KEY", "sk-process-key"),
            ("TULS_LIVE_MODEL", OPENROUTER_MODEL),
        ]);
        let dotenv = dotenv(&[
            ("OPENROUTER_API_KEY", "sk-dotenv-key"),
            ("TULS_LIVE_MODEL", OPENROUTER_MODEL),
        ]);
        let config = select_config(&process, &dotenv).expect("process values win");
        assert_eq!(config.api_key, "sk-process-key");
        assert_eq!(config.model, OPENROUTER_MODEL);
    }

    #[test]
    fn openai_key_is_never_accepted_as_a_credential() {
        let process = resolver(&[("TULS_LIVE", "1")]);
        let dotenv = dotenv(&[
            ("OPENAI_API_KEY", "sk-openai-fallback"),
            ("OPENAI_MODEL", "some-model"),
            ("TULS_LIVE_MODEL", "openai/gpt-5.6-luna"),
        ]);
        assert!(
            select_config(&process, &dotenv).is_none(),
            "OPENAI_* values must never satisfy the live configuration"
        );
    }

    #[test]
    fn model_is_required() {
        let process = resolver(&[("TULS_LIVE", "1"), ("OPENROUTER_API_KEY", "sk-test-key")]);
        assert!(select_config(&process, &BTreeMap::new()).is_none());
    }

    #[test]
    fn model_must_match_the_live_openrouter_contract() {
        let process = resolver(&[
            ("TULS_LIVE", "1"),
            ("OPENROUTER_API_KEY", "sk-test-key"),
            ("TULS_LIVE_MODEL", "another/model"),
        ]);
        assert!(select_config(&process, &BTreeMap::new()).is_none());
    }

    #[test]
    fn api_key_is_required() {
        let process = resolver(&[
            ("TULS_LIVE", "1"),
            ("TULS_LIVE_MODEL", "openai/gpt-5.6-luna"),
        ]);
        assert!(select_config(&process, &BTreeMap::new()).is_none());
    }

    #[test]
    fn empty_values_are_treated_as_missing() {
        let process = resolver(&[
            ("TULS_LIVE", "1"),
            ("OPENROUTER_API_KEY", ""),
            ("TULS_LIVE_MODEL", "  "),
        ]);
        assert!(select_config(&process, &BTreeMap::new()).is_none());
    }

    #[test]
    fn endpoint_is_the_fixed_openrouter_base() {
        assert_eq!(configured().base_url, "https://openrouter.ai/api/v1");
    }
}

// ---------------------------------------------------------------------------
// Agent definitions against the real provider
// ---------------------------------------------------------------------------

fn write_agent_toml(workspace: &Path, name: &str, body: &str) {
    let dir = workspace.join(".agents/agents");
    fs::create_dir_all(&dir).expect("create agents dir");
    fs::write(dir.join(format!("{name}.toml")), body).expect("write agent TOML");
}

fn write_skill(workspace: &Path) {
    let dir = workspace.join(".agents/skills/markdown-writer");
    fs::create_dir_all(&dir).expect("create skill dir");
    fs::write(
        dir.join("SKILL.md"),
        "---\nname: markdown-writer\ndescription: Writes markdown reports that must start with an H1 heading and end with the footer comment.\n---\nWhen you write a markdown report:\n1. The first line must be an H1 heading (#) naming the report.\n2. Use a bullet list for key findings.\n3. End the file with exactly this comment on its own line: <!-- reviewed-by: ai -->\n",
    )
    .expect("write SKILL.md");
}

/// TOML body for an agent definition against the real OpenRouter provider.
/// The provider is first-class, so the definition carries no base_url,
/// env_key, or wire_api overrides.
fn provider_agent(
    name: &str,
    description: &str,
    instructions: &str,
    live: &LiveConfig,
    extras: &str,
) -> String {
    format!(
        "name = \"{name}\"\n\
         description = \"{description}\"\n\
         instructions = \"\"\"{instructions}\"\"\"\n\
         model_provider = \"openrouter\"\n\
         model = \"{model}\"\n\
         max_turns = 60\n\
         {extras}",
        model = live.model,
    )
}

fn tuls_command(args: &str) -> String {
    format!("command = \"{}\"\nargs = [{args}]", toml_tuls_bin())
}

async fn spawn_agents_server(workspace: &Path, live: &LiveConfig) -> TulsServer {
    TulsServer::connect(
        &["agents", workspace.to_str().unwrap()],
        &[("OPENROUTER_API_KEY", live.api_key.clone())],
    )
    .await
}

/// Spawn an agent through the agents MCP server and return its agentId.
async fn spawn_agent(server: &TulsServer, name: &str, task: &str) -> String {
    let spawned = structured_of(
        &server
            .call_ok("spawn_agent", json!({"name": name, "task": task}))
            .await,
    );
    let id = spawned
        .get("agentId")
        .and_then(Value::as_str)
        .expect("spawn_agent returned agentId")
        .to_string();
    assert_eq!(
        spawned.get("status").and_then(Value::as_str),
        Some("running"),
        "spawn_agent status: {spawned}"
    );
    id
}

/// Wait for agents to reach a terminal state; returns the wait result JSON.
async fn wait_agents(server: &TulsServer, targets: &[&str], timeout_ms: u64) -> Value {
    server
        .call_ok(
            "wait_agent",
            json!({"targets": targets, "timeoutMs": timeout_ms}),
        )
        .await
}

fn agent_results(wait: &Value) -> Vec<Value> {
    wait.get("structuredContent")
        .and_then(|value| value.get("agents"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn assert_agent_completed(wait: &Value, label: &str) -> Vec<Value> {
    let results = agent_results(wait);
    assert!(!results.is_empty(), "{label}: no agent results in {wait}");
    for result in &results {
        let id = result.get("agentId").and_then(Value::as_str).unwrap_or("?");
        let status = result.get("status").and_then(Value::as_str).unwrap_or("?");
        let error = result
            .get("error")
            .map(Value::to_string)
            .unwrap_or_else(|| "none".into());
        assert_eq!(
            status, "completed",
            "{label}: agent {id} ended with status {status:?} error={error} full={result}"
        );
        let outcome = result
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            !outcome.trim().is_empty(),
            "{label}: agent {id} completed with an empty result"
        );
    }
    results
}

const SUPERVISOR_INSTRUCTIONS: &str = "\
You coordinate workspace tasks and verify every step before moving on. \
Work inside the workspace directory only. All file paths are relative to the workspace root. \
When a step fails, retry it once with a corrected call before reporting. \
Your final result must be a checklist of the numbered steps below with the status of each.";

#[tokio::test]
async fn live_agents_supervisor_uses_all_mcps_tools_and_subagents() {
    let Some(live) = load_config() else {
        skip();
        return;
    };
    let workspace = TempDir::new().expect("tempdir");
    let root = workspace.path();

    fs::create_dir_all(root.join("notes")).expect("notes dir");
    fs::write(
        root.join("notes/original.txt"),
        "The quick brown fox jumps over the lazy dog.\nRust edition 2024.\n",
    )
    .expect("original note");
    write_skill(root);
    write_agent_toml(
        root,
        "supervisor",
        &provider_agent(
            "supervisor",
            "Coordinates workspace tasks",
            SUPERVISOR_INSTRUCTIONS,
            &live,
            &format!(
                "skills = [\"markdown-writer\"]\n\
                 allow_tools = [\"filesystem/*\", \"fetch/*\", \"memory/*\", \"shell/*\", \"skills/*\", \"agents/*\"]\n\
                 \n\
                 [mcp_servers.filesystem]\n{fs_args}\n\
                 [mcp_servers.fetch]\n{fetch_args}\n\
                 [mcp_servers.memory]\n{memory_args}\n\
                 [mcp_servers.shell]\n{shell_args}\n\
                 [mcp_servers.skills]\n{skills_args}\n\
                 [mcp_servers.agents]\n{agents_args}\n\
                 env = {{ OPENROUTER_API_KEY = \"${{OPENROUTER_API_KEY}}\" }}",
                fs_args = tuls_command("\"filesystem\", \".\""),
                fetch_args = tuls_command("\"fetch\", \"--network\", \"public\""),
                memory_args = tuls_command("\"memory\", \"--memory-file\", \"memory.jsonl\""),
                shell_args = tuls_command("\"shell\", \".\""),
                skills_args = tuls_command("\"skills\", \".\""),
                agents_args = tuls_command("\"agents\", \".\""),
            ),
        ),
    );
    write_agent_toml(
        root,
        "researcher",
        &provider_agent(
            "researcher",
            "Fetches URLs and writes summaries",
            "Fetch the URL given in the task, then write a one-paragraph summary to the file path \
             given in the task using write_file. Report the file path you wrote and its first sentence.",
            &live,
            &format!(
                "allow_tools = [\"filesystem/*\", \"fetch/*\"]\n\
                 \n\
                 [mcp_servers.filesystem]\n{fs_args}\n\
                 [mcp_servers.fetch]\n{fetch_args}",
                fs_args = tuls_command(
                    "\"filesystem\", \".\", \"--allow\", \"filesystem.read\", \"--allow\", \"filesystem.write\"",
                ),
                fetch_args = tuls_command("\"fetch\", \"--network\", \"public\""),
            ),
        ),
    );

    let server = spawn_agents_server(root, &live).await;

    let task = "\
Complete each step in order and verify each result before continuing:

1. List the workspace root with list_directory.
2. Read notes/original.txt with read_text_file.
3. Activate the markdown-writer skill with activate_skill on the skills server and follow its formatting rules for the report in step 6.
4. Create memory entities: an entity named 'fox' of type 'animal' with observation 'swift', and an entity named 'notes' of type 'document'. Create a relation 'notes' -> 'fox' with relationType 'mentions'. Add an observation 'is a document' to 'notes'.
5. Search the memory graph with search_nodes for 'fox', then read the full graph with read_graph.
6. Write notes/report.md with write_file: an H1 heading, a bullet list with the memory entity names, the fetched page title from step 7, and the required skill footer.
7. Fetch https://example.com/ with the fetch tool (maxLength 3000) and record the page title.
8. Use execute_command on the shell server to print the first line of notes/original.txt (program \"head\", args [\"-n\", \"1\", \"notes/original.txt\"]).
9. Spawn the researcher agent with task \"Fetch https://example.com/ and write a one-paragraph summary to notes/research.md\". Save its agentId, call wait_agent on it, and confirm it completed without error.
10. Move notes/report.md to notes/final-report.md with move_file and report the final workspace state.

Finish by reporting the checklist of steps 1-10 with the status of each.";

    let supervisor_id = spawn_agent(&server, "supervisor", task).await;
    let wait = wait_agents(&server, &[&supervisor_id], 300_000).await;
    let results = assert_agent_completed(&wait, "supervisor");

    let outcome = results[0]
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or_default();
    eprintln!("supervisor final report:\n{outcome}");

    let report = root.join("notes/final-report.md");
    assert!(
        report.is_file(),
        "notes/final-report.md was not created (moved report missing)"
    );
    let report_text = read_file(&report);
    assert!(
        report_text.starts_with('#'),
        "skill rule violated: report must start with an H1 heading, got: {report_text}"
    );
    assert!(
        report_text.contains("<!-- reviewed-by: ai -->"),
        "skill rule violated: report must end with the required footer comment: {report_text}"
    );
    assert!(
        report_text.to_ascii_lowercase().contains("fox"),
        "report should mention the memory entity: {report_text}"
    );

    let research = root.join("notes/research.md");
    assert!(
        research.is_file(),
        "researcher subagent did not write notes/research.md"
    );
    let research_text = read_file(&research);
    assert!(
        research_text.to_ascii_lowercase().contains("example"),
        "research.md should mention example.com content: {research_text}"
    );

    let memory_file = root.join("memory.jsonl");
    assert!(memory_file.is_file(), "memory file missing");
    let memory_text = read_file(&memory_file);
    assert!(
        memory_text.contains("\"name\":\"fox\"") || memory_text.contains("\"name\": \"fox\""),
        "memory graph should contain the fox entity: {memory_text}"
    );
}

/// Dedicated send_input resume: run to completion, send follow-up input, run
/// to completion again. No automatic retry anywhere in this flow.
#[tokio::test]
async fn live_agents_send_input_resumes_agent() {
    let Some(live) = load_config() else {
        skip();
        return;
    };
    let workspace = TempDir::new().expect("tempdir");
    let root = workspace.path();
    fs::create_dir_all(root.join("notes")).expect("notes dir");
    write_agent_toml(
        root,
        "note-writer",
        &provider_agent(
            "note-writer",
            "Writes workspace notes",
            "Write a short text file at the path given in the task using write_file. \
             After the write succeeds, reply with a brief confirmation naming the file.",
            &live,
            &format!(
                "allow_tools = [\"filesystem/*\"]\n\
                 \n\
                 [mcp_servers.filesystem]\n{fs_args}",
                fs_args = tuls_command("\"filesystem\", \".\""),
            ),
        ),
    );

    let server = spawn_agents_server(root, &live).await;

    let first_id = spawn_agent(
        &server,
        "note-writer",
        "Write notes/sequence.txt containing exactly the text: first draft",
    )
    .await;
    let first_wait = wait_agents(&server, &[&first_id], 120_000).await;
    assert_agent_completed(&first_wait, "note-writer first run");

    let resume_message =
        "Append a second line to notes/sequence.txt containing exactly: second draft";
    let ack = structured_of(
        &server
            .call_ok(
                "send_input",
                json!({"target": first_id, "message": resume_message}),
            )
            .await,
    );
    assert_eq!(
        ack.get("accepted").and_then(Value::as_bool),
        Some(true),
        "send_input was not accepted: {ack}"
    );

    let resumed_wait = wait_agents(&server, &[&first_id], 120_000).await;
    assert_agent_completed(&resumed_wait, "note-writer resumed run");

    let sequence = root.join("notes/sequence.txt");
    assert!(sequence.is_file(), "notes/sequence.txt missing");
    let text = read_file(&sequence);
    assert!(text.contains("first draft"), "sequence.txt: {text}");
    assert!(text.contains("second draft"), "sequence.txt: {text}");
}

#[tokio::test]
async fn live_agents_skill_context_injection() {
    let Some(live) = load_config() else {
        skip();
        return;
    };
    let workspace = TempDir::new().expect("tempdir");
    let root = workspace.path();
    write_skill(root);
    write_agent_toml(
        root,
        "skill-check",
        &provider_agent(
            "skill-check",
            "Reports the skill instructions in its context",
            "Report the exact markdown formatting rules that were loaded into your context by the \
             markdown-writer skill. Quote the three numbered rules and the footer comment requirement \
             verbatim, then state how many skills are loaded.",
            &live,
            "skills = [\"markdown-writer\"]",
        ),
    );

    let server = spawn_agents_server(root, &live).await;

    let id = spawn_agent(
        &server,
        "skill-check",
        "Report the skill instructions in your context.",
    )
    .await;
    let wait = wait_agents(&server, &[&id], 120_000).await;
    let results = assert_agent_completed(&wait, "skill-check");

    let outcome = results[0]
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        outcome.contains("reviewed-by: ai") && outcome.contains("h1 heading"),
        "agent did not receive the skill instructions in its context: {outcome}"
    );
}
