//! The retry judge: whether a work order may be retried, and what the retry
//! re-issues (design 0003 section 6, CFG-R16).
//!
//! Only a failed order nobody retried may be retried, and only while no
//! submission has named it or any earlier attempt of its chain, so pending
//! work is never escalated, an answer to any attempt of a chain ends it, and a
//! session retries only the newest attempt of a chain. The retry keeps the
//! parent's role, subject and inputs and counts one attempt more.
//!
//! The judge reads no policy: the binary resolves the route again under the
//! current policy with the new attempt. It never sees a replay either, since
//! the store answers a repeated request id from its record.

use std::num::NonZeroU32;

use baley_store::{Hash, PayloadRef};
use serde_json::{Map, Value};

use super::event::OrderInput;
use super::view::{ANSWERED, FAILED, PENDING};
use crate::policy::Role;

/// The code of an order id this project's `work_order` view does not hold.
pub const NO_SUCH_WORK_ORDER: &str = "no-such-work-order";
/// The code of a failure recorded for an order that is not pending.
pub const WORK_ORDER_NOT_PENDING: &str = "work-order-not-pending";
/// The code of a retry of an order that is pending or answered, or that
/// failed after a submission named it or any earlier attempt of its chain,
/// since an answer to any attempt of a chain ends it.
pub const WORK_ORDER_NOT_FAILED: &str = "work-order-not-failed";
/// The code of a retry of an order already retried; it names the retry.
pub const WORK_ORDER_RETRIED: &str = "work-order-retried";
/// The code of a retry whose input this project purged; it names the input.
pub const MATERIAL_UNAVAILABLE: &str = "material-unavailable";
/// The code of a start made while a start planner order is pending; it names
/// that order.
pub const START_ORDER_PENDING: &str = "start-order-pending";
/// The code of a submission naming an order that is not this project's
/// start planner order.
pub const WRONG_WORK_ORDER: &str = "wrong-work-order";

/// What a retry re-issues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retry {
    /// The logical request: the id of the chain's first order.
    pub request: String,
    /// The failed order retried.
    pub parent: String,
    /// The parent's attempt plus one.
    pub attempt: NonZeroU32,
    /// The parent's role.
    pub role: Role,
    /// The parent's subject.
    pub subject: String,
    /// The parent's inputs, unchanged.
    pub inputs: Vec<OrderInput>,
}

/// Why an order may not be retried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryRefusal {
    /// `work-order-not-failed`: the order is pending or answered.
    NotFailed {
        /// Its state, `answered` for a failed order a submission named.
        state: String,
    },
    /// `work-order-not-failed`: a submission named an earlier attempt of the
    /// order's chain. Plan 3 answers it with details
    /// `{"state": "answered", "work_order": <the earlier attempt's id>}`.
    EarlierAnswered {
        /// The earlier attempt's id.
        work_order: String,
    },
    /// `work-order-retried`: a retry was already issued from it.
    Retried {
        /// The retry's id.
        child: String,
    },
    /// `material-unavailable`: this project purged an input's body.
    MaterialUnavailable {
        /// The input's name.
        input: String,
    },
    /// The attempt would pass `u32::MAX`.
    Exhausted,
    /// The document does not read as an order.
    Unreadable {
        /// What could not be read.
        reason: String,
    },
}

/// Judges a retry of `order`, a document as the `work_order` view holds it,
/// given its earlier attempts, parent first, back to the chain's first order,
/// as the view holds them, and the `(seq, hash)` references this project's
/// purges released. An answer to any attempt of the chain ends it, so a
/// submission naming the order or any earlier attempt refuses the retry.
pub fn judge_retry(
    order: &Value,
    earlier: &[Value],
    released: &[(u64, Hash)],
) -> Result<Retry, RetryRefusal> {
    let unreadable = |reason| RetryRefusal::Unreadable { reason };
    let order = read(order).map_err(unreadable)?;
    if order.state != FAILED {
        return Err(RetryRefusal::NotFailed { state: order.state });
    }
    // A return that came after the failure ends the chain as surely as one
    // that came before it.
    if order.answered {
        return Err(RetryRefusal::NotFailed {
            state: ANSWERED.into(),
        });
    }
    if let Some(child) = order.child {
        return Err(RetryRefusal::Retried { child });
    }
    // A late return lands on the attempt it named, which may be any earlier
    // one, so the candidate's own drafts are not enough.
    let earlier = earlier
        .iter()
        .map(read_earlier)
        .collect::<Result<Vec<_>, _>>()
        .map_err(unreadable)?;
    if let Some((id, _)) = earlier.into_iter().find(|(_, answered)| *answered) {
        return Err(RetryRefusal::EarlierAnswered { work_order: id });
    }
    if let Some(input) = order
        .inputs
        .iter()
        .find(|input| released.contains(&(order.seq, input.body.hash)))
    {
        return Err(RetryRefusal::MaterialUnavailable {
            input: input.name.clone(),
        });
    }
    let attempt = order
        .attempt
        .checked_add(1)
        .ok_or(RetryRefusal::Exhausted)?;
    Ok(Retry {
        request: order.request,
        parent: order.id,
        attempt,
        role: order.role,
        subject: order.subject,
        inputs: order.inputs,
    })
}

