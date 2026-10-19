//! The route record: what was asked, what was sent, what Claude Code's effort
//! table supports and what was observed, kept apart (design 0003 section 6,
//! CFG-R15).
//!
//! It lives here rather than beside `resolve_route` because it reads the
//! catalog's compiled tables, and the policy module takes everything it
//! judges as supplied values.
//!
//! The effective model and effort are always `unknown`: nothing in this
//! build observes a run, and filling them from the table or the agent stub
//! would make an alias, or a concrete id under a cap, read as proven.

use serde_json::{Value, json};

use crate::catalog::{EFFORT_TABLE_DATE, SupportedEffort, host_aliases, supported_effort};
use crate::policy::{Host, Route, SettingSource};

/// What an effective value is until a run is observed.
const UNKNOWN: &str = "unknown";
/// The route passes no model, so the session's own runs.
const NO_MODEL: &str = "no-model";
/// The model is one of the host's aliases, which the host resolves itself.
const ALIAS: &str = "alias";
/// The model is a concrete id, but no run of it was observed.
const UNOBSERVED_ID: &str = "unobserved-id";
/// A managed `maxEffortLevel` or a per-role cap may lower any level, and
/// nothing here resolves one.
const POSSIBLE_HOST_CAP: &str = "possible-host-cap";

/// The JSON a work order records for `route` and the `route` query serves:
/// `role`, `model` (text or null), `starting_rung`, `rung`, `attempt`,
/// `escalated`, `host_effort`, `supported_effort` (a rung name,
/// `not-applicable` or `unknown`), `effective_model` and `effective_effort`
/// (each `unknown`), `unknown_because`, `effort_source` and `model_source`
/// (each `{setting, layer, file}`), `policy_version`, `catalog_version` and
/// `reasons`.
///
/// The supported level is the table's answer for the model at the requested
/// rung, after escalation. With no model, or an alias, it is `unknown`.
/// `unknown_because` gives the model's reason (`no-model`, `alias` or
/// `unobserved-id`), then `possible-host-cap`. `reasons` is the route's own,
/// then one sentence giving the supported answer and the table's date.
pub fn route_record(route: &Route, host: Host) -> Value {
    let table = format!("Claude Code's effort table of {EFFORT_TABLE_DATE}");
    let rung = route.rung.name();
    let (supported, because, sentence) = match route.model.as_deref() {
        None => (
            SupportedEffort::Unknown,
            NO_MODEL,
            format!("no model is passed, so {table} cannot say what the session's model supports"),
        ),
        Some(model) if host_aliases(host).contains(&model) => (
            SupportedEffort::Unknown,
            ALIAS,
            format!(
                "{model} is a {} alias, which {table} does not list",
                host.name()
            ),
        ),
        Some(model) => {
            let supported = match host {
                Host::ClaudeCode => supported_effort(model, route.rung),
            };
            let sentence = match supported {
                SupportedEffort::Level(level) => format!(
                    "{table} supports {} for {model} at rung {rung}",
                    level.name()
                ),
                SupportedEffort::NotApplicable => {
                    format!("{table} lists {model} with no effort setting")
                }
                SupportedEffort::Unknown => format!("{table} does not list {model}"),
            };
            (supported, UNOBSERVED_ID, sentence)
        }
    };
    let mut reasons = route.reasons.clone();
    reasons.push(sentence);
    json!({
        "role": route.role.name(),
        "model": route.model,
        "starting_rung": route.starting_rung.name(),
        "rung": rung,
        "attempt": route.attempt,
        "escalated": route.escalated,
        "host_effort": route.host_effort,
        "supported_effort": supported.as_str(),
        "effective_model": UNKNOWN,
        "effective_effort": UNKNOWN,
        "unknown_because": [because, POSSIBLE_HOST_CAP],
        "effort_source": source_value(&route.effort_source),
        "model_source": source_value(&route.model_source),
        "policy_version": route.policy_version,
        "catalog_version": route.catalog_version,
        "reasons": reasons,
    })
}

fn source_value(source: &SettingSource) -> Value {
    json!({
        "setting": source.setting,
        "layer": source.layer.name(),
        "file": source.file.as_ref().map(|file| file.display().to_string()),
    })
}

#[cfg(test)]
mod tests {
    //! Expected values come from design 0003 section 6, the effort table's
    //! rows of 2026-10-18 and the phase's decisions, never from running this
    //! code. Each route is resolved by `resolve_route` from supplied settings.

    use std::num::NonZeroU32;

    use super::*;
    use crate::catalog::rung_map;
    use crate::policy::{
        AcceptedNames, Role, RouteRequest, Schema, SettingsFile, effective_policy, resolve_route,
    };

