//! A plan's tasks (design 0005 PLN-R11): each task's grammar, read at
//! `tasks`'s place in the member walk, and step (3), a task file outside
//! the lease (PLN-R13, D-11). Task prose is never read for paths.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::content::first_unknown;
use super::judge::PlanRefusal;
use super::lease::inside_lease;

/// A task's members, in the order they are judged.
const MEMBERS: [&str; 5] = ["id", "title", "files", "action", "verify"];

/// One task of an accepted plan, its texts kept exactly as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Its id, unique within the plan.
    pub id: String,
    /// Its title.
    pub title: String,
    /// The files it touches, in submitted order.
    pub files: Vec<String>,
    /// What it does.
    pub action: String,
    /// The commands that verify it, at least one, none blank.
    pub verify: Vec<String>,
}

/// Reads a plan's tasks in submitted order, each whole before the next.
pub(super) fn read(tasks: &[Value]) -> Result<Vec<Task>, PlanRefusal> {
    let mut seen = HashSet::new();
    tasks
        .iter()
        .enumerate()
        .map(|(n, entry)| task(n, entry, &mut seen))
        .collect()
}

/// Step (3): the first task file, in task order and file order, outside
/// the lease of `files` and `directories` is refused `lease`.
pub(super) fn within_lease(
    tasks: &[Task],
    files: &[String],
    directories: &[String],
) -> Result<(), PlanRefusal> {
    for (position, task) in tasks.iter().enumerate() {
        if let Some(file) = task
            .files
            .iter()
            .find(|file| !inside_lease(file, files, directories))
        {
            return Err(PlanRefusal::Lease {
                position,
                id: task.id.clone(),
                file: file.clone(),
            });
        }
    }
    Ok(())
}

/// One task read: an unknown member first, the first by name, then each
/// member in order, every fault `typed-content` named by its path.
fn task(n: usize, entry: &Value, seen: &mut HashSet<String>) -> Result<Task, PlanRefusal> {
    let at = |member: &str| PlanRefusal::typed(format!("tasks[{n}].{member}"));
    let Value::Object(task) = entry else {
        return Err(PlanRefusal::typed(format!("tasks[{n}]")));
    };
    if let Some(unknown) = first_unknown(task, &MEMBERS) {
        return Err(at(unknown));
    }
    let id = text(task, "id")
        .filter(|id| !id.trim().is_empty() && seen.insert(id.clone()))
        .ok_or_else(|| at("id"))?;
    let title = text(task, "title").ok_or_else(|| at("title"))?;
    let entry = |name: &'static str| {
        move |fault: Option<usize>| match fault {
            None => at(name),
            Some(m) => at(&format!("{name}[{m}]")),
        }
    };
    let files = texts(task, "files", |_| true).map_err(entry("files"))?;
    let action = text(task, "action").ok_or_else(|| at("action"))?;
    // PLN-R11: one or more verify commands, none blank.
    let verify =
        texts(task, "verify", |command| !command.trim().is_empty()).map_err(entry("verify"))?;
    if verify.is_empty() {
        return Err(at("verify"));
    }
    Ok(Task {
        id,
        title,
        files,
        action,
        verify,
    })
}

/// The text member `name` as written, or `None` when missing or not text.
fn text(task: &Map<String, Value>, name: &str) -> Option<String> {
    task.get(name).and_then(Value::as_str).map(str::to_owned)
}

/// The list of texts `name`, each passing `keep`. The error is `None` when
/// the member is missing or not a list, else the first failing position.
fn texts(
    task: &Map<String, Value>,
    name: &str,
    keep: impl Fn(&str) -> bool,
) -> Result<Vec<String>, Option<usize>> {
    let Some(Value::Array(entries)) = task.get(name) else {
        return Err(None);
    };
    entries
        .iter()
        .enumerate()
        .map(|(m, entry)| {
            entry
                .as_str()
                .filter(|entry| keep(entry))
                .map(str::to_owned)
                .ok_or(Some(m))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::{Value, json};

    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{accepted, f1, k1, p1, refusal, typed, with, without};

    /// P1 with K1's member `member` set to `to`.
    fn k1_with(member: &str, to: Value) -> Value {
        with(p1(), "/tasks/0", member, to)
    }

    fn lease(position: usize, id: &str, file: &str) -> PlanRefusal {
        PlanRefusal::Lease {
            position,
            id: id.into(),
            file: file.into(),
        }
    }

    #[test]
    fn a_task_with_a_bad_member_accepted_is_caught() {
        let facts = f1();
        let listed = with(p1(), "", "tasks", json!(["k1"]));
        let cases = [
            (k1_with("owner", json!("Ada")), "tasks[0].owner"),
            (without(p1(), "/tasks/0", "verify"), "tasks[0].verify"),
            (without(p1(), "/tasks/0", "action"), "tasks[0].action"),
            (k1_with("verify", json!([])), "tasks[0].verify"),
            (
                k1_with("verify", json!(["cargo test judge", "  "])),
                "tasks[0].verify[1]",
            ),
            (
                k1_with("files", json!(["src/judge.rs", 3])),
                "tasks[0].files[1]",
            ),
            (k1_with("title", json!(3)), "tasks[0].title"),
            (listed.clone(), "tasks[0]"),
            (without(listed, "", "questions"), "tasks[0]"),
        ];
        for (plan, path) in cases {
            assert_eq!(refusal(&plan, &facts), typed(path), "{plan}");
        }
    }

    #[test]
    fn a_task_id_blank_or_repeated_accepted_is_caught() {
        let facts = f1();
        let twice = with(p1(), "", "tasks", json!([k1(), k1()]));
        assert_eq!(refusal(&twice, &facts), typed("tasks[1].id"));
        assert_eq!(
            refusal(&k1_with("id", json!(" ")), &facts),
            typed("tasks[0].id")
        );
    }

    #[test]
    fn a_task_file_outside_the_lease_named_wrongly_is_caught() {
        let facts = f1();
        assert_eq!(
            refusal(&k1_with("files", json!(["docs"])), &facts),
            lease(0, "k1", "docs")
        );
        let mut k2 = k1();
        k2["id"] = json!("k2");
        k2["files"] = json!(["docs/../src/x.rs"]);
        let two = with(p1(), "", "tasks", json!([k1(), k2]));
        assert_eq!(refusal(&two, &facts), lease(1, "k2", "docs/../src/x.rs"));
        accepted(
            &k1_with("files", json!(["src/judge.rs", "docs/a/b.md"])),
            &facts,
        );
    }

    #[test]
    fn an_empty_task_list_or_task_files_refused_is_caught() {
        let facts = f1();
        accepted(&with(p1(), "", "tasks", json!([])), &facts);
        accepted(&k1_with("files", json!([])), &facts);
    }
}
