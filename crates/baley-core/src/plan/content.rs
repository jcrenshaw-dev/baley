//! Step (1) of the plan judge: the plan's own members read in a fixed order
//! (PLN-R11, D-03), the first fault answering `typed-content` with its
//! path. No text is trimmed or rewritten.

use serde_json::{Map, Value};

use super::judge::{Plan, PlanRefusal};

/// Reads a plan into the accepted plan, each member in the judge's order:
/// `phase`, `stories`, `files`, `directories`, `goal`, `context`, `notes`,
/// `tasks`, `evidence_map`, then the presence of `questions`, whose value
/// step (2) judges.
pub(super) fn read(value: &Value) -> Result<Plan, PlanRefusal> {
    let Value::Object(plan) = value else {
        return Err(PlanRefusal::TypedContent { path: None });
    };
    let phase = whole(plan.get("phase")).ok_or_else(|| PlanRefusal::typed("phase"))?;
    let stories = list(plan, "stories")?
        .iter()
        .enumerate()
        .map(|(n, story)| {
            whole(Some(story)).ok_or_else(|| PlanRefusal::typed(format!("stories[{n}]")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let files = list(plan, "files")?.clone();
    let directories = list(plan, "directories")?.clone();
    let goal = text(plan, "goal")?;
    let context = text(plan, "context")?;
    let notes = text(plan, "notes")?;
    let tasks = list(plan, "tasks")?.clone();
    let evidence_map = list(plan, "evidence_map")?.clone();
    let questions = plan
        .get("questions")
        .ok_or_else(|| PlanRefusal::typed("questions"))?
        .clone();
    Ok(Plan {
        phase,
        stories,
        files,
        directories,
        goal,
        context,
        notes,
        tasks,
        evidence_map,
        questions,
    })
}

/// A whole number, or `None` when the value is missing or anything else.
fn whole(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64)
}

/// The list member `name`, refused naming it when missing or not a list.
fn list<'a>(plan: &'a Map<String, Value>, name: &str) -> Result<&'a Vec<Value>, PlanRefusal> {
    match plan.get(name) {
        Some(Value::Array(entries)) => Ok(entries),
        _ => Err(PlanRefusal::typed(name)),
    }
}

/// The text member `name` as written, blank allowed, refused naming it when
/// missing or not text.
fn text(plan: &Map<String, Value>, name: &str) -> Result<String, PlanRefusal> {
    match plan.get(name) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(PlanRefusal::typed(name)),
    }
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{accepted, f1, p0, p1, refusal, typed, with, without};

    #[test]
    fn a_plan_member_missing_or_of_another_type_accepted_is_caught() {
        let facts = f1();
        assert_eq!(
            refusal(&json!([p1()]), &facts),
            PlanRefusal::TypedContent { path: None }
        );
        for member in [
            "phase",
            "stories",
            "files",
            "directories",
            "goal",
            "context",
            "notes",
            "tasks",
            "evidence_map",
            "questions",
        ] {
            assert_eq!(
                refusal(&without(p1(), "", member), &facts),
                typed(member),
                "without {member}"
            );
        }
        let cases = [
            (with(p1(), "", "phase", json!(8.5)), "phase"),
            (with(p1(), "", "stories", json!("1")), "stories"),
            (with(p1(), "", "stories", json!(["1"])), "stories[0]"),
            (with(p1(), "", "goal", json!(3)), "goal"),
            (with(p1(), "", "tasks", json!({})), "tasks"),
            (with(p1(), "", "evidence_map", json!(null)), "evidence_map"),
        ];
        for (plan, path) in cases {
            assert_eq!(refusal(&plan, &facts), typed(path), "{plan}");
        }
    }

    #[test]
    fn an_accepted_plan_trimmed_or_an_empty_member_refused_is_caught() {
        let facts = f1();
        let plan = with(
            with(p1(), "", "goal", json!(" Judge a plan whole. ")),
            "",
            "context",
            json!(""),
        );
        let plan = accepted(&plan, &facts);
        assert_eq!(plan.goal(), " Judge a plan whole. ");
        assert_eq!(plan.context(), "");
        assert!(accepted(&p0(), &facts).stories().is_empty());
    }
}