    /// The planner's route under a global file holding `text`, at `attempt`.
    fn planner(text: &str, attempt: u32) -> Route {
        let global = SettingsFile {
            path: "/c/config.toml".into(),
            bytes: text.as_bytes().to_vec(),
            digest: "digest of /c/config.toml".into(),
        };
        let policy = effective_policy(
            Schema::standard(),
            Some(Host::ClaudeCode),
            Some(&global),
            None,
        )
        .unwrap();
        let names = [
            "opus",
            "sonnet",
            "haiku",
            "fable",
            "claude-opus-5-5",
            "claude-opus-4-6",
            "claude-haiku-4-5-20251001",
            "claude-made-up-9",
        ];
        let catalog = AcceptedNames {
            names: names.iter().map(|name| name.to_string()).collect(),
            version: 3,
        };
        resolve_route(&RouteRequest {
            policy: &policy,
            policy_version: 7,
            role: Role::Planner,
            host: Host::ClaudeCode,
            attempt: NonZeroU32::new(attempt).unwrap(),
            catalog: &catalog,
            rungs: &rung_map(Host::ClaudeCode),
        })
        .unwrap()
    }

    fn record(text: &str, attempt: u32) -> Value {
        route_record(&planner(text, attempt), Host::ClaudeCode)
    }

    #[test]
    fn an_effective_model_or_effort_filled_for_any_route_is_caught() {
        let cases = [
            ("", json!(["no-model", "possible-host-cap"]), "unknown"),
            (
                "roles.planner.model = \"opus\"\n",
                json!(["alias", "possible-host-cap"]),
                "unknown",
            ),
            (
                "roles.planner.model = \"claude-opus-5-5\"\n",
                json!(["unobserved-id", "possible-host-cap"]),
                "high",
            ),
            (
                "roles.planner.model = \"claude-made-up-9\"\n",
                json!(["unobserved-id", "possible-host-cap"]),
                "unknown",
            ),
        ];
        for (text, because, supported) in cases {
            let record = record(text, 1);
            assert_eq!(record["effective_model"], "unknown", "{text}");
            assert_eq!(record["effective_effort"], "unknown", "{text}");
            assert_eq!(record["unknown_because"], because, "{text}");
            assert_eq!(record["supported_effort"], supported, "{text}");
        }
    }

    #[test]
    fn a_supported_level_taken_from_the_starting_rung_or_another_model_is_caught() {
        let capped = record(
            "roles.planner.model = \"claude-opus-4-6\"\nroles.planner.effort = \"xhigh\"\n",
            1,
        );
        assert_eq!(capped["rung"], "xhigh");
        assert_eq!(capped["host_effort"], "xhigh");
        assert_eq!(capped["supported_effort"], "high");

        let none = record("roles.planner.model = \"claude-haiku-4-5-20251001\"\n", 1);
        assert_eq!(none["supported_effort"], "not-applicable");

        let escalated = record(
            "escalate_on_failure = true\nroles.planner.model = \"claude-opus-5-5\"\n",
            2,
        );
        assert_eq!(escalated["starting_rung"], "high");
        assert_eq!(escalated["rung"], "xhigh");
        assert_eq!(escalated["escalated"], true);
        assert_eq!(escalated["supported_effort"], "xhigh");
    }

    #[test]
    fn a_route_record_missing_a_member_or_carrying_another_is_caught() {
        let text = "roles.planner.model = \"claude-opus-5-5\"\n";
        let route = planner(text, 1);
        let record = route_record(&route, Host::ClaudeCode);
        let mut members: Vec<&str> = record
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        members.sort_unstable();
        let mut expected = [
            "role",
            "model",
            "starting_rung",
            "rung",
            "attempt",
            "escalated",
            "host_effort",
            "supported_effort",
            "effective_model",
            "effective_effort",
            "unknown_because",
            "effort_source",
            "model_source",
            "policy_version",
            "catalog_version",
            "reasons",
        ];
        expected.sort_unstable();
        assert_eq!(members, expected);
        assert_eq!(record["role"], "planner");
        assert_eq!(record["model"], "claude-opus-5-5");
        assert_eq!(record["starting_rung"], "high");
        assert_eq!(record["rung"], "high");
        assert_eq!(record["attempt"], 1);
        assert_eq!(record["escalated"], false);
        assert_eq!(record["host_effort"], "high");
        assert_eq!(
            record["model_source"],
            json!({"setting": "roles.planner.model", "layer": "global",
                "file": "/c/config.toml"})
        );
        assert_eq!(
            record["effort_source"],
            json!({"setting": "roles.planner.effort", "layer": "default", "file": null})
        );
        assert_eq!(record["policy_version"], 7);
        assert_eq!(record["catalog_version"], 3);
        let reasons: Vec<&str> = record["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|reason| reason.as_str().unwrap())
            .collect();
        assert_eq!(reasons.len(), route.reasons.len() + 1);
        assert_eq!(reasons[..route.reasons.len()], route.reasons);
        assert!(
            reasons.last().unwrap().contains("2026-10-18"),
            "{reasons:?}"
        );
    }
}
