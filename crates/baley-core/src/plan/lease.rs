//! The lease: the files and directories a plan's tasks may touch, and the
//! one path rule every lease entry and every lease check reads (design 0005
//! PLN-R11, D-05, D-11).

use std::collections::HashSet;

use serde_json::Value;

use super::judge::PlanRefusal;

/// The most declarations one lease holds, files and directories counted
/// together (PLN-R11).
const LEASE_LIMIT: usize = 256;

/// Whether `path` is a plain relative path: not empty, not starting with
/// `/`, holding no `\`, and split on `/` with no empty, `.` or `..` part,
/// so a trailing separator and `//` are refused. Every lease entry, file or
/// directory, is one (D-05), and every check of a path against a lease
/// reads this rule. It matches `safe_relative_path` in the binary's patch
/// code.
pub fn plain_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
}

/// Whether `path` lies inside the lease of `files` and `directories`: it
/// is a plain relative path and either equals a declared file or has every
/// part of a declared directory as its leading parts and at least one part
/// more, compared part by part. A directory is not inside itself, and a
/// file is never taken as a directory.
pub fn inside_lease(path: &str, files: &[String], directories: &[String]) -> bool {
    if !plain_relative_path(path) {
        return false;
    }
    if files.iter().any(|file| file == path) {
        return true;
    }
    let parts: Vec<&str> = path.split('/').collect();
    directories.iter().any(|directory| {
        let leading: Vec<&str> = directory.split('/').collect();
        parts.len() > leading.len() && parts.starts_with(&leading)
    })
}

/// Reads a plan's lease from its `files` and `directories` lists, the
/// first fault answering `typed-content`: each entry of `files`, then of
/// `directories`, that is not text passing the path rule; then an entry
/// equal to an earlier one, both lists scanned as one, named at its later
/// position; then the 257th declaration, files first; then a lease with no
/// file and no directory, named `files` (D-11).
pub(super) fn read(
    files: &[Value],
    directories: &[Value],
) -> Result<(Vec<String>, Vec<String>), PlanRefusal> {
    let files = entries(files, "files")?;
    let directories = entries(directories, "directories")?;
    let declared: Vec<(&str, usize, &str)> = files
        .iter()
        .enumerate()
        .map(|(n, path)| ("files", n, path.as_str()))
        .chain(
            directories
                .iter()
                .enumerate()
                .map(|(n, path)| ("directories", n, path.as_str())),
        )
        .collect();
    let named = |(list, n, _): &(&str, usize, &str)| PlanRefusal::typed(format!("{list}[{n}]"));
    let mut seen = HashSet::new();
    if let Some(repeat) = declared.iter().find(|(_, _, path)| !seen.insert(*path)) {
        return Err(named(repeat));
    }
    if let Some(over) = declared.get(LEASE_LIMIT) {
        return Err(named(over));
    }
    if declared.is_empty() {
        return Err(PlanRefusal::typed("files"));
    }
    Ok((files, directories))
}

/// The entries of the lease list `name`, each text passing the path rule,
/// else refused naming its position.
fn entries(list: &[Value], name: &str) -> Result<Vec<String>, PlanRefusal> {
    list.iter()
        .enumerate()
        .map(|(n, entry)| {
            entry
                .as_str()
                .filter(|path| plain_relative_path(path))
                .map(str::to_owned)
                .ok_or_else(|| PlanRefusal::typed(format!("{name}[{n}]")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::{Value, json};

    use super::*;
    use crate::plan::fixtures::{accepted, f1, p1, refusal, typed, with};

    /// P1 with its lease replaced.
    fn leased(files: Value, directories: Value) -> Value {
        with(
            with(p1(), "", "files", files),
            "",
            "directories",
            directories,
        )
    }

    fn texts(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn a_lease_entry_with_a_trailing_separator_or_another_bad_part_is_caught() {
        for path in [
            "docs/",
            "/src/judge.rs",
            "src\\judge.rs",
            "src//judge.rs",
            "./src",
            "src/./judge.rs",
            "src/../judge.rs",
            "..",
            "",
        ] {
            assert!(!plain_relative_path(path), "{path:?} refused");
        }
        for path in [
            "src/judge.rs",
            "docs",
            ".github/workflows/test.yml",
            "a..b/c",
        ] {
            assert!(plain_relative_path(path), "{path:?} accepted");
        }
    }

    #[test]
    fn a_repeated_lease_entry_accepted_or_named_at_the_first_is_caught() {
        let facts = f1();
        let cases = [
            (
                leased(json!(["src/judge.rs", "src/judge.rs"]), json!(["docs"])),
                "files[1]",
            ),
            (
                leased(json!(["src/judge.rs"]), json!(["src/judge.rs", "docs"])),
                "directories[0]",
            ),
            (
                leased(json!(["src/judge.rs"]), json!(["docs", "docs"])),
                "directories[1]",
            ),
        ];
        for (plan, path) in cases {
            assert_eq!(refusal(&plan, &facts), typed(path), "{plan}");
        }
    }

    #[test]
    fn a_lease_over_its_limit_accepted_or_one_at_it_refused_is_caught() {
        let facts = f1();
        let mut directories = vec![json!("docs")];
        directories.extend((0..=253).map(|n| json!(format!("d/{n}"))));
        let at_limit = with(p1(), "", "directories", Value::Array(directories.clone()));
        accepted(&at_limit, &facts);
        directories.push(json!("d/254"));
        let over = with(p1(), "", "directories", Value::Array(directories));
        assert_eq!(refusal(&over, &facts), typed("directories[255]"));
    }

    #[test]
    fn an_empty_lease_or_files_beside_directories_judged_wrongly_is_caught() {
        let facts = f1();
        accepted(&leased(json!([]), json!(["docs", "src"])), &facts);
        assert_eq!(
            refusal(&leased(json!([]), json!([])), &facts),
            typed("files")
        );
        accepted(
            &leased(json!(["src/judge.rs", "docs/plan.md"]), json!([])),
            &facts,
        );
    }

    #[test]
    fn a_path_inside_the_lease_refused_or_one_outside_accepted_is_caught() {
        let files = texts(&["src/judge.rs"]);
        let directories = texts(&["docs"]);
        for path in ["src/judge.rs", "docs/plan.md", "docs/design/a.md"] {
            assert!(inside_lease(path, &files, &directories), "{path} inside");
        }
        for path in [
            "docs",
            "docsx/a.md",
            "src/judge.rs/x",
            "src/judge.rss",
            "src",
            "docs/../src/x.rs",
            "/docs/a.md",
            "docs/",
        ] {
            assert!(!inside_lease(path, &files, &directories), "{path} outside");
        }
        let private = texts(&["docs/private"]);
        assert!(inside_lease("docs/private/a.md", &[], &private));
        assert!(!inside_lease("docs/public/a.md", &[], &private));
    }
}
