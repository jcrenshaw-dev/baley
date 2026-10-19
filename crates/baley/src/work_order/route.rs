//! The route an order is issued under (design 0003 section 6, CFG-R13 to
//! CFG-R17): the host's accepted model names, read from the per-user `user`
//! project, and the route composed from the host's effective policy.

use std::num::NonZeroU32;

use baley_core::catalog::{
    Catalog, MODEL_CATALOG_VIEW, USER_PROJECT, accepted_names, catalog_key, rung_map, state_key,
};
use baley_core::policy::{
    AcceptedNames, EffectivePolicy, Host, Role, Route, RouteRefusal, RouteRequest, resolve_route,
};
use baley_core::work_order::{agent_name, route_record};
use baley_store::{Admin, ProjectId, Refusal, StoreError, Views};
use serde_json::Value;

/// The model names `host` accepts and the catalog version, read from the
/// `user` project outside any transaction: the host's catalog document and
/// the state document, in one snapshot. When `user` is absent, or a newer
/// binary has brought its views past this one's so the store answers it
/// read-only, the names are the host's aliases at version 0, as the policy
/// step reads the version. The read never creates or seeds `user`.
pub fn host_names(store: &(impl Admin + Views), host: Host) -> Result<AcceptedNames, StoreError> {
    let user = ProjectId(USER_PROJECT.into());
    // The aliases alone, at version 0: what the catalog accepts with no entry.
    let aliases = || names(host, None, None);
    if !store.projects()?.iter().any(|(known, _)| *known == user) {
        return aliases();
    }
    let keys = [catalog_key(Catalog::Host(host)), state_key()];
    let documents = match store.get_many(&user, MODEL_CATALOG_VIEW, &keys) {
        Ok(documents) => documents,
        // The fence on `user` must not reach the project being started.
        Err(StoreError::Refused(Refusal::ProjectReadOnly { .. })) => return aliases(),
        Err(error) => return Err(error),
    };
    let mut bodies = documents
        .into_iter()
        .map(|document| document.map(|document| document.body));
    let catalog = bodies.next().flatten();
    let state = bodies.next().flatten();
    names(host, catalog.as_ref(), state.as_ref())
}

fn names(
    host: Host,
    catalog: Option<&Value>,
    state: Option<&Value>,
) -> Result<AcceptedNames, StoreError> {
    accepted_names(host.name(), catalog, state)
        .map_err(|error| StoreError::Unavailable(error.to_string()))
}

/// One order's route, composed: the route itself, the record the order
/// carries and the agent stub that runs it.
#[derive(Debug, Clone, PartialEq)]
pub struct IssuedRoute {
    /// The route `resolve_route` gave.
    pub route: Route,
    /// The route record, which keeps what was asked, what was sent, what the
    /// effort table supports and what was observed apart.
    pub record: Value,
    /// The agent stub for the role at the rung run, `bal-<role>-<rung>`.
    pub agent: String,
}

/// Composes the route for `role` at `attempt` under `policy`, the host's
/// effective policy at `policy_version`, with the host's rung map and the
/// accepted `names`. A stored model the catalog does not accept is
/// `resolve_route`'s `unknown-model` refusal, unchanged (CFG-R14).
pub fn compose(
    policy: &EffectivePolicy,
    policy_version: u64,
    host: Host,
    role: Role,
    attempt: NonZeroU32,
    names: &AcceptedNames,
) -> Result<IssuedRoute, RouteRefusal> {
    let rungs = rung_map(host);
    let route = resolve_route(&RouteRequest {
        policy,
        policy_version,
        role,
        host,
        attempt,
        catalog: names,
        rungs: &rungs,
    })?;
    Ok(IssuedRoute {
        record: route_record(&route, host),
        agent: agent_name(role, route.rung),
        route,
    })
}

#[cfg(test)]
mod tests {
    //! Expected values come from design 0003 section 6, Claude Code's
    //! identity rung map and the effort table's rows, never from this code.

    use std::collections::BTreeSet;
    use std::path::Path;

    use baley_core::catalog::owner_changed_payload;
    use baley_core::catalog::{MODELS_OWNER_CHANGED, MODELS_STREAM, OwnerChange};
    use baley_core::policy::{Schema, effective_policy};
    use baley_store::{
        Actor, Command, CommandKind, Decision, Hash, NewEvent, Observed, OutcomeKind, RequestId,
        StreamName,
    };
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::scope::fixtures::{T0, T1};

    fn policy(global: &str) -> EffectivePolicy {
        let file = crate::settings::file(Path::new("/c/config.toml"), global.into());
        effective_policy(
            Schema::standard(),
            Some(Host::ClaudeCode),
            Some(&file),
            None,
        )
        .unwrap()
    }

    fn names(extra: &[&str], version: u64) -> AcceptedNames {
        let mut names: BTreeSet<String> = ["fable", "haiku", "opus", "sonnet"]
            .into_iter()
            .map(String::from)
            .collect();
        names.extend(extra.iter().map(|name| (*name).to_string()));
        AcceptedNames { names, version }
    }