/// The members of an order document the judge reads.
struct Order {
    id: String,
    request: String,
    role: Role,
    subject: String,
    attempt: NonZeroU32,
    inputs: Vec<OrderInput>,
    seq: u64,
    state: String,
    answered: bool,
    child: Option<String>,
}

fn read(order: &Value) -> Result<Order, String> {
    let Value::Object(fields) = order else {
        return Err("the document is not an object".into());
    };
    let text = |name: &str| {
        fields
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{name} is not text"))
    };
    let state = text("state")?;
    if ![PENDING, FAILED, ANSWERED].contains(&state.as_str()) {
        return Err(format!("state {state:?} is no order state"));
    }
    let role = Role::parse(&text("role")?).ok_or("role is no role")?;
    let attempt = fields
        .get("attempt")
        .and_then(Value::as_u64)
        .and_then(|attempt| u32::try_from(attempt).ok())
        .and_then(NonZeroU32::new)
        .ok_or("attempt is not a counter from 1")?;
    let seq = fields
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or("seq is not a sequence")?;
    let Some(Value::Array(drafts)) = fields.get("drafts") else {
        return Err("drafts is not a list".into());
    };
    let child = match fields.get("child") {
        Some(Value::Null) => None,
        Some(Value::String(child)) => Some(child.clone()),
        _ => return Err("child is not text or null".into()),
    };
    Ok(Order {
        id: text("id")?,
        request: text("request")?,
        role,
        subject: text("subject")?,
        attempt,
        inputs: inputs(fields)?,
        seq,
        state,
        answered: !drafts.is_empty(),
        child,
    })
}

/// An earlier attempt's id, and whether a submission named it.
fn read_earlier(attempt: &Value) -> Result<(String, bool), String> {
    let Some(id) = attempt.get("id").and_then(Value::as_str) else {
        return Err("an earlier attempt's id is not text".into());
    };
    let Some(Value::Array(drafts)) = attempt.get("drafts") else {
        return Err(format!("drafts of earlier attempt {id} is not a list"));
    };
    Ok((id.to_owned(), !drafts.is_empty()))
}

