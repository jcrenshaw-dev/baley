//! A plan's evidence items (design 0005 section 6, PLN-R12, D-01, D-12):
//! each item's grammar, read at `evidence_map`'s place in the member walk,
//! its spec judged by its kind. Every fault is `typed-content` named by its
//! full path, such as `evidence_map[0].spec.expected.kind`.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::content::{counter, first_unknown};
use super::judge::{EvidenceKind, PlanRefusal};

/// An item's members, in the order they are judged.
const ITEM: [&str; 7] = [
    "id",
    "story",
    "truth",
    "truth_version",
    "kind",
    "spec",
    "reason",
];
/// An artifact spec's members, exactly one of which it holds.
const ARTIFACT: [&str; 2] = ["path", "record"];
/// A link spec's members, in the order they are judged.
const LINK: [&str; 3] = ["caller", "callee", "value"];
/// An observation spec's members, in the order they are judged.
const OBSERVATION: [&str; 2] = ["what", "by"];
/// A check spec's members, in the order they are judged (D-12).
const CHECK: [&str; 8] = [
    "command", "expected", "test", "setup", "call", "level", "boundary", "fakes",
];
/// A check's expected value's members.
const EXPECTED: [&str; 2] = ["kind", "value"];
/// A check's test's members.
const TEST: [&str; 2] = ["file", "function"];

/// One evidence item of an accepted plan, its texts kept exactly as
/// written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceItem {
    /// Its id, unique within the plan.
    pub id: String,
    /// The story of the truth it serves (D-01).
    pub story: u32,
    /// The id of the truth it serves, unique within that story.
    pub truth: String,
    /// The version of the truth it serves.
    pub truth_version: u32,
    /// What it holds, by its kind.
    pub spec: Spec,
    /// The defect it catches.
    pub reason: String,
}

impl EvidenceItem {
    /// The item's kind, as its spec has it.
    pub fn kind(&self) -> EvidenceKind {
        match self.spec {
            Spec::Artifact(_) => EvidenceKind::Artifact,
            Spec::Link(_) => EvidenceKind::Link,
            Spec::Check(_) => EvidenceKind::Check,
            Spec::Observation(_) => EvidenceKind::Observation,
        }
    }
}

/// An evidence item's spec, typed by the item's kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    /// A file or record that must exist.
    Artifact(Artifact),
    /// A value that must pass from a caller to a callee.
    Link(Link),
    /// A command whose result proves the truth.
    Check(Check),
    /// Something a named person must see.
    Observation(Observation),
}

/// What an artifact names: a path or a record, never both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artifact {
    /// A file's path.
    Path(String),
    /// A record's name.
    Record(String),
}

/// A link's spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// What passes the value.
    pub caller: String,
    /// What receives it.
    pub callee: String,
    /// The value, which the served truth names.
    pub value: String,
}

/// An observation's spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// What is to be seen.
    pub what: String,
    /// By whom.
    pub by: String,
}

/// A check's spec (D-12, ADR 0042).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// The command that runs it.
    pub command: String,
    /// What it expects.
    pub expected: Expected,
    /// The test it runs.
    pub test: CheckTest,
    /// The state it sets up.
    pub setup: String,
    /// The call it makes.
    pub call: String,
    /// Whether it exercises one unit or several.
    pub level: Level,
    /// The unit or units it exercises.
    pub boundary: String,
    /// Every seam it fakes, in submitted order.
    pub fakes: Vec<String>,
}

/// A check's expected value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// A literal the check's output must match.
    Literal(String),
    /// A property the check's output must hold.
    Property(String),
}

/// The test a check runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckTest {
    /// The test's file.
    pub file: String,
    /// The test's function.
    pub function: String,
}

/// A check's level (ADR 0042).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// It exercises one unit.
    Unit,
    /// It exercises several units across a mocked boundary.
    Integration,
}

/// Reads a plan's evidence items in submitted order, each whole before the
/// next.
pub(super) fn read(items: &[Value]) -> Result<Vec<EvidenceItem>, PlanRefusal> {
    let mut seen = HashSet::new();
    items
        .iter()
        .enumerate()
        .map(|(n, entry)| item(&format!("evidence_map[{n}]"), entry, &mut seen))
        .collect()
}

