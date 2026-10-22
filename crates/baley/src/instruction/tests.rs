use serde_json::json;

use super::*;

/// The nineteen front doors not served yet, with their owning builds, written out from the
/// design rather than read from the table under test.
const UNAVAILABLE: &[(&str, u32)] = &[
    ("bal-context", 4),
    ("bal-plan", 4),
    ("bal-review", 4),
    ("bal-plan-review", 4),
    ("bal-decision-review", 4),
    ("bal-minimalism-review", 4),
    ("bal-execute", 5),
    ("bal-verify", 5),
    ("bal-audit", 5),
    ("bal-coverage", 5),
    ("bal-land", 6),
    ("bal-milestone", 6),
    ("bal-undo", 6),
    ("bal-progress", 7),
    ("bal-suggest", 7),
    ("bal-task", 8),
    ("bal-debug", 8),
    ("bal-spike", 8),
    ("bal-why", 8),
];

#[test]
fn an_unavailable_front_door_is_never_presented_as_usable_and_names_its_owning_build() {
    assert_eq!(UNAVAILABLE.len(), 19);
    for (identity, build) in UNAVAILABLE {
        assert_eq!(
            lookup(identity),
            Lookup::Unavailable {
                identity,
                build: *build
            },
            "{identity}"
        );
    }
    let unserved = ENTRIES.iter().filter(|entry| entry.text.is_none()).count();
    assert_eq!(unserved, UNAVAILABLE.len());
}

#[test]
fn the_unavailable_refusal_names_the_build_in_words_and_as_an_integer_in_slot_identity() {
    let refusal = unavailable("bal-plan", 4);
    assert_eq!(refusal["status"], "refused");
    assert_eq!(refusal["code"], "instruction-unavailable");
    assert_eq!(refusal["slot"], "identity");
    assert_eq!(refusal["details"], json!({"build": 4}));
    let reason = refusal["reason"].as_str().unwrap();
    assert!(reason.contains("Build 4") && reason.contains("bal-plan"));
}

#[test]
fn a_path_shaped_identity_is_unknown_and_no_file_content_reaches_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("bal-help");
    let skill = dir.path().join("skills/bal-help/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&plain, "MARKER-PLAIN-FILE\n").unwrap();
    std::fs::write(&skill, "MARKER-SKILL-FILE\n").unwrap();
    let spellings = [
        plain.to_str().unwrap(),
        skill.to_str().unwrap(),
        "skills/bal-help/SKILL.md",
    ];
    for spelling in spellings {
        assert_eq!(lookup(spelling), Lookup::Unknown, "{spelling}");
    }
    let refusal = unknown().to_string();
    assert!(!refusal.contains("MARKER"));
    assert!(refusal.contains("unknown-instruction") && refusal.contains("identity"));
}

#[test]
fn a_loose_spelling_of_a_registered_identity_is_unknown() {
    assert_eq!(lookup("BAL-PLAN"), Lookup::Unknown);
    assert_eq!(lookup("/bal-plan"), Lookup::Unknown);
    assert_eq!(lookup("bal-plan "), Lookup::Unknown);
    assert_eq!(lookup(""), Lookup::Unknown);
}

/// Every (identity, version, hash) this registry has served, written out by
/// hand. Rows are only ever added: a changed text gets a new version and a new
/// row, and the old row stays so the old pair is never reused.
const SERVED_EVER: &[(&str, &str, &str)] = &[
    (
        "bal-help",
        "1",
        "6a546a62197f3070cdfa4725732e5f8e3c00d21addf3b77696459c0dcb469cad",
    ),
    (
        "bal-read-contract",
        "1",
        "4feb8474b44d0f4b85290797064cc1e57a369f5b3afdc0ca866afe8ea7d6c217",
    ),
    (
        "bal-capture",
        "1",
        "52867052fb5ae23a0962120f07e6fbfa3ba0010bf2e52c109351ff8a1e8b7c5e",
    ),
    (
        "bal-read-contract",
        "2",
        "e20a4e61b36b0e0787a672c58558b04102a4357d428155ac3b618243b578ddd2",
    ),
    (
        "bal-start-planner",
        "1",
        "5c2872a582d0d1b38105c8ff971cbeae8727d0a74e51593e80b794a2bc90488a",
    ),
    (
        "bal-start-survey",
        "1",
        "ef390f27faddc98daf28a40e86a917589caf0ae1fd8185a9ebcce06592986955",
    ),
    (
        "bal-project",
        "1",
        "7153c55fcb0acaf2712762b305b9d96cfd6a48ae4e44aa354b3685820411d3d1",
    ),
    (
        "bal-scope",
        "1",
        "8ca51542feb46f946b787f5609c6a896ee5cd7954655c9ba7397a709390e74c8",
    ),
    (
        "bal-phase",
        "1",
        "8232b5d90eea73bef1d92f09919ce23ec8dae6dec60bd76b75dca6c65c09ba3b",
    ),
    (
        "bal-story",
        "1",
        "7688ddacbf34f6c93a4eb82b8eb60d69089eb2fee0ab5176097a4dc560877693",
    ),
    (
        "bal-backlog",
        "1",
        "4dceb6272fd0d130c5d74c904919128e51a06eab7330fb7bc02b932aa3322351",
    ),
    (
        "bal-help",
        "2",
        "ab97190b91b54fb647853049e3dbf54e82d736a31ba36996a83bc5d981471733",
    ),
    (
        "bal-rounds",
        "1",
        "8bab77d68c4253a45e63cf5f16397e58e058c21f94781674c0cf08c09bfa3e6d",
    ),
    (
        "bal-analyzer-questions",
        "1",
        "bf73b552ba3f4247d11f2ac8897f12065de8f9e63f065619e4837317a293a937",
    ),
    (
        "bal-analyzer-drafting",
        "1",
        "8cfeef1bdc7915b39f5cbe2002a89db30e2a333b678af2dc468cde794bac0810",
    ),
];

