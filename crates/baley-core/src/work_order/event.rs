//! The work-order events: their types, their registration, their streams
//! and their payloads (design 0001 Streams and Payloads, design 0012).
//!
//! An order's failure goes on the stream its issue went on. No payload holds
//! a caller or a time: the envelope carries both, hashed with the event.

use baley_store::{InstructionEvidence, PayloadRef};
use serde_json::{Value, json};

use crate::policy::{Role, Rung};
use crate::registry::{Registry, RegistryError};
use crate::scope::ROADMAP_STREAM;

/// One work order issued, on its subject's stream (`roadmap` for the start):
/// `{id, request, parent, role, subject, attempt, route, agent,
/// instructions, inputs}`. `request` is the id of its chain's first order,
/// `parent` the failed order it retries or null, `instructions` a list of
/// `{identity, version, hash}` and `inputs` a list of `{name, body}`, each
/// body a reference to a `record` payload.
pub const WORK_ORDER_ISSUED: &str = "work_order.issued";
/// The current `work_order.issued` payload version.
pub const WORK_ORDER_ISSUED_VERSION: u32 = 1;
/// One work order recorded as failed, on the stream its issue went on:
/// `{id, reason}`.
pub const WORK_ORDER_FAILED: &str = "work_order.failed";
/// The current `work_order.failed` payload version.
pub const WORK_ORDER_FAILED_VERSION: u32 = 1;

/// The subject of the start planner's order.
pub const START_SUBJECT: &str = "start";

/// Registers the two work-order types at version 1 with no upcasters.
pub fn register_work_order_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(WORK_ORDER_ISSUED, WORK_ORDER_ISSUED_VERSION, [])?;
    registry.register(WORK_ORDER_FAILED, WORK_ORDER_FAILED_VERSION, [])
}

/// The stream an order of `subject` is recorded on: `roadmap` for the
/// start. Other subjects get theirs with the builds that issue them.
pub fn order_stream(subject: &str) -> Option<&'static str> {
    (subject == START_SUBJECT).then_some(ROADMAP_STREAM)
}

/// The agent stub a role runs at a rung, `bal-<role>-<rung>`. Install places
/// a stub under every such name, so an order always names one that exists.
pub fn agent_name(role: Role, rung: Rung) -> String {
    format!("bal-{}-{}", role.name(), rung.name())
}

/// One input an order names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderInput {
    /// The input's name, such as `brief`.
    pub name: String,
    /// The reference to the `record` payload holding its body.
    pub body: PayloadRef,
}

/// Everything `work_order.issued` records for one order.
#[derive(Debug, Clone, Copy)]
pub struct IssuedOrder<'a> {
    /// The order's id, a lowercase UUID.
    pub id: &'a str,
    /// The id of its chain's first order, which is its own id at attempt 1.
    pub request: &'a str,
    /// The failed order it retries, if any.
    pub parent: Option<&'a str>,
    /// The role it is for.
    pub role: Role,
    /// What it is about, such as `start`.
    pub subject: &'a str,
    /// The attempt, from 1.
    pub attempt: u32,
    /// The route record's JSON.
    pub route: &'a Value,
    /// The agent stub's name.
    pub agent: &'a str,
    /// The instructions it carries, in order.
    pub instructions: &'a [InstructionEvidence],
    /// The inputs it carries, in order.
    pub inputs: &'a [OrderInput],
}

/// The payload of `work_order.issued` for `order`.
pub fn issued_payload(order: &IssuedOrder<'_>) -> Value {
    let instructions: Vec<Value> = order
        .instructions
        .iter()
        .map(|entry| {
            json!({
                "identity": entry.identity(),
                "version": entry.version(),
                "hash": entry.hash(),
            })
        })
        .collect();
    let inputs: Vec<Value> = order
        .inputs
        .iter()
        .map(|input| json!({"name": input.name, "body": input.body.to_value()}))
        .collect();
    json!({
        "id": order.id,
        "request": order.request,
        "parent": order.parent,
        "role": order.role.name(),
        "subject": order.subject,
        "attempt": order.attempt,
        "route": order.route,
        "agent": order.agent,
        "instructions": instructions,
        "inputs": inputs,
    })
}