/// One item read: an unknown member first, the first by name, then each
/// member in order.
fn item(
    path: &str,
    entry: &Value,
    seen: &mut HashSet<String>,
) -> Result<EvidenceItem, PlanRefusal> {
    let item = object(path, entry, &ITEM)?;
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty() && seen.insert((*id).to_owned()))
        .ok_or_else(|| typed(path, "id"))?
        .to_owned();
    let story = counter(item.get("story")).ok_or_else(|| typed(path, "story"))?;
    let truth = text(path, item, "truth")?;
    let truth_version =
        counter(item.get("truth_version")).ok_or_else(|| typed(path, "truth_version"))?;
    let kind = item.get("kind").and_then(Value::as_str);
    let at = format!("{path}.spec");
    let spec = item.get("spec").unwrap_or(&Value::Null);
    let spec = match kind {
        Some("artifact") => Spec::Artifact(artifact(&at, spec)?),
        Some("link") => Spec::Link(link(&at, spec)?),
        Some("check") => Spec::Check(check(&at, spec)?),
        Some("observation") => Spec::Observation(observation(&at, spec)?),
        _ => return Err(typed(path, "kind")),
    };
    let reason = text(path, item, "reason")?;
    Ok(EvidenceItem {
        id,
        story,
        truth,
        truth_version,
        spec,
        reason,
    })
}

/// An artifact's spec: exactly one of `path` or `record`, as text.
fn artifact(path: &str, value: &Value) -> Result<Artifact, PlanRefusal> {
    let spec = object(path, value, &ARTIFACT)?;
    if spec.len() != 1 {
        return Err(PlanRefusal::typed(path));
    }
    if spec.contains_key("path") {
        Ok(Artifact::Path(text(path, spec, "path")?))
    } else {
        Ok(Artifact::Record(text(path, spec, "record")?))
    }
}

/// A link's spec.
fn link(path: &str, value: &Value) -> Result<Link, PlanRefusal> {
    let spec = object(path, value, &LINK)?;
    Ok(Link {
        caller: text(path, spec, "caller")?,
        callee: text(path, spec, "callee")?,
        value: text(path, spec, "value")?,
    })
}

/// An observation's spec.
fn observation(path: &str, value: &Value) -> Result<Observation, PlanRefusal> {
    let spec = object(path, value, &OBSERVATION)?;
    Ok(Observation {
        what: text(path, spec, "what")?,
        by: text(path, spec, "by")?,
    })
}

/// A check's spec, each member in D-12's order.
fn check(path: &str, value: &Value) -> Result<Check, PlanRefusal> {
    let spec = object(path, value, &CHECK)?;
    let command = text(path, spec, "command")?;
    let at = format!("{path}.expected");
    let expected = object(&at, spec.get("expected").unwrap_or(&Value::Null), &EXPECTED)?;
    let kind = expected.get("kind").and_then(Value::as_str);
    if !matches!(kind, Some("literal" | "property")) {
        return Err(typed(&at, "kind"));
    }
    let value = text(&at, expected, "value")?;
    let expected = if kind == Some("literal") {
        Expected::Literal(value)
    } else {
        Expected::Property(value)
    };
    let at = format!("{path}.test");
    let test = object(&at, spec.get("test").unwrap_or(&Value::Null), &TEST)?;
    let test = CheckTest {
        file: text(&at, test, "file")?,
        function: text(&at, test, "function")?,
    };
    let setup = text(path, spec, "setup")?;
    let call = text(path, spec, "call")?;
    // ADR 0042: a check without a level, or with another, is out of grammar.
    let level = match spec.get("level").and_then(Value::as_str) {
        Some("unit") => Level::Unit,
        Some("integration") => Level::Integration,
        _ => return Err(typed(path, "level")),
    };
    let boundary = text(path, spec, "boundary")?;
    let Some(Value::Array(fakes)) = spec.get("fakes") else {
        return Err(typed(path, "fakes"));
    };
    let fakes = fakes
        .iter()
        .enumerate()
        .map(|(m, fake)| {
            fake.as_str()
                .map(str::to_owned)
                .ok_or_else(|| typed(path, &format!("fakes[{m}]")))
        })
        .collect::<Result<_, _>>()?;
    Ok(Check {
        command,
        expected,
        test,
        setup,
        call,
        level,
        boundary,
        fakes,
    })
}

/// The object at `path` with no member outside `members`, else refused
/// naming `path`, or the first unknown member by name.
fn object<'a>(
    path: &str,
    value: &'a Value,
    members: &[&str],
) -> Result<&'a Map<String, Value>, PlanRefusal> {
    let Value::Object(object) = value else {
        return Err(PlanRefusal::typed(path));
    };
    match first_unknown(object, members) {
        Some(unknown) => Err(typed(path, unknown)),
        None => Ok(object),
    }
}

/// The text member `name` of the object at `path` as written, blank
/// allowed, else refused naming it.
fn text(path: &str, object: &Map<String, Value>, name: &str) -> Result<String, PlanRefusal> {
    object
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| typed(path, name))
}