fn inputs(fields: &Map<String, Value>) -> Result<Vec<OrderInput>, String> {
    let Some(Value::Array(inputs)) = fields.get("inputs") else {
        return Err("inputs is not a list".into());
    };
    inputs
        .iter()
        .map(|input| {
            let name = input.get("name").and_then(Value::as_str);
            let body = input.get("body").and_then(PayloadRef::from_value);
            match (name, body) {
                (Some(name), Some(body)) => Ok(OrderInput {
                    name: name.to_owned(),
                    body,
                }),
                _ => Err("an input is not a name and a body reference".into()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Expected answers come from the phase's retry decisions and the plan's
    //! test data, never from running this code.

    use baley_store::RetentionClass;
    use serde_json::json;

    use super::*;

    const I0: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const I1: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    const I2: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";

    fn h() -> Hash {
        Hash([0xab; 32])
    }

    fn d1() -> Hash {
        Hash([0x11; 32])
    }

    fn d2() -> Hash {
        Hash([0x22; 32])
    }

    fn brief() -> Vec<OrderInput> {
        vec![OrderInput {
            name: "brief".into(),
            body: PayloadRef {
                hash: h(),
                bytes: 300,
                class: RetentionClass::Record,
            },
        }]
    }

    /// O1's document in `state` at `attempt`, issued at seq 5.
    fn document(state: &str, attempt: u64) -> Value {
        json!({"id": I0, "request": I0, "parent": null, "role": "planner",
            "subject": "start", "attempt": attempt, "route": {"rung": "high"},
            "agent": "bal-planner-high",
            "instructions": [{"identity": "bal-start-planner", "version": "1",
                "hash": "c".repeat(64)}],
            "inputs": [{"name": "brief", "body": brief()[0].body.to_value()}],
            "state": state, "seq": 5, "recorded_at": "2026-10-18T09:00:00Z",
            "failure": if state == "failed" {
                json!({"reason": "no return", "seq": 6, "recorded_at": "2026-10-18T09:00:01Z"})
            } else {
                Value::Null
            },
            "drafts": if state == "answered" { json!([d1().to_hex()]) } else { json!([]) },
            "child": null})
    }

    fn attempt(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    #[test]
    fn a_retry_of_a_pending_or_answered_order_accepted_is_caught() {
        let mut late = document("failed", 1);
        late["drafts"] = json!([d1().to_hex()]);
        for (order, state) in [
            (document("pending", 1), "pending"),
            (document("answered", 1), "answered"),
            (late, "answered"),
        ] {
            assert_eq!(
                judge_retry(&order, &[], &[]),
                Err(RetryRefusal::NotFailed {
                    state: state.into()
                })
            );
        }
    }

    #[test]
    fn a_retry_of_an_order_already_retried_accepted_is_caught() {
        let mut order = document("failed", 1);
        order["child"] = json!(I2);
        assert_eq!(
            judge_retry(&order, &[], &[]),
            Err(RetryRefusal::Retried { child: I2.into() })
        );
    }

    #[test]
    fn a_retry_after_an_earlier_attempt_was_answered_accepted_is_caught() {
        let mut o3 = document("failed", 3);
        o3["id"] = json!(I2);
        o3["parent"] = json!(I1);
        let mut o2 = document("failed", 2);
        o2["id"] = json!(I1);
        o2["parent"] = json!(I0);
        o2["child"] = json!(I2);
        let mut o1 = document("failed", 1);
        o1["child"] = json!(I1);

        let mut o1_answered = o1.clone();
        o1_answered["drafts"] = json!([d1().to_hex()]);
        assert_eq!(
            judge_retry(&o3, &[o2.clone(), o1_answered], &[]),
            Err(RetryRefusal::EarlierAnswered {
                work_order: I0.into()
            })
        );

        let mut o2_answered = o2.clone();
        o2_answered["drafts"] = json!([d2().to_hex()]);
        assert_eq!(
            judge_retry(&o3, &[o2_answered, o1.clone()], &[]),
            Err(RetryRefusal::EarlierAnswered {
                work_order: I1.into()
            })
        );

        let retry = judge_retry(&o3, &[o2, o1], &[]).expect("no attempt was answered");
        assert_eq!(retry.attempt, attempt(4));
        assert_eq!(retry.parent, I2);
        assert_eq!(retry.request, I0);
    }

    #[test]
    fn a_retry_attempt_off_by_one_or_with_another_role_subject_or_input_is_caught() {
        let expected = |parent: &str, n: u32| Retry {
            request: I0.into(),
            parent: parent.into(),
            attempt: attempt(n),
            role: Role::Planner,
            subject: "start".into(),
            inputs: brief(),
        };
        assert_eq!(
            judge_retry(&document("failed", 1), &[], &[]),
            Ok(expected(I0, 2))
        );
        assert_eq!(
            judge_retry(&document("failed", 4), &[], &[]),
            Ok(expected(I0, 5))
        );
        let mut o2 = document("failed", 2);
        o2["id"] = json!(I1);
        o2["parent"] = json!(I0);
        o2["agent"] = json!("bal-planner-xhigh");
        assert_eq!(judge_retry(&o2, &[], &[]), Ok(expected(I1, 3)));
    }

    #[test]
    fn a_retry_needing_an_input_this_project_purged_accepted_is_caught() {
        let order = document("failed", 1);
        assert_eq!(
            judge_retry(&order, &[], &[(5, h())]),
            Err(RetryRefusal::MaterialUnavailable {
                input: "brief".into()
            })
        );
        assert!(judge_retry(&order, &[], &[(9, h())]).is_ok());
        assert!(judge_retry(&order, &[], &[(5, d1())]).is_ok());
    }

    #[test]
    fn a_document_that_is_not_an_order_retried_is_caught() {
        let mut stateless = document("failed", 1);
        stateless.as_object_mut().unwrap().remove("state");
        let mut unknown = document("failed", 1);
        unknown["state"] = json!("unknown");
        for order in [stateless, unknown] {
            assert!(
                matches!(
                    judge_retry(&order, &[], &[]),
                    Err(RetryRefusal::Unreadable { .. })
                ),
                "{order}"
            );
        }
    }

    #[test]
    fn an_attempt_past_the_counter_accepted_is_caught() {
        let order = document("failed", u64::from(u32::MAX));
        assert_eq!(judge_retry(&order, &[], &[]), Err(RetryRefusal::Exhausted));
    }
}
