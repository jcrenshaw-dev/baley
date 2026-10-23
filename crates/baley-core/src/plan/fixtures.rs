//! The plan judge's test data, written out from design 0005 and the phase's
//! decisions, never from running the judge: one copy every test of the
//! module reads.

// Builders the later steps' tests read; removed once every one is read.
#![allow(dead_code)]

use std::num::NonZeroU32;

use serde_json::{Value, json};

use super::{
    ApprovedItem, CommittedStory, CurrentTruth, EvidenceKind, Plan, PlanFacts, PlanRefusal,
    judge_plan,
};

fn truth(
    id: &str,
    version: u32,
    trigger: &str,
    observer: &str,
    outcome: &str,
    human_verify: Option<&str>,
) -> CurrentTruth {
    CurrentTruth {
        id: id.into(),
        version,
        trigger: trigger.into(),
        observer: observer.into(),
        outcome: outcome.into(),
        human_verify: human_verify.map(str::to_owned),
    }
}

/// F1: committed stories 1 then 2, no approved items, size 0, no capacity.
pub(super) fn f1() -> PlanFacts {
    PlanFacts {
        stories: vec![
            CommittedStory {
                id: 1,
                truths: vec![
                    truth(
                        "t1",
                        1,
                        "a plan names a task file outside its lease",
                        "the session",
                        "lease, a valid refusal",
                        None,
                    ),
                    truth(
                        "t2",
                        2,
                        "the owner opens a plan preview",
                        "the owner",
                        "every task title in order",
                        Some("a browser"),
                    ),
                ],
            },
            CommittedStory {
                id: 2,
                truths: vec![truth(
                    "t1",
                    3,
                    "a session reads an approved plan",
                    "the session",
                    "each check level",
                    None,
                )],
            },
        ],
        approved: Vec::new(),
        size: 0,
        capacity: None,
    }
}

/// F0: F1 with both stories holding no truth.
pub(super) fn f0() -> PlanFacts {
    let mut facts = f1();
    for story in &mut facts.stories {
        story.truths.clear();
    }
    facts
}

/// F2: F1 with story 1 holding no truth.
pub(super) fn f2() -> PlanFacts {
    let mut facts = f1();
    facts.stories[0].truths.clear();
    facts
}

/// K1, a task.
pub(super) fn k1() -> Value {
    json!({"id": "k1", "title": "The judge", "files": ["src/judge.rs", "docs/plan.md"],
        "action": "Judge the plan whole.", "verify": ["cargo test judge"]})
}

/// C1, a unit check for story 1's `t1`.
pub(super) fn c1() -> Value {
    json!({"id": "c1", "story": 1, "truth": "t1", "truth_version": 1, "kind": "check",
        "spec": {"command": "cargo test lease",
            "expected": {"kind": "literal", "value": "lease"},
            "test": {"file": "src/judge.rs", "function": "outside_lease"},
            "setup": "a task file outside the lease", "call": "judge the plan",
            "level": "unit", "boundary": "the plan judge", "fakes": []},
        "reason": "a task file outside the lease accepted"})
}

/// C2, an integration check for story 1's `t2`.
pub(super) fn c2() -> Value {
    json!({"id": "c2", "story": 1, "truth": "t2", "truth_version": 2, "kind": "check",
        "spec": {"command": "cargo test preview",
            "expected": {"kind": "property", "value": "every task title in order"},
            "test": {"file": "src/preview.rs", "function": "titles_in_order"},
            "setup": "a plan of two tasks", "call": "render the preview",
            "level": "integration", "boundary": "the plan judge and the preview render",
            "fakes": ["the clock"]},
        "reason": "a task title dropped from the preview"})
}

/// O1, an observation for story 1's `t2`.
pub(super) fn o1() -> Value {
    json!({"id": "o1", "story": 1, "truth": "t2", "truth_version": 2, "kind": "observation",
        "spec": {"what": "every task title in order in the preview", "by": "the owner"},
        "reason": "the browser preview drops a task"})
}

/// C4: C1 for story 2's `t1`.
pub(super) fn c4() -> Value {
    let mut item = c1();
    item["id"] = json!("c4");
    item["story"] = json!(2);
    item["truth_version"] = json!(3);
    item["spec"]["command"] = json!("cargo test read");
    item
}

/// Ar, an artifact for story 1's `t1`.
pub(super) fn ar() -> Value {
    json!({"id": "a1", "story": 1, "truth": "t1", "truth_version": 1, "kind": "artifact",
        "spec": {"path": "src/judge.rs"}, "reason": "the judge file removed"})
}

/// L1, a link for story 1's `t1`.
pub(super) fn l1() -> Value {
    json!({"id": "l1", "story": 1, "truth": "t1", "truth_version": 1, "kind": "link",
        "spec": {"caller": "the judge", "callee": "the refusal", "value": "lease"},
        "reason": "the refusal names another code"})
}

/// Qa, a question with no dependency.
pub(super) fn qa() -> Value {
    json!({"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
        "depends_on": []})
}

/// Qb, a question depending on `q1`.
pub(super) fn qb() -> Value {
    json!({"id": "q2", "question": "Where does the ledger live?",
        "recommended": "Outside the checkout", "depends_on": ["q1"]})
}

/// P1, a plan for story 1.
pub(super) fn p1() -> Value {
    json!({"phase": 8, "stories": [1], "files": ["src/judge.rs"], "directories": ["docs"],
        "goal": "Judge a plan whole.", "context": "Pure core.", "notes": "",
        "tasks": [k1()], "evidence_map": [c1(), c2(), o1()], "questions": []})
}