/// The `typed-content` refusal naming member `name` of the object at
/// `path`.
fn typed(path: &str, name: &str) -> PlanRefusal {
    PlanRefusal::typed(format!("{path}.{name}"))
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::{Value, json};

    use super::*;
    use crate::plan::fixtures::{accepted, ar, f1, l1, p1, refusal, typed, with, without};

    /// P1 with member `member` of the object at `pointer` within C1 set to
    /// `to`.
    fn c1_with(pointer: &str, member: &str, to: Value) -> Value {
        with(p1(), &format!("/evidence_map/0{pointer}"), member, to)
    }

    /// P1 with `item` added after O1.
    fn plus(item: Value) -> Value {
        let mut plan = p1();
        plan["evidence_map"].as_array_mut().unwrap().push(item);
        plan
    }

    #[test]
    fn an_evidence_item_with_a_bad_member_accepted_is_caught() {
        let facts = f1();
        let listed = with(p1(), "", "evidence_map", json!(["c1"]));
        let cases = [
            (c1_with("", "owner", json!("Ada")), "evidence_map[0].owner"),
            (
                without(p1(), "/evidence_map/0", "story"),
                "evidence_map[0].story",
            ),
            (c1_with("", "story", json!(0)), "evidence_map[0].story"),
            (
                c1_with("", "truth_version", json!("1")),
                "evidence_map[0].truth_version",
            ),
            (
                without(p1(), "/evidence_map/0", "truth_version"),
                "evidence_map[0].truth_version",
            ),
            (c1_with("", "truth", json!(1)), "evidence_map[0].truth"),
            (c1_with("", "kind", json!("test")), "evidence_map[0].kind"),
            (
                without(p1(), "/evidence_map/0", "reason"),
                "evidence_map[0].reason",
            ),
            (c1_with("", "spec", json!("x")), "evidence_map[0].spec"),
            (listed.clone(), "evidence_map[0]"),
            (without(listed, "", "questions"), "evidence_map[0]"),
        ];
        for (plan, path) in cases {
            assert_eq!(refusal(&plan, &facts), typed(path), "{plan}");
        }
    }

    #[test]
    fn an_item_id_blank_or_repeated_in_one_plan_accepted_is_caught() {
        let facts = f1();
        assert_eq!(
            refusal(&c1_with("", "id", json!(" ")), &facts),
            typed("evidence_map[0].id")
        );
        assert_eq!(
            refusal(&with(p1(), "/evidence_map/1", "id", json!("c1")), &facts),
            typed("evidence_map[1].id")
        );
    }

    #[test]
    fn a_check_spec_out_of_grammar_accepted_is_caught() {
        let facts = f1();
        let cases = [
            (
                c1_with("/spec", "owner", json!("Ada")),
                "evidence_map[0].spec.owner",
            ),
            (
                c1_with("/spec/expected", "kind", json!("exact")),
                "evidence_map[0].spec.expected.kind",
            ),
            (
                without(p1(), "/evidence_map/0/spec/expected", "value"),
                "evidence_map[0].spec.expected.value",
            ),
            (
                without(p1(), "/evidence_map/0/spec/test", "function"),
                "evidence_map[0].spec.test.function",
            ),
            (
                c1_with("/spec", "fakes", json!("the clock")),
                "evidence_map[0].spec.fakes",
            ),
            (
                c1_with("/spec", "fakes", json!(["the clock", 3])),
                "evidence_map[0].spec.fakes[1]",
            ),
            (
                c1_with("/spec", "setup", json!(null)),
                "evidence_map[0].spec.setup",
            ),
        ];
        for (plan, path) in cases {
            assert_eq!(refusal(&plan, &facts), typed(path), "{plan}");
        }
    }

    #[test]
    fn an_artifact_link_or_observation_spec_out_of_grammar_accepted_is_caught() {
        let facts = f1();
        let plan = accepted(&plus(ar()), &facts);
        assert_eq!(
            plan.evidence_map()[3].spec,
            Spec::Artifact(Artifact::Path("src/judge.rs".into()))
        );
        let ar_with = |spec: Value| with(plus(ar()), "/evidence_map/3", "spec", spec);
        let plan = accepted(&ar_with(json!({"record": "plan.approved"})), &facts);
        assert_eq!(
            plan.evidence_map()[3].spec,
            Spec::Artifact(Artifact::Record("plan.approved".into()))
        );
        assert_eq!(
            refusal(&ar_with(json!({})), &facts),
            typed("evidence_map[3].spec")
        );
        assert_eq!(
            refusal(
                &ar_with(json!({"path": "src/judge.rs", "record": "plan.approved"})),
                &facts
            ),
            typed("evidence_map[3].spec")
        );
        assert_eq!(
            refusal(
                &without(plus(l1()), "/evidence_map/3/spec", "value"),
                &facts
            ),
            typed("evidence_map[3].spec.value")
        );
        let o1_with = with(
            p1(),
            "/evidence_map/2",
            "spec",
            json!({"what": "every task title", "who": "the owner"}),
        );
        assert_eq!(refusal(&o1_with, &facts), typed("evidence_map[2].spec.who"));
    }
}
