//! The agent stubs Claude Code runs a work order's worker as, one per role
//! and rung, named `bal-<role>-<rung>` (D-06, D-22, HST-R12).
//!
//! A stub fixes no model and defines no server. It inherits the session's
//! model, or the Agent tool's `model` parameter when the order names one
//! (D-23), and names the session's own `baley` entry, so every subagent shares
//! the session's connection (ADR 0034). Its body only sends the worker to its
//! order, so the words a worker follows stay in the registry. The bytes
//! depend on nothing but the role, rung and name tables.

use baley_core::policy::Host;
use baley_core::policy::schema::{Role, Rung};
use baley_core::work_order::agent_name;
use serde_json::json;

use super::registration::KEY;
use super::stubs::{Entry, Kind};
use crate::store::model::digest;

/// The text of the stub `role` runs at `rung`: YAML frontmatter holding its
/// name, a description, `model: inherit`, the rung's effort and the session's
/// server by name, then one body line.
///
/// The description is a JSON string literal, as a skill stub writes its own,
/// and says to delegate only when an order names the stub.
fn render(role: Role, rung: Rung) -> String {
    let name = agent_name(role, rung);
    let description = format!(
        "Baley {} at {} effort. Use only when a Baley work order names this agent.",
        role.name(),
        rung.name()
    );
    let description = serde_json::to_string(&description).expect("a string serializes");
    let effort = rung.name();
    let query = json!({
        "operation": "document",
        "identity": {"kind": "work_order", "id": "<that id>"},
    });
    format!(
        "---\nname: {name}\ndescription: {description}\nmodel: inherit\neffort: {effort}\nmcpServers:\n  - {KEY}\n---\n\nRead the Baley work order whose id your prompt gives: call `baley_query` with `{query}` and follow it.\n"
    )
}

/// Every agent stub, for each role of [`Role::ALL`] each rung of
/// [`Rung::ALL`].
pub fn manifest() -> Vec<Entry> {
    Role::ALL
        .into_iter()
        .flat_map(|role| Rung::ALL.into_iter().map(move |rung| (role, rung)))
        .map(|(role, rung)| {
            let bytes = render(role, rung).into_bytes();
            Entry {
                host: Host::ClaudeCode,
                kind: Kind::Agent,
                identity: agent_name(role, rung),
                digest: digest(&bytes),
                bytes,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The role and rung names, written out from design 0003.
    const ROLES: [&str; 6] = [
        "planner", "analyzer", "checker", "executor", "verifier", "reviewer",
    ];
    const RUNGS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

    /// `bal-planner-high` as design 0012 and D-22 spell it, every line ending
    /// in a newline.
    const PLANNER_HIGH: &str = concat!(
        "---\n",
        "name: bal-planner-high\n",
        "description: \"Baley planner at high effort. Use only when a Baley work order names this agent.\"\n",
        "model: inherit\n",
        "effort: high\n",
        "mcpServers:\n",
        "  - baley\n",
        "---\n",
        "\n",
        "Read the Baley work order whose id your prompt gives: call `baley_query` with `{\"operation\":\"document\",\"identity\":{\"kind\":\"work_order\",\"id\":\"<that id>\"}}` and follow it.\n",
    );

    /// Each expected (identity, rung), roles outer and rungs inner.
    fn expected() -> Vec<(String, &'static str)> {
        ROLES
            .iter()
            .flat_map(|role| {
                RUNGS
                    .iter()
                    .map(move |rung| (format!("bal-{role}-{rung}"), *rung))
            })
            .collect()
    }

    /// The frontmatter lines and the body of a rendered stub.
    fn split(stub: &str) -> (Vec<&str>, &str) {
        let (frontmatter, body) = stub
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .expect("a frontmatter block");
        (frontmatter.lines().collect(), body)
    }

    #[test]
    fn an_agent_stub_off_the_role_rung_set_or_named_otherwise_is_caught() {
        let entries = manifest();
        assert_eq!(entries.len(), 30);
        let identities: Vec<&str> = entries
            .iter()
            .map(|entry| entry.identity.as_str())
            .collect();
        let expected: Vec<String> = expected().into_iter().map(|(name, _)| name).collect();
        assert_eq!(identities, expected);
        assert_eq!(identities[0], "bal-planner-low");
        assert_eq!(identities[29], "bal-reviewer-max");
        let mut distinct = identities.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), 30);
        for entry in &entries {
            assert_eq!(entry.kind, Kind::Agent, "{}", entry.identity);
            assert_eq!(entry.host, Host::ClaudeCode, "{}", entry.identity);
        }
    }

    #[test]
    fn an_agent_stub_with_a_fixed_model_an_inline_server_or_another_effort_is_caught() {
        let entries = manifest();
        assert_eq!(entries.len(), 30);
        for (entry, (name, rung)) in entries.iter().zip(expected()) {
            let stub = std::str::from_utf8(&entry.bytes).unwrap();
            let (lines, body) = split(stub);
            assert_eq!(lines.len(), 6, "{name}: {lines:?}");
            assert_eq!(lines[0], format!("name: {name}"));
            assert!(
                lines[1].starts_with("description: \"") && lines[1].ends_with('"'),
                "{name}: {}",
                lines[1]
            );
            assert_eq!(lines[2], "model: inherit", "{name}");
            assert_eq!(lines[3], format!("effort: {rung}"), "{name}");
            assert_eq!(lines[4], "mcpServers:", "{name}");
            assert_eq!(lines[5], "  - baley", "{name}");
            for key in ["command:", "type:", "args:", "url:", "tools:"] {
                assert!(!stub.contains(key), "{name} holds {key}");
            }
            let body: Vec<&str> = body
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect();
            assert_eq!(body.len(), 1, "{name}: {body:?}");
            assert!(body[0].contains("baley_query"), "{name}");
            assert!(body[0].contains(r#""kind":"work_order""#), "{name}");
        }
    }

    #[test]
    fn an_agent_stub_rendered_from_anything_but_the_compiled_tables_is_caught() {
        let find = || {
            manifest()
                .into_iter()
                .find(|entry| entry.identity == "bal-planner-high")
                .expect("bal-planner-high is rendered")
        };
        let (first, second) = (find(), find());
        assert_eq!(first.bytes.len(), 348);
        assert_eq!(std::str::from_utf8(&first.bytes).unwrap(), PLANNER_HIGH);
        assert_eq!(
            first.digest,
            "bad29486e8d5164fa6f1af909d8fe3a2a47e3edb3ed5c4a0e48cefeff8f83108"
        );
        assert_eq!(first, second);
    }
}