/// P0: P1 serving no story and holding no evidence item.
pub(super) fn p0() -> Value {
    let mut plan = p1();
    plan["stories"] = json!([]);
    plan["evidence_map"] = json!([]);
    plan
}

/// P3: P1 for story 2, proved by C4.
pub(super) fn p3() -> Value {
    let mut plan = p1();
    plan["stories"] = json!([2]);
    plan["evidence_map"] = json!([c4()]);
    plan
}

/// F3: F1 with story 2's `t1` at version 1, so both stories hold a `t1` at
/// version 1.
pub(super) fn f3() -> PlanFacts {
    let mut facts = f1();
    facts.stories[1].truths[0].version = 1;
    facts
}

/// F1 with `approved` as the approved plans' items.
pub(super) fn under(approved: Vec<ApprovedItem>) -> PlanFacts {
    let mut facts = f1();
    facts.approved = approved;
    facts
}

/// F1 with capacity `capacity` and current size `size`.
pub(super) fn at(capacity: u32, size: u32) -> PlanFacts {
    let mut facts = f1();
    facts.capacity = Some(NonZeroU32::new(capacity).expect("a capacity above 0"));
    facts.size = size;
    facts
}

/// `item` as approved plan `plan` recorded it, its definition the item
/// whole.
pub(super) fn approved(plan: u32, item: Value) -> ApprovedItem {
    let text = |member: &str| item[member].as_str().expect("a text").to_owned();
    let number =
        |member: &str| u32::try_from(item[member].as_u64().expect("a number")).expect("a counter");
    let kind = match item["kind"].as_str() {
        Some("artifact") => EvidenceKind::Artifact,
        Some("link") => EvidenceKind::Link,
        Some("check") => EvidenceKind::Check,
        Some("observation") => EvidenceKind::Observation,
        other => panic!("no kind {other:?}"),
    };
    ApprovedItem {
        plan,
        id: text("id"),
        story: number("story"),
        truth: text("truth"),
        truth_version: number("truth_version"),
        kind,
        definition: item,
    }
}

/// A1: plan 1's C1 with reason `another reason`.
pub(super) fn a1() -> Vec<ApprovedItem> {
    vec![approved(
        1,
        with(c1(), "", "reason", json!("another reason")),
    )]
}

/// A2: plan 1's C1 exactly.
pub(super) fn a2() -> Vec<ApprovedItem> {
    vec![approved(1, c1())]
}

/// C3: C1 with id `c3`, a second check for story 1's `t1`.
pub(super) fn c3() -> Value {
    let mut item = c1();
    item["id"] = json!("c3");
    item["spec"]["command"] = json!("cargo test lease_again");
    item
}

/// C5: C4 for version 1 of story 2's `t1`.
pub(super) fn c5() -> Value {
    with(c4(), "", "truth_version", json!(1))
}

/// C6: C1 with id `c6`, a third check for story 1's `t1`.
pub(super) fn c6() -> Value {
    let mut item = c1();
    item["id"] = json!("c6");
    item["spec"]["command"] = json!("cargo test lease_third");
    item
}

/// K3: K1 with id `k3`.
pub(super) fn k3() -> Value {
    with(k1(), "", "id", json!("k3"))
}

/// P2: P1 for stories 1 and 2, proved by C1, C2, O1 and C4.
pub(super) fn p2() -> Value {
    let mut plan = p1();
    plan["stories"] = json!([1, 2]);
    plan["evidence_map"] = json!([c1(), c2(), o1(), c4()]);
    plan
}

/// P4: P1 with tasks K1 and K3.
pub(super) fn p4() -> Value {
    with(p1(), "", "tasks", json!([k1(), k3()]))
}

/// `plan` with `item` added at the end of its evidence map.
pub(super) fn plus(mut plan: Value, item: Value) -> Value {
    plan["evidence_map"]
        .as_array_mut()
        .expect("an evidence map")
        .push(item);
    plan
}

/// `plan` without the evidence items whose ids are `ids`.
pub(super) fn dropping(mut plan: Value, ids: &[&str]) -> Value {
    plan["evidence_map"]
        .as_array_mut()
        .expect("an evidence map")
        .retain(|item| !ids.contains(&item["id"].as_str().expect("an id")));
    plan
}

/// `plan` with the evidence item whose id is `id` replaced by `item`.
pub(super) fn replacing(mut plan: Value, id: &str, item: Value) -> Value {
    let map = plan["evidence_map"]
        .as_array_mut()
        .expect("an evidence map");
    let at = map
        .iter()
        .position(|entry| entry["id"] == id)
        .expect("the item is present");
    map[at] = item;
    plan
}

/// `value` with member `member` of the object at `pointer` set to `to`,
/// added when absent.
pub(super) fn with(mut value: Value, pointer: &str, member: &str, to: Value) -> Value {
    value
        .pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .expect("an object at the pointer")
        .insert(member.into(), to);
    value
}

/// `value` with member `member` of the object at `pointer` removed.
pub(super) fn without(mut value: Value, pointer: &str, member: &str) -> Value {
    value
        .pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .expect("an object at the pointer")
        .remove(member)
        .expect("the member is present");
    value
}

/// The `typed-content` refusal naming `path`.
pub(super) fn typed(path: &str) -> PlanRefusal {
    PlanRefusal::TypedContent {
        path: Some(path.into()),
    }
}

/// The refusal of `plan` under `facts`.
pub(super) fn refusal(plan: &Value, facts: &PlanFacts) -> PlanRefusal {
    judge_plan(plan, facts).expect_err("refused")
}

/// The accepted plan of `plan` under `facts`.
pub(super) fn accepted(plan: &Value, facts: &PlanFacts) -> Plan {
    judge_plan(plan, facts).expect("accepted")
}