/// The payload of `work_order.failed`: the order and why it failed.
pub fn failed_payload(id: &str, reason: &str) -> Value {
    json!({"id": id, "reason": reason})
}

#[cfg(test)]
mod tests {
    //! Expected payloads are written out from the phase's decisions and the
    //! plan's test data, never from running this code.

    use std::collections::BTreeSet;

    use baley_store::{EventSchema, Hash, RetentionClass};

    use super::*;

    const I0: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const I1: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";

    fn brief() -> Vec<OrderInput> {
        vec![OrderInput {
            name: "brief".into(),
            body: PayloadRef {
                hash: Hash([0xab; 32]),
                bytes: 300,
                class: RetentionClass::Record,
            },
        }]
    }

    fn evidence() -> Vec<InstructionEvidence> {
        vec![InstructionEvidence::new("bal-start-planner", "1", &"c".repeat(64)).unwrap()]
    }

    #[test]
    fn a_work_order_event_left_unregistered_or_at_another_version_is_caught() {
        let mut registry = Registry::new();
        register_work_order_events(&mut registry).unwrap();
        for name in ["work_order.issued", "work_order.failed"] {
            assert_eq!(registry.current_version(name), Some(1), "{name}");
            assert!(!registry.reads(name, 2), "{name}");
        }
        assert!(register_work_order_events(&mut registry).is_err());
    }

    #[test]
    fn an_issued_payload_missing_a_member_or_carrying_another_is_caught() {
        let route = json!({"rung": "high"});
        let (instructions, inputs) = (evidence(), brief());
        let o1 = IssuedOrder {
            id: I0,
            request: I0,
            parent: None,
            role: Role::Planner,
            subject: "start",
            attempt: 1,
            route: &route,
            agent: "bal-planner-high",
            instructions: &instructions,
            inputs: &inputs,
        };
        let expected = json!({"id": I0, "request": I0, "parent": null, "role": "planner",
            "subject": "start", "attempt": 1, "route": {"rung": "high"},
            "agent": "bal-planner-high",
            "instructions": [{"identity": "bal-start-planner", "version": "1",
                "hash": "c".repeat(64)}],
            "inputs": [{"name": "brief",
                "body": {"payload": "ab".repeat(32), "bytes": 300, "class": "record"}}]});
        assert_eq!(issued_payload(&o1), expected);

        let o2 = IssuedOrder {
            id: I1,
            parent: Some(I0),
            attempt: 2,
            agent: "bal-planner-xhigh",
            ..o1
        };
        let mut retry = expected;
        retry["id"] = json!(I1);
        retry["parent"] = json!(I0);
        retry["attempt"] = json!(2);
        retry["agent"] = json!("bal-planner-xhigh");
        assert_eq!(issued_payload(&o2), retry);
        assert_eq!(issued_payload(&o2)["request"], I0);
    }

    #[test]
    fn a_failed_payload_without_its_order_or_reason_is_caught() {
        assert_eq!(
            failed_payload(I0, "the worker could not launch"),
            json!({"id": I0, "reason": "the worker could not launch"})
        );
    }

    #[test]
    fn a_start_order_off_the_roadmap_stream_or_an_agent_off_the_name_rule_is_caught() {
        assert_eq!(order_stream("start"), Some("roadmap"));
        let names: BTreeSet<String> = Role::ALL
            .into_iter()
            .flat_map(|role| Rung::ALL.map(|rung| agent_name(role, rung)))
            .collect();
        assert_eq!(names.len(), 30);
        for name in [
            "bal-planner-high",
            "bal-analyzer-medium",
            "bal-checker-low",
            "bal-reviewer-max",
        ] {
            assert!(names.contains(name), "{name}");
        }
    }
}
