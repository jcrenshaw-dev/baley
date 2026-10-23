//! The judgement of one truth submission, its digest and its target
//! (design 0005 sections 5 and 6, PLN-R2, PLN-R3, PLN-R5).
//!
//! A submission arrives as plain JSON and is judged truth by truth in
//! submitted order, each truth whole before the next, so one submission
//! always gets one answer. Baley classifies no sentence (PLN-R3): it checks
//! the mechanical form and reads the two attestations the session records.
//! Nothing here records anything.

use std::collections::HashSet;
use std::num::NonZeroU32;

use baley_store::{Hash, canonical_json, request_digest};
use serde_json::{Value, json};

use crate::capture::INLINE_TEXT_LIMIT;

/// The refusal code for a truth out of form: a list that is not a list, an
/// entry that is not an object, an unknown, missing or wrong-type member, a
/// text over 4,096 bytes, or a sentence part the form rules refuse.
pub const TRUTH_FORM: &str = "truth-form";
/// The refusal code for a blank id or one an earlier truth already has.
pub const TRUTH_ID: &str = "truth-id";
/// The refusal code for an attestation missing or not a boolean.
pub const TRUTH_ATTESTATION: &str = "truth-attestation";
/// The refusal code for a truth attested as not observable from outside.
pub const TRUTH_INTERNAL: &str = "truth-internal";
/// The refusal code for a truth attested as having no fixed oracle.
pub const TRUTH_PROSE_ORACLE: &str = "truth-prose-oracle";
/// The kind a truth draft's body and `scope.submitted` spell.
pub const STORY_TRUTHS: &str = "story-truths";

const MEMBERS: &[&str] = &[
    "id",
    "form",
    "trigger",
    "observer",
    "verb",
    "outcome",
    "kind",
    "observable",
    "fixed_oracle",
    "human_verify",
];

/// Why a truth submission is refused. Positions count from 0 in the list as
/// submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TruthRefusal {
    /// The truth at `position` is out of form, or the list is not a list
    /// when `position` is `None`. `id` is named once the truth's shape and
    /// id were read.
    Form {
        position: Option<usize>,
        id: Option<String>,
    },
    /// The truth at `position` has a blank id, or `id` is an earlier
    /// truth's.
    Id { position: usize, id: String },
    /// The truth at `position` lacks an attestation or has one that is not
    /// a boolean.
    Attestation { position: usize, id: String },
    /// The truth at `position` is attested as not observable from outside.
    Internal { position: usize, id: String },
    /// The truth at `position` is attested as having no fixed oracle.
    ProseOracle { position: usize, id: String },
}

impl TruthRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Form { .. } => TRUTH_FORM,
            Self::Id { .. } => TRUTH_ID,
            Self::Attestation { .. } => TRUTH_ATTESTATION,
            Self::Internal { .. } => TRUTH_INTERNAL,
            Self::ProseOracle { .. } => TRUTH_PROSE_ORACLE,
        }
    }
}

/// One accepted truth, its text kept exactly as written. Both attestations
/// of an accepted truth are true, so they are not kept here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truth {
    /// Its id, unique within the submission.
    pub id: String,
    /// `when` or `for-any`.
    pub form: String,
    /// The one trigger, or a `for-any` truth's input.
    pub trigger: String,
    /// The one observer.
    pub observer: String,
    /// `sees`, `gets` or `is refused`.
    pub verb: String,
    /// The one outcome.
    pub outcome: String,
    /// `literal` or `property`.
    pub kind: String,
    /// The tool or live service a person needs to prove the truth, if any.
    pub human_verify: Option<String>,
}

/// A truth submission judged whole and accepted. Only [`judge_truths`]
/// builds one, so nothing unjudged can be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruthSubmission {
    story: u32,
    set: String,
    truths: Vec<Truth>,
    body: Vec<u8>,
    digest: Hash,
    target: String,
}

impl TruthSubmission {
    /// The story the truths are for.
    pub fn story(&self) -> u32 {
        self.story
    }

    /// The question set the truths were drafted from.
    pub fn set(&self) -> &str {
        &self.set
    }

    /// The truths in submitted order.
    pub fn truths(&self) -> &[Truth] {
        &self.truths
    }

    /// The canonical JSON bytes the digest is taken over, stored as the
    /// draft's body.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The SHA-256 of the body and nothing else. No version is in it, since
    /// versions are given at approval (D-02).
    pub fn digest(&self) -> Hash {
        self.digest
    }

    /// What the draft is filed under among `story-truths` drafts:
    /// `story/<id>`, so only a newer draft for the same story makes it
    /// stale.
    pub fn target(&self) -> &str {
        &self.target
    }
}