    fn attempt(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    #[test]
    fn a_route_composed_under_another_map_attempt_or_version_is_caught() {
        let policy =
            policy("escalate_on_failure = true\n[roles.planner]\nmodel = \"claude-opus-5-5\"\n");
        let names = names(&["claude-opus-5-5"], 3);
        let compose_at = |n| {
            compose(
                &policy,
                7,
                Host::ClaudeCode,
                Role::Planner,
                attempt(n),
                &names,
            )
        };

        let first = compose_at(1).unwrap();
        assert_eq!(first.record["rung"], "high");
        assert_eq!(first.record["host_effort"], "high");
        assert_eq!(first.record["supported_effort"], "high");
        assert_eq!(first.record["escalated"], false);
        assert_eq!(first.agent, "bal-planner-high");

        let second = compose_at(2).unwrap();
        assert_eq!(second.record["rung"], "xhigh");
        assert_eq!(second.record["host_effort"], "xhigh");
        assert_eq!(second.record["supported_effort"], "xhigh");
        assert_eq!(second.record["escalated"], true);
        assert_eq!(second.agent, "bal-planner-xhigh");

        for issued in [&first, &second] {
            assert_eq!(issued.record["policy_version"], 7);
            assert_eq!(issued.record["catalog_version"], 3);
            assert_eq!(issued.record["model"], "claude-opus-5-5");
        }
    }

    #[test]
    fn a_stored_model_the_catalog_refuses_routed_anyway_is_caught() {
        let policy = policy("[roles.planner]\nmodel = \"claude-gone\"\n");
        let refusal = compose(
            &policy,
            1,
            Host::ClaudeCode,
            Role::Planner,
            attempt(1),
            &names(&[], 0),
        )
        .unwrap_err();
        assert_eq!(refusal.code(), "unknown-model");
        let text = refusal.to_string();
        assert!(text.contains("roles.planner.model"), "{text}");
        assert!(text.contains("claude-gone"), "{text}");
    }

    fn open(home: &Path, options: baley_store_sqlite::Options) -> SqliteStore {
        crate::ledger::open::store(home, T0, options).unwrap()
    }

    /// Appends `models.owner_changed` events to `user` in one owner command,
    /// as the catalog tests record owner entries, and gives the last one's
    /// sequence number.
    fn add_owner_entries(store: &SqliteStore, entries: &[(Catalog, &str)]) -> u64 {
        let user = ProjectId(USER_PROJECT.into());
        let command = Command {
            project: user,
            kind: CommandKind("test.append".into()),
            request_id: RequestId("00000000-0000-4000-8000-000000000001".into()),
            digest: Hash([0xee; 32]),
            scope: vec![],
            policy_version: 0,
            recorded_at: T1.into(),
            actor: Actor::Owner,
            caller: None,
        };
        let mut last = 0;
        store
            .transact(&command, &mut |tx| {
                for (catalog, name) in entries {
                    last = tx.append(NewEvent {
                        stream: StreamName(MODELS_STREAM.into()),
                        type_name: MODELS_OWNER_CHANGED.into(),
                        type_version: 1,
                        git: None,
                        payload: owner_changed_payload(*catalog, name, OwnerChange::Added(None), 0),
                        attachments: vec![],
                    })?;
                }
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();
        last
    }

    fn lists_user(store: &SqliteStore) -> bool {
        store
            .projects()
            .unwrap()
            .iter()
            .any(|(id, _)| id.0 == USER_PROJECT)
    }

    #[test]
    fn a_host_catalog_read_that_seeds_user_or_mixes_in_a_provider_is_caught() {
        use baley_core::catalog::Provider;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = open(&home, crate::ledger::open::options());
        assert_eq!(host_names(&store, Host::ClaudeCode).unwrap(), names(&[], 0));
        assert!(!lists_user(&store), "the read created user");

        store
            .create_project(&ProjectId(USER_PROJECT.into()), "per-user records", T0)
            .unwrap();
        let version = add_owner_entries(
            &store,
            &[
                (Catalog::Host(Host::ClaudeCode), "claude-opus-5-5"),
                (Catalog::Provider(Provider::OpenAi), "gpt-6-luna"),
            ],
        );
        assert_eq!(
            host_names(&store, Host::ClaudeCode).unwrap(),
            names(&["claude-opus-5-5"], version)
        );

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let options = crate::ledger::open::options();
        let newer = open(
            &home,
            baley_store_sqlite::Options {
                view_set_version: NonZeroU32::new(options.view_set_version.get() + 1).unwrap(),
                ..crate::ledger::open::options()
            },
        );
        newer
            .create_project(&ProjectId(USER_PROJECT.into()), "per-user records", T0)
            .unwrap();
        add_owner_entries(
            &newer,
            &[(Catalog::Host(Host::ClaudeCode), "claude-opus-5-5")],
        );
        drop(newer);
        let store = open(&home, crate::ledger::open::options());
        assert_eq!(host_names(&store, Host::ClaudeCode).unwrap(), names(&[], 0));
    }
}