fn served() -> impl Iterator<Item = (&'static str, &'static Text)> {
    ENTRIES
        .iter()
        .filter_map(|entry| entry.text.as_ref().map(|text| (entry.identity, text)))
}

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn a_served_text_changed_without_its_pinned_hash_or_a_new_version_row_fails() {
    assert!(served().count() >= 1);
    for (identity, text) in served() {
        assert_eq!(
            sha256_hex(text.body),
            text.hash,
            "{identity}: the text no longer matches its pinned hash"
        );
        assert!(
            SERVED_EVER.contains(&(identity, text.version, text.hash)),
            "{identity} version {} hash {} is not in the served-ever list",
            text.version,
            text.hash
        );
    }
}

#[test]
fn one_identity_and_version_never_carry_two_hashes_in_the_served_ever_list() {
    for (index, (identity, version, hash)) in SERVED_EVER.iter().enumerate() {
        for (other_identity, other_version, other_hash) in &SERVED_EVER[index + 1..] {
            if identity == other_identity && version == other_version {
                assert_eq!(hash, other_hash, "{identity} {version} has two hashes");
            }
        }
    }
}

#[test]
fn no_served_text_carries_inherited_lifecycle_or_disk_skill_wording() {
    let forbidden = [
        "document-search",
        ".planning",
        "skills/",
        "SKILL.md",
        "CLAUDE_PLUGIN_ROOT",
    ];
    for (identity, text) in served() {
        for word in forbidden {
            assert!(!text.body.contains(word), "{identity} contains {word}");
        }
    }
}

#[test]
fn the_help_text_asks_for_its_identity_on_apply_calls_only() {
    let Lookup::Served { text, .. } = lookup("bal-help") else {
        panic!("bal-help is served");
    };
    let body = text.body;
    assert!(body.contains("`bal-help`") && body.contains("`instruction`"));
    assert!(body.contains("Never send it on a `baley_query` call"));
}