/// Judges the truth list `truths` drafted for `story` from question set
/// `set`, each truth whole in submitted order. An empty list is accepted:
/// there is no truth cap and no rule against none (ADR 0017).
pub fn judge_truths(
    story: NonZeroU32,
    set: &str,
    truths: &Value,
) -> Result<TruthSubmission, TruthRefusal> {
    let Value::Array(entries) = truths else {
        return Err(TruthRefusal::Form {
            position: None,
            id: None,
        });
    };
    let mut seen = HashSet::new();
    let judged = entries
        .iter()
        .enumerate()
        .map(|(position, entry)| judge_truth(position, entry, &mut seen))
        .collect::<Result<Vec<_>, _>>()?;
    let story = story.get();
    let value = json!({
        "kind": STORY_TRUTHS,
        "story": story,
        "set": set,
        "truths": entries,
    });
    // Every leaf of an accepted submission is text, a boolean or a number
    // within u32, so it always has a canonical form.
    let digest = request_digest(&value).expect("an accepted submission is canonical");
    let body = canonical_json(&value).expect("an accepted submission is canonical");
    Ok(TruthSubmission {
        story,
        set: set.to_owned(),
        truths: judged,
        body,
        digest,
        target: format!("story/{story}"),
    })
}

/// Judges one truth whole: its shape, then its id, then its form, then its
/// attestations, so the first fault in that order answers.
fn judge_truth(
    position: usize,
    entry: &Value,
    seen: &mut HashSet<String>,
) -> Result<Truth, TruthRefusal> {
    let truth = shape(entry).ok_or(TruthRefusal::Form {
        position: Some(position),
        id: None,
    })?;
    let id = || truth.id.clone();
    if is_blank(&truth.id) || !seen.insert(truth.id.clone()) {
        return Err(TruthRefusal::Id { position, id: id() });
    }
    if !in_form(&truth) {
        return Err(TruthRefusal::Form {
            position: Some(position),
            id: Some(id()),
        });
    }
    let attested = |name: &str| entry.get(name).and_then(Value::as_bool);
    let (Some(observable), Some(fixed_oracle)) = (attested("observable"), attested("fixed_oracle"))
    else {
        return Err(TruthRefusal::Attestation { position, id: id() });
    };
    if !observable {
        return Err(TruthRefusal::Internal { position, id: id() });
    }
    if !fixed_oracle {
        return Err(TruthRefusal::ProseOracle { position, id: id() });
    }
    Ok(truth)
}

/// One entry's texts read, or `None` when it is not an object, has an
/// unknown member, or has a text missing, of another type or over the
/// inline limit.
fn shape(entry: &Value) -> Option<Truth> {
    let Value::Object(object) = entry else {
        return None;
    };
    if object.keys().any(|name| !MEMBERS.contains(&name.as_str())) {
        return None;
    }
    // The bound is on bytes as written, before any trimming.
    let text = |value: Option<&Value>| match value {
        Some(Value::String(text)) if text.len() <= INLINE_TEXT_LIMIT => Some(text.clone()),
        _ => None,
    };
    let human_verify = match object.get("human_verify") {
        None => None,
        given => Some(text(given)?),
    };
    Some(Truth {
        id: text(object.get("id"))?,
        form: text(object.get("form"))?,
        trigger: text(object.get("trigger"))?,
        observer: text(object.get("observer"))?,
        verb: text(object.get("verb"))?,
        outcome: text(object.get("outcome"))?,
        kind: text(object.get("kind"))?,
        human_verify,
    })
}

/// Whether a truth meets the mechanical form rules (PLN-R2, D-15, D-16):
/// one trigger, one observer, an allowed verb and kind, no blank part, and
/// a `for-any` truth only as `gets` with kind `property`.
fn in_form(truth: &Truth) -> bool {
    let form = matches!(truth.form.as_str(), "when" | "for-any");
    let parts = [&truth.trigger, &truth.observer, &truth.verb, &truth.outcome];
    let filled = !parts.iter().any(|part| is_blank(part))
        && !truth.human_verify.as_deref().is_some_and(is_blank);
    let one_trigger = !truth.trigger.contains(" or ");
    let one_observer = ![" and ", " & ", ",", ";"]
        .iter()
        .any(|separator| truth.observer.contains(separator));
    let verb = matches!(truth.verb.as_str(), "sees" | "gets" | "is refused");
    let kind = matches!(truth.kind.as_str(), "literal" | "property");
    let property = truth.form != "for-any" || (truth.verb == "gets" && truth.kind == "property");
    form && filled && one_trigger && one_observer && verb && kind && property
}

fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use super::*;

    const SET: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    /// `sha256sum` over U1's 536-byte body.
    const U1_DIGEST: &str = "c85deb14471b95d360f3d94b276751e67770ea1ce447597bce8cec83436e2bab";

    fn t1() -> Value {
        json!({"id": "t1", "form": "when",
            "trigger": "the owner approves a draft by a stale digest",
            "observer": "the owner", "verb": "is refused",
            "outcome": "stale-draft naming the first differing part", "kind": "literal",
            "observable": true, "fixed_oracle": true})
    }

    fn t2() -> Value {
        json!({"id": "t2", "form": "for-any",
            "trigger": "order the answers of a set are recorded in",
            "observer": "a ledger reader", "verb": "gets", "outcome": "the same round",
            "kind": "property", "observable": true, "fixed_oracle": true})
    }

    fn u1() -> Value {
        json!([t1(), t2()])
    }

    fn with(mut truth: Value, member: &str, to: Value) -> Value {
        truth[member] = to;
        truth
    }

    fn without(mut truth: Value, member: &str) -> Value {
        truth.as_object_mut().unwrap().remove(member);
        truth
    }

    fn story1() -> NonZeroU32 {
        NonZeroU32::new(1).unwrap()
    }

    fn judge(truths: &Value) -> Result<TruthSubmission, TruthRefusal> {
        judge_truths(story1(), SET, truths)
    }

    fn refusal(truths: &Value) -> TruthRefusal {
        judge(truths).expect_err("refused")
    }

    fn body_of(truths: &Value) -> Value {
        let accepted = judge(truths).expect("accepted");
        serde_json::from_slice(accepted.body()).unwrap()
    }

    fn form(position: usize, id: Option<&str>) -> TruthRefusal {
        TruthRefusal::Form {
            position: Some(position),
            id: id.map(str::to_owned),
        }
    }

    fn named(code: fn(usize, String) -> TruthRefusal, position: usize, id: &str) -> TruthRefusal {
        code(position, id.to_owned())
    }

    fn id(position: usize, id: String) -> TruthRefusal {
        TruthRefusal::Id { position, id }
    }

    fn attestation(position: usize, id: String) -> TruthRefusal {
        TruthRefusal::Attestation { position, id }
    }

    fn internal(position: usize, id: String) -> TruthRefusal {
        TruthRefusal::Internal { position, id }
    }

    fn prose_oracle(position: usize, id: String) -> TruthRefusal {
        TruthRefusal::ProseOracle { position, id }
    }

    #[test]
    fn a_truth_out_of_form_or_unattested_accepted_or_refused_under_another_code_is_caught() {
        let body = concat!(
            r#"{"kind":"story-truths","set":"5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d","story":1,"#,
            r#""truths":[{"fixed_oracle":true,"form":"when","id":"t1","kind":"literal","#,
            r#""observable":true,"observer":"the owner","#,
            r#""outcome":"stale-draft naming the first differing part","#,
            r#""trigger":"the owner approves a draft by a stale digest","verb":"is refused"},"#,
            r#"{"fixed_oracle":true,"form":"for-any","id":"t2","kind":"property","#,
            r#""observable":true,"observer":"a ledger reader","outcome":"the same round","#,
            r#""trigger":"order the answers of a set are recorded in","verb":"gets"}]}"#
        );
        assert_eq!(body.len(), 536);
        let accepted = judge(&u1()).expect("U1 is accepted");
        assert_eq!(accepted.target(), "story/1");
        assert_eq!(std::str::from_utf8(accepted.body()).unwrap(), body);
        assert_eq!(accepted.digest().to_hex(), U1_DIGEST);

        let in_t1 = |member: &str, to: Value| json!([with(t1(), member, to), t2()]);
        let in_t2 = |member: &str, to: Value| json!([t1(), with(t2(), member, to)]);
        let t1_without = |member: &str| json!([without(t1(), member), t2()]);
        let both_false = json!([
            with(
                with(t1(), "observable", json!(false)),
                "fixed_oracle",
                json!(false)
            ),
            t2()
        ]);
        let cases = [
            (
                in_t1("trigger", json!("the owner approves or rejects a draft")),
                form(0, Some("t1")),
            ),
            (
                in_t1("observer", json!("the owner and the session")),
                form(0, Some("t1")),
            ),
            (
                in_t1("observer", json!("the owner, the session")),
                form(0, Some("t1")),
            ),
            (in_t1("outcome", json!("  ")), form(0, Some("t1"))),
            (in_t1("verb", json!("observes")), form(0, Some("t1"))),
            (in_t1("kind", json!("fuzzy")), form(0, Some("t1"))),
            (in_t1("form", json!("if")), form(0, Some("t1"))),
            (in_t2("verb", json!("sees")), form(1, Some("t2"))),
            (in_t2("kind", json!("literal")), form(1, Some("t2"))),
            (t1_without("observable"), named(attestation, 0, "t1")),
            (t1_without("fixed_oracle"), named(attestation, 0, "t1")),
            (in_t1("observable", json!(false)), named(internal, 0, "t1")),
            (
                in_t1("fixed_oracle", json!(false)),
                named(prose_oracle, 0, "t1"),
            ),
            (both_false, named(internal, 0, "t1")),
        ];
        for (truths, expected) in cases {
            let refused = refusal(&truths);
            assert_eq!(refused, expected, "{truths}");
            let code = match &expected {
                TruthRefusal::Form { .. } => "truth-form",
                TruthRefusal::Id { .. } => "truth-id",
                TruthRefusal::Attestation { .. } => "truth-attestation",
                TruthRefusal::Internal { .. } => "truth-internal",
                TruthRefusal::ProseOracle { .. } => "truth-prose-oracle",
            };
            assert_eq!(refused.code(), code, "{truths}");
        }
    }

    #[test]
    fn a_truth_id_blank_or_repeated_accepted_is_caught() {
        assert_eq!(
            refusal(&json!([t1(), with(t2(), "id", json!("t1"))])),
            named(id, 1, "t1")
        );
        assert_eq!(
            refusal(&json!([with(t1(), "id", json!(" ")), t2()])),
            named(id, 0, " ")
        );
    }

    #[test]
    fn a_truth_with_an_unknown_member_or_a_wrong_type_accepted_is_caught() {
        let cases = [
            (json!([with(t1(), "owner", json!("Ada"))]), form(0, None)),
            (json!([with(t1(), "verb", json!(3))]), form(0, None)),
            (
                json!([with(t1(), "human_verify", json!(true))]),
                form(0, None),
            ),
            (
                json!({"t1": t1()}),
                TruthRefusal::Form {
                    position: None,
                    id: None,
                },
            ),
            (json!([null]), form(0, None)),
            (json!(["t1"]), form(0, None)),
        ];
        for (truths, expected) in cases {
            assert_eq!(refusal(&truths), expected, "{truths}");
        }
    }

    #[test]
    fn a_human_verify_field_dropped_or_blank_accepted_is_caught() {
        let named = with(t1(), "human_verify", json!("a running Claude Code session"));
        let body = body_of(&json!([named]));
        assert_eq!(
            body["truths"][0]["human_verify"],
            json!("a running Claude Code session")
        );
        let accepted = judge(&json!([named])).unwrap();
        assert_eq!(
            accepted.truths()[0].human_verify.as_deref(),
            Some("a running Claude Code session")
        );
        assert_eq!(
            refusal(&json!([with(t1(), "human_verify", json!("   "))])),
            form(0, Some("t1"))
        );
    }

    #[test]
    fn an_empty_list_refused_or_text_trimmed_is_caught() {
        let empty = judge(&json!([])).expect("an empty list is accepted");
        assert_eq!(empty.target(), "story/1");
        assert!(empty.truths().is_empty());
        let padded = " stale-draft naming the first differing part ";
        let body = body_of(&json!([with(t1(), "outcome", json!(padded))]));
        assert_eq!(body["truths"][0]["outcome"], json!(padded));
    }

    #[test]
    fn faults_judged_in_another_order_are_caught() {
        let blank_id = with(with(t1(), "id", json!(" ")), "verb", json!("observes"));
        assert_eq!(refusal(&json!([blank_id])), named(id, 0, " "));
        let unknown = without(with(t1(), "owner", json!("Ada")), "observable");
        assert_eq!(refusal(&json!([unknown])), form(0, None));
        let observes = without(with(t1(), "verb", json!("observes")), "observable");
        assert_eq!(refusal(&json!([observes])), form(0, Some("t1")));
        let t2_faults = with(with(t2(), "observable", json!(false)), "id", json!(" "));
        assert_eq!(refusal(&json!([t1(), t2_faults])), named(id, 1, " "));
    }

    #[test]
    fn a_truth_text_over_the_inline_limit_accepted_or_one_at_it_refused_is_caught() {
        let at_bound = "a".repeat(4_096);
        let body = body_of(&json!([with(t1(), "outcome", json!(at_bound))]));
        assert_eq!(body["truths"][0]["outcome"], json!(at_bound));
        let accents = "é".repeat(2_048);
        assert_eq!(accents.len(), 4_096);
        let body = body_of(&json!([with(t1(), "outcome", json!(accents))]));
        assert_eq!(body["truths"][0]["outcome"], json!(accents));

        let refused = [
            with(t1(), "outcome", json!("a".repeat(4_097))),
            with(t1(), "outcome", json!(format!(" {}", "a".repeat(4_096)))),
            with(t1(), "outcome", json!("é".repeat(2_049))),
            with(t1(), "id", json!("a".repeat(4_097))),
            with(t1(), "human_verify", json!("a".repeat(4_097))),
        ];
        for truth in refused {
            assert_eq!(refusal(&json!([truth])), form(0, None));
        }
    }
}