#[test]
fn a_capture_text_without_its_identity_request_or_either_kind_is_caught() {
    let Lookup::Served { text, .. } = lookup("bal-capture") else {
        panic!("bal-capture is served");
    };
    let body = text.body;
    assert!(body.contains("`baley_apply`"));
    assert!(body.contains(r#""operation":"capture""#));
    assert!(body.contains(r#""instruction":"bal-capture""#));
    assert!(body.contains(r#""kind":"note""#) && body.contains(r#""kind":"story""#));
}

#[test]
fn a_read_contract_that_never_says_how_to_read_a_capture_back_is_caught() {
    let Lookup::Served { text, .. } = lookup("bal-read-contract") else {
        panic!("bal-read-contract is served");
    };
    assert!(text.body.contains(r#""operation":"document""#));
    assert!(text.body.contains(r#""kind":"capture""#));
}

#[test]
fn the_read_contract_stays_within_a_description_and_keeps_its_paging_and_host_tools_rules() {
    let Lookup::Served { text, .. } = lookup("bal-read-contract") else {
        panic!("bal-read-contract is served");
    };
    let body = text.body;
    assert!(
        body.chars().count() <= 2048,
        "{} characters",
        body.chars().count()
    );
    for needle in [
        "\"operation\":\"instruction\"",
        "\"identity\"",
        "part",
        "next",
        "host's own",
    ] {
        assert!(body.contains(needle), "the read contract lost {needle}");
    }
}

#[test]
fn the_help_evidence_takes_its_version_and_hash_from_the_registry_not_the_binary() {
    let evidence = evidence("bal-help").unwrap();
    // The list is append-only, so the newest row is the last one for the identity.
    let (_, version, hash) = SERVED_EVER
        .iter()
        .rfind(|(identity, _, _)| *identity == "bal-help")
        .unwrap();
    assert_eq!(evidence.identity(), "bal-help");
    assert_eq!(evidence.version(), *version);
    assert_eq!(evidence.hash(), *hash);
    assert_ne!(evidence.version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn every_served_entry_fits_the_caller_records_limits_as_evidence() {
    for (identity, text) in served() {
        let evidence = evidence(identity).unwrap_or_else(|_| panic!("{identity} is refused"));
        assert_eq!(evidence.identity(), identity);
        assert_eq!(evidence.version(), text.version);
        assert_eq!(evidence.hash(), text.hash);
    }
}

#[test]
fn an_unavailable_or_unknown_identity_is_refused_and_given_no_evidence() {
    assert_eq!(
        evidence("bal-plan").unwrap_err(),
        unavailable("bal-plan", 4)
    );
    assert_eq!(evidence("no-such-instruction").unwrap_err(), unknown());
}

fn served_text(identity: &str) -> &'static str {
    let Lookup::Served { text, .. } = lookup(identity) else {
        panic!("{identity} is served");
    };
    text.body
}

#[test]
fn a_start_planner_text_missing_its_order_binding_or_submission_shape_is_caught() {
    let body = served_text("bal-start-planner");
    for needle in [
        "inputs",
        "brief",
        "scope-submit",
        "\"work_order\"",
        "request_id",
        "bal-start-survey",
        "baley scope approve",
        "description",
        "what",
        "core_value",
        "constraints",
        "stories",
        "label",
        "sentence",
        "status",
        "active",
        "deferred",
        "excluded",
        "roadmap",
        "name",
        "goal",
        "detail",
        "depends_on",
    ] {
        assert!(
            body.contains(needle),
            "the start planner text lacks {needle}"
        );
    }
    assert!(!body.contains("scope-approve"));
}

#[test]
fn a_survey_text_that_writes_or_submits_is_caught() {
    let body = served_text("bal-start-survey");
    assert!(body.contains("host's own file, search and shell tools"));
    assert!(body.contains("Write no file") && body.contains("make no commit"));
    assert!(!body.contains("scope-submit"));
}

#[test]
fn a_start_instruction_given_a_help_row_or_a_skill_stub_is_caught() {
    let doors = crate::host_artifacts::stubs::front_doors();
    for identity in ["bal-start-planner", "bal-start-survey"] {
        let Lookup::Served { entry, text } = lookup(identity) else {
            panic!("{identity} is served");
        };
        assert_eq!(entry.build, 4, "{identity}");
        assert_eq!(text.version, "1", "{identity}");
        assert!(
            !crate::help::table::COMMANDS
                .iter()
                .any(|command| command.name == identity),
            "{identity} is a help row"
        );
        assert!(
            !doors.iter().any(|(name, _)| *name == identity),
            "{identity} is a front door"
        );
    }
}

#[test]
fn a_project_front_door_missing_an_operation_it_drives_or_the_terminal_approval_is_caught() {
    let body = served_text("bal-project");
    for needle in [
        "project-start",
        "brief",
        r#""instruction":"bal-project""#,
        "settings",
        "start-order-pending",
        "launch",
        "prompt",
        "work-order-fail",
        "work-order-retry",
        "project-describe",
        "baley scope approve",
    ] {
        assert!(
            body.contains(needle),
            "the project front door lacks {needle}"
        );
    }
    assert!(!body.contains("scope-approve"));
}

/// The four scope change front doors, each with the operations it drives,
/// written out from design 0004 rather than read from the texts under test.
const CHANGE_DOORS: &[(&str, &[&str])] = &[
    ("bal-scope", &[r#""kind":"draft""#]),
    (
        "bal-phase",
        &[
            "phase-declare",
            "phase-edit",
            "phase-reorder",
            "phase-withdraw",
        ],
    ),
    (
        "bal-story",
        &[
            "story-declare",
            "story-edit",
            "story-reassign",
            "story-drop",
        ],
    ),
    ("bal-backlog", &["backlog-reorder"]),
];

#[test]
fn a_change_front_door_missing_an_operation_it_drives_or_the_terminal_approval_is_caught() {
    for (identity, operations) in CHANGE_DOORS {
        let body = served_text(identity);
        for needle in operations.iter().chain(&["baley scope approve"]) {
            assert!(body.contains(needle), "{identity} lacks {needle}");
        }
        assert!(!body.contains("scope-approve"), "{identity}");
    }
}

#[test]
fn a_change_front_door_asking_for_an_argument_its_operations_refuse_is_caught() {
    for (identity, _) in CHANGE_DOORS {
        assert!(
            !served_text(identity).contains(r#""instruction":"#),
            "{identity} asks for instruction"
        );
    }
}

#[test]
fn a_rounds_instruction_given_a_help_row_or_a_skill_stub_is_caught() {
    let Lookup::Served { entry, text } = lookup("bal-rounds") else {
        panic!("bal-rounds is served");
    };
    assert_eq!(entry.build, 4);
    assert_eq!(text.version, "1");
    assert!(
        !crate::help::table::COMMANDS
            .iter()
            .any(|command| command.name == "bal-rounds"),
        "bal-rounds is a help row"
    );
    assert!(
        !crate::host_artifacts::stubs::front_doors()
            .iter()
            .any(|(name, _)| *name == "bal-rounds"),
        "bal-rounds is a front door"
    );
}

#[test]
fn a_rounds_instruction_missing_its_read_its_answer_or_its_relay_rules_is_caught() {
    let body = served_text("bal-rounds");
    for needle in [
        r#""kind":"questions""#,
        "questions-answer",
        r#""act":"take""#,
        r#""act":"answer""#,
        r#""act":"defer""#,
        "recommended",
        "deferred",
        "closed",
        "baley questions answer",
    ] {
        assert!(body.contains(needle), "the rounds relay lacks {needle}");
    }
    let forbidding = body
        .split(". ")
        .find(|sentence| sentence.contains("drop"))
        .expect("a sentence forbids dropping");
    for word in ["answer", "drop", "merge", "reword", "reorder"] {
        assert!(forbidding.contains(word), "the forbidding lacks {word}");
    }
    assert!(!body.contains("approve"));
}

/// Names of Cadence's own files, commands and state that no ported analyzer
/// text may carry (ADR 0048).
const CADENCE_NAMES: &[&str] = &[
    ".planning",
    "CONTEXT.md",
    "ROADMAP.md",
    "planning.mjs",
    "skim.mjs",
    "excerpt",
    "recall",
    "AskUserQuestion",
    "/cad-",
];

#[test]
fn a_questions_text_missing_its_inputs_or_return_or_naming_cadence_state_is_caught() {
    let body = served_text("bal-analyzer-questions");
    for needle in [
        "`story`",
        "`description`",
        "`roadmap`",
        "`truths`",
        "`document`",
        "\"id\"",
        "\"question\"",
        "\"recommended\"",
        "\"depends_on\"",
        "`depends_on`",
        "`[]`",
        "Call no `baley_apply` operation",
    ] {
        assert!(body.contains(needle), "the questions text lacks {needle}");
    }
    for name in CADENCE_NAMES {
        assert!(!body.contains(name), "the questions text names {name}");
    }
}

#[test]
fn a_drafting_text_missing_the_form_refusals_or_human_verify_is_caught() {
    let body = served_text("bal-analyzer-drafting");
    for needle in [
        "`when`",
        "`for-any`",
        "`sees`",
        "`gets`",
        "`is refused`",
        "`truth-form`",
        "`truth-internal`",
        "`truth-prose-oracle`",
        "`human_verify`",
        "`questions`",
        "deferred question",
        "keeps that truth's id",
        "never on a recommendation the owner rejected",
        "Leave out `observable` and `fixed_oracle`",
    ] {
        assert!(body.contains(needle), "the drafting text lacks {needle}");
    }
    // The return's shape never carries the attestations the owner makes.
    assert!(!body.contains("\"observable\"") && !body.contains("\"fixed_oracle\""));
    for name in CADENCE_NAMES {
        assert!(!body.contains(name), "the drafting text names {name}");
    }
}

#[test]
fn an_analyzer_text_given_a_help_row_or_a_skill_stub_is_caught() {
    let doors = crate::host_artifacts::stubs::front_doors();
    for identity in ["bal-analyzer-questions", "bal-analyzer-drafting"] {
        let Lookup::Served { entry, text } = lookup(identity) else {
            panic!("{identity} is served");
        };
        assert_eq!(entry.build, 4, "{identity}");
        assert_eq!(text.version, "1", "{identity}");
        assert!(
            !crate::help::table::COMMANDS
                .iter()
                .any(|command| command.name == identity),
            "{identity} is a help row"
        );
        assert!(
            !doors.iter().any(|(name, _)| *name == identity),
            "{identity} is a front door"
        );
    }
}
