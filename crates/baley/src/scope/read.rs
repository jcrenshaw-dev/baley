//! Scope read back from its views (design 0004 sections 5 and 6). A draft
//! is read by its digest, as the latest submission of that digest recorded
//! it, so the owner sees exactly what an approval would record; phase
//! numbers and story ids are assigned at approval, so a preview has none.
//! Accepted scope is read from the `roadmap`, `phase`, `phase_detail` and
//! `backlog` views, which only an approval changes, except the backlog's
//! record of each story's latest question set; a story's refinement state
//! is worked out from that set when read. A draft or phase detail
//! whose body this project purged answers a tombstone with this project's
//! reason and is never opened, even when another project still holds the
//! same bytes (design 0001 Views). `document` and the owner's `baley
//! scope`, `phase`, `backlog` and `story` commands read through here.
//! The accepted scope a change is judged on is read here too, with a
//! confirmation its transaction runs that none of it moved. Nothing is
//! appended.

use std::collections::BTreeMap;
use std::io::Read;

use baley_core::questions::{QUESTIONS_VIEW, questions_key, round_of, set_facts};
use baley_core::scope::{
    AcceptedScope, BACKLOG_VIEW, DRAFT_VIEW, PHASE_DETAIL_VIEW, PHASE_VIEW, PhaseProgress,
    ROADMAP_VIEW, backlog_key, draft_key, phase_detail_key, phase_key, roadmap_key,
};
use baley_core::truth::{Refinement, refinement};
use baley_store::{
    DocKey, Document, Hash, Ledger, PayloadBody, Payloads, ProjectId, StaleInput, StoreError,
    Transaction, Views,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::scope::submit::start_approved;

/// The code for a story id the accepted roadmap does not hold (PRJ-R18).
pub use baley_core::scope::NO_SUCH_STORY;

/// A draft as its latest submission recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The change kind, such as `start`.
    pub kind: String,
    /// The submission's digest.
    pub digest: Hash,
    /// The sequence of the latest submission of this digest.
    pub seq: u64,
    /// When the latest submission of this digest was recorded.
    pub recorded_at: String,
    /// The submission as recorded.
    pub submission: Value,
}

/// A draft whose body this project purged: what the view still knows of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    /// The change kind, such as `start`.
    pub kind: String,
    /// The submission's digest.
    pub digest: Hash,
    /// When the latest submission of this digest was recorded.
    pub recorded_at: String,
    /// The reason this project's purge recorded.
    pub reason: String,
}

/// A draft read by its digest: its submission, or the tombstone this
/// project's purge of its body left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Draft {
    /// The body is held, so the submission is read from it.
    Held(Preview),
    /// This project purged the body, so it is never opened.
    Purged(Tombstone),
}

/// Why a preview could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewError {
    /// No submission has this digest, answered `unknown-draft`.
    UnknownDraft,
    /// The draft is recorded, but its document or body cannot be read: a
    /// body no longer held, one that is not JSON, or, for a caller that
    /// needs the submission, one this project purged. Callers answer it
    /// `failed` `ledger-unavailable`, never as a missing draft.
    Unreadable(String),
    /// The store's own error.
    Store(StoreError),
}

/// Reads the draft with `digest` in `project`: its `draft` document, then
/// its body by hash, parsed as JSON, unless this project purged it. Nothing
/// is appended.
pub fn draft<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    digest: Hash,
) -> Result<Draft, PreviewError> {
    match store.get(project, DRAFT_VIEW, &draft_key(digest)) {
        Ok(Some(document)) => draft_of(store, &document),
        Ok(None) => Err(PreviewError::UnknownDraft),
        Err(error) => Err(PreviewError::Store(error)),
    }
}

/// Reads the draft with `digest` in `project` for a caller that needs its
/// submission, such as the approval. A draft this project purged has none.
pub fn preview<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    digest: Hash,
) -> Result<Preview, PreviewError> {
    draft(store, project, digest).and_then(held)
}

/// The preview a `draft` document stands for, its body opened through
/// `payloads` unless this project purged it.
pub(crate) fn preview_of<P: Payloads + ?Sized>(
    payloads: &P,
    document: &Document,
) -> Result<Preview, PreviewError> {
    draft_of(payloads, document).and_then(held)
}

/// A purged draft has no submission to give, so the caller is told why,
/// with this project's reason.
fn held(draft: Draft) -> Result<Preview, PreviewError> {
    match draft {
        Draft::Held(preview) => Ok(preview),
        Draft::Purged(tombstone) => Err(PreviewError::Unreadable(format!(
            "this project purged the draft's body: {}",
            tombstone.reason
        ))),
    }
}

/// The draft a `draft` document stands for. The view's purge state is
/// checked before the body is opened, since a body is stored by hash for
/// every project and another project may still hold the same bytes.
fn draft_of<P: Payloads + ?Sized>(
    payloads: &P,
    document: &Document,
) -> Result<Draft, PreviewError> {
    let unreadable = || PreviewError::Unreadable("the draft's view document cannot be read".into());
    let body = &document.body;
    let text = |name: &str| body.get(name).and_then(Value::as_str);
    let kind = text("kind").ok_or_else(unreadable)?;
    let digest = text("digest")
        .and_then(Hash::from_hex)
        .ok_or_else(unreadable)?;
    let seq = body
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or_else(unreadable)?;
    let recorded_at = text("recorded_at").ok_or_else(unreadable)?;
    let hash = text("hash")
        .and_then(Hash::from_hex)
        .ok_or_else(unreadable)?;
    match text("state") {
        None => {}
        Some("purged") => {
            return Ok(Draft::Purged(Tombstone {
                kind: kind.into(),
                digest,
                recorded_at: recorded_at.into(),
                reason: text("reason").ok_or_else(unreadable)?.into(),
            }));
        }
        Some(_) => return Err(unreadable()),
    }
    let mut reader = match payloads.open(&hash).map_err(PreviewError::Store)? {
        PayloadBody::Present(reader) => reader,
        PayloadBody::Gone(_) => {
            return Err(PreviewError::Unreadable(
                "the draft is recorded, but its body is no longer held".into(),
            ));
        }
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| PreviewError::Store(StoreError::Unavailable(error.to_string())))?;
    let submission = serde_json::from_slice(&bytes).map_err(|_| {
        PreviewError::Unreadable("the draft is recorded, but its body is not JSON".into())
    })?;
    Ok(Draft::Held(Preview {
        kind: kind.into(),
        digest,
        seq,
        recorded_at: recorded_at.into(),
        submission,
    }))
}

/// The accepted project description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Description {
    /// What the project is.
    pub what: String,
    /// The one thing it must get right.
    pub core_value: String,
    /// Its constraints, possibly none.
    pub constraints: Vec<String>,
}

/// One accepted phase as the roadmap lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoadmapPhase {
    /// Its number, assigned at approval and never reused.
    pub number: u32,
    /// Its name.
    pub name: String,
    /// Its goal.
    pub goal: String,
    /// Its status, such as `declared`.
    pub status: String,
    /// The numbers of the phases it depends on.
    pub depends_on: Vec<u32>,
    /// The ids of its stories.
    pub stories: Vec<u32>,
}

/// One accepted story as the roadmap lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Story {
    /// Its id, assigned at approval.
    pub id: u32,
    /// The story itself.
    pub sentence: String,
    /// Its status, such as `active` or `deferred`.
    pub status: String,
    /// The number of the phase it is committed to, or none in the backlog.
    pub phase: Option<u32>,
    /// How many truths it carries.
    pub truths: u64,
}

/// The accepted roadmap: the description, the phases in roadmap order and
/// the stories in priority order. A project with no approved scope has no
/// description, phase or story.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Roadmap {
    /// The project description, or none before the first approval.
    pub description: Option<Description>,
    /// The phases in roadmap order.
    pub phases: Vec<RoadmapPhase>,
    /// The stories in priority order.
    pub stories: Vec<Story>,
}

/// A phase's detail: its text, or the reason this project purged its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    /// The detail text, as approved.
    Text(String),
    /// This project purged the body, for this reason, so it is never opened.
    Purged(String),
}

/// One accepted phase with its detail, read from its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    /// Its number.
    pub number: u32,
    /// Its name.
    pub name: String,
    /// Its goal.
    pub goal: String,
    /// Its status, such as `declared`.
    pub status: String,
    /// Its detail text, or why this project no longer shows it.
    pub detail: Detail,
    /// The numbers of the phases it depends on.
    pub depends_on: Vec<u32>,
    /// The ids of its stories.
    pub stories: Vec<u32>,
}

/// One story as the backlog lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BacklogStory {
    /// Its id.
    pub id: u32,
    /// The story itself.
    pub sentence: String,
    /// Its status.
    pub status: String,
    /// The number of the phase it is committed to, or none.
    pub phase: Option<u32>,
    /// Its refinement state: `unrefined`, `questions-open`, `drafting` or
    /// `refined`, worked out from its latest question set when read.
    pub refinement: String,
    /// The round its latest set waits on while questions are open, or none.
    pub round: Option<u32>,
    /// How many truths it carries.
    pub truths: u64,
    /// The number of tasks its approved plans need, or none before planning.
    pub size: Option<u64>,
}

/// Why accepted scope could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// No accepted phase has this number, answered `no-such-phase`.
    NoSuchPhase,
    /// No accepted story has this id, answered `no-such-story`.
    NoSuchStory,
    /// The phase is recorded and its detail not purged by this project, but
    /// its body is no longer held. Callers answer it `failed` `ledger-unavailable`, never as a phase
    /// with empty or missing detail.
    BodyGone,
    /// A view document or body cannot be read as what it should hold.
    Unreadable(String),
    /// The store's own error.
    Store(StoreError),
}

/// The `roadmap` document as the projector writes it; the members a read
/// does not need are left out.
#[derive(Deserialize)]
struct RoadmapDocument {
    description: Option<Description>,
    phases: Vec<RoadmapPhase>,
    order: Vec<u32>,
    stories: Vec<Story>,
}

/// A `phase` document: its detail is found by the sequence it was declared
/// at. Its stories are read from the roadmap instead.
#[derive(Deserialize)]
struct PhaseDocument {
    number: u32,
    name: String,
    goal: String,
    status: String,
    declared_seq: u64,
    depends_on: Vec<u32>,
}

/// A `phase_detail` document: the detail body's hex hash, and this
/// project's purge state and reason once it released the body.
#[derive(Deserialize)]
struct DetailDocument {
    hash: String,
    state: Option<String>,
    reason: Option<String>,
}

/// The `backlog` document.
#[derive(Deserialize)]
struct BacklogDocument {
    stories: Vec<BacklogEntry>,
}

/// One story in the `backlog` document: its latest question set and the
/// set its latest refinement names, from which its state is worked out.
#[derive(Deserialize)]
struct BacklogEntry {
    id: u32,
    sentence: String,
    status: String,
    phase: Option<u32>,
    set: Option<String>,
    refined: Option<String>,
    truths: u64,
    size: Option<u64>,
}

/// Reads `project`'s accepted roadmap from its `roadmap` document, the
/// phases in the document's `order`, then each withdrawn phase in number
/// order, which keeps its entry but leaves the order (D-07). No document is
/// a project with no approved scope.
pub fn roadmap<S: Views + ?Sized>(store: &S, project: &ProjectId) -> Result<Roadmap, ReadError> {
    let Some(document) = read::<RoadmapDocument, _>(store, project, ROADMAP_VIEW, &roadmap_key())?
    else {
        return Ok(Roadmap {
            description: None,
            phases: Vec::new(),
            stories: Vec::new(),
        });
    };
    // Every phase not withdrawn is in the order exactly once and no
    // withdrawn phase is; anything else is a broken document, never a
    // roadmap with a phase dropped.
    let (withdrawn, held): (Vec<RoadmapPhase>, Vec<RoadmapPhase>) = document
        .phases
        .into_iter()
        .partition(|phase| phase.status == WITHDRAWN);
    if document.order.len() != held.len() {
        return Err(unreadable(ROADMAP_VIEW));
    }
    let mut phases = Vec::with_capacity(held.len() + withdrawn.len());
    for number in &document.order {
        let phase = held
            .iter()
            .find(|phase| phase.number == *number)
            .ok_or_else(|| unreadable(ROADMAP_VIEW))?;
        if phases.contains(phase) {
            return Err(unreadable(ROADMAP_VIEW));
        }
        phases.push(phase.clone());
    }
    // The projector keeps phases by number, so these are in number order.
    phases.extend(withdrawn);
    Ok(Roadmap {
        description: document.description,
        phases,
        stories: document.stories,
    })
}

/// The status a withdrawn phase holds.
const WITHDRAWN: &str = "withdrawn";

/// Reads `project`'s accepted description from its `roadmap` document, or
/// none before the start. Only an approved `project.described` replaces it,
/// so a pending `project-describe` draft never reaches it (D-16). T7's
/// refinement and T9's planning read the description through here; earlier
/// descriptions stay readable as their `project.described` events.
pub fn description<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
) -> Result<Option<Description>, ReadError> {
    Ok(
        read::<RoadmapDocument, _>(store, project, ROADMAP_VIEW, &roadmap_key())?
            .and_then(|document| document.description),
    )
}

/// Reads accepted phase `number` from the `phase` view, with its detail
/// from its body, or the tombstone its `phase_detail` document holds when
/// this project purged that body. The purge state is checked before the
/// body is opened, since a body is stored by hash for every project. Its
/// stories are the ones its `roadmap` entry says it serves now, after any
/// reassign or drop, since the `phase` document keeps the stories its latest
/// declaration listed. A withdrawn phase reads with its status.
pub fn phase<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    number: u32,
) -> Result<Phase, ReadError> {
    let document = read::<PhaseDocument, _>(store, project, PHASE_VIEW, &phase_key(number))?
        .ok_or(ReadError::NoSuchPhase)?;
    let stories = read::<RoadmapDocument, _>(store, project, ROADMAP_VIEW, &roadmap_key())?
        .and_then(|roadmap| {
            roadmap
                .phases
                .into_iter()
                .find(|phase| phase.number == number)
        })
        .map(|phase| phase.stories)
        .ok_or_else(|| unreadable(ROADMAP_VIEW))?;
    let held = read::<DetailDocument, _>(
        store,
        project,
        PHASE_DETAIL_VIEW,
        &phase_detail_key(document.declared_seq),
    )?
    .ok_or_else(|| unreadable(PHASE_DETAIL_VIEW))?;
    let detail = match (held.state.as_deref(), held.reason) {
        (None, _) => Detail::Text(detail_text(store, &held.hash)?),
        (Some("purged"), Some(reason)) => Detail::Purged(reason),
        _ => return Err(unreadable(PHASE_DETAIL_VIEW)),
    };
    Ok(Phase {
        number: document.number,
        name: document.name,
        goal: document.goal,
        status: document.status,
        detail,
        depends_on: document.depends_on,
        stories,
    })
}

/// The key and `produced_seq` of the `phase_detail` document phase
/// `number`'s detail is read under, or none when the view holds no such
/// phase or document. A purge of that detail moves the document, so a
/// transaction carrying the detail confirms it against these.
pub fn detail_document<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    number: u32,
) -> Result<Option<(DocKey, u64)>, ReadError> {
    let Some(document) = read::<PhaseDocument, _>(store, project, PHASE_VIEW, &phase_key(number))?
    else {
        return Ok(None);
    };
    let key = phase_detail_key(document.declared_seq);
    Ok(store
        .get(project, PHASE_DETAIL_VIEW, &key)
        .map_err(ReadError::Store)?
        .map(|held| (key, held.produced_seq)))
}

/// A detail body this project still holds, read as text.
fn detail_text<P: Payloads + ?Sized>(payloads: &P, hash: &str) -> Result<String, ReadError> {
    let hash = Hash::from_hex(hash).ok_or_else(|| unreadable(PHASE_DETAIL_VIEW))?;
    let mut reader = match payloads.open(&hash).map_err(ReadError::Store)? {
        PayloadBody::Present(reader) => reader,
        PayloadBody::Gone(_) => return Err(ReadError::BodyGone),
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| ReadError::Store(StoreError::Unavailable(error.to_string())))?;
    // The body was stored from the submission's UTF-8 text.
    String::from_utf8(bytes).map_err(|_| {
        ReadError::Unreadable("the phase is recorded, but its detail body is not text".into())
    })
}

/// How many times `backlog` reads the pair before a disagreement is a
/// fault. Each read again follows a write that landed between the two
/// reads, so a few cover any real interleaving.
const BACKLOG_READS: usize = 3;

/// Reads `project`'s backlog from its `backlog` document, in priority order,
/// each story's refinement state worked out from its latest set's
/// `questions` document, all those documents read in one `get_many`. The two
/// reads are separate snapshots, so when they disagree, because another
/// command replaced a story's set between them, both are read again, up to
/// `BACKLOG_READS` times. No document is an empty backlog.
pub fn backlog<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
) -> Result<Vec<BacklogStory>, ReadError> {
    for _ in 0..BACKLOG_READS {
        let Some(document) =
            read::<BacklogDocument, _>(store, project, BACKLOG_VIEW, &backlog_key())?
        else {
            return Ok(Vec::new());
        };
        let keys: Vec<DocKey> = document
            .stories
            .iter()
            .filter_map(|story| story.set.as_deref().map(questions_key))
            .collect();
        let sets = store
            .get_many(project, QUESTIONS_VIEW, &keys)
            .map_err(ReadError::Store)?
            .into_iter()
            .map(|set| set.map(|set| set.body))
            .collect();
        if let BacklogPair::Agreed(stories) = backlog_of(document, sets)? {
            return Ok(stories);
        }
    }
    Err(ReadError::Unreadable(format!(
        "the {BACKLOG_VIEW} view document and its question sets disagreed on {BACKLOG_READS} reads"
    )))
}

/// What a `backlog` document and its latest sets' documents, read one after
/// the other, say together.
#[derive(Debug, PartialEq, Eq)]
enum BacklogPair {
    /// The reads agree, so this is the backlog.
    Agreed(Vec<BacklogStory>),
    /// A story's latest set came back abandoned. Only a later set abandons
    /// a story's set, and that set would be the latest, so the backlog
    /// moved between the reads.
    Moved,
}

/// Judges a `backlog` document against the bodies of its stories' latest
/// sets, in the order the stories name them, none for a set the view does
/// not hold. A missing or unreadable set is a fault, never a moved backlog:
/// the `backlog` and `questions` views change in one transaction, so a set
/// the backlog names is already held when the sets are read.
fn backlog_of(
    document: BacklogDocument,
    sets: Vec<Option<Value>>,
) -> Result<BacklogPair, ReadError> {
    let mut sets = sets.into_iter();
    let mut stories = Vec::with_capacity(document.stories.len());
    for story in document.stories {
        let round = match &story.set {
            Some(_) => {
                let set = sets
                    .next()
                    .flatten()
                    .ok_or_else(|| unreadable(QUESTIONS_VIEW))?;
                Some(round_of(
                    &set_facts(&set).map_err(|_| unreadable(QUESTIONS_VIEW))?,
                ))
            }
            None => None,
        };
        let latest = story.set.as_deref().zip(round.as_ref());
        let Ok(state) = refinement(latest, story.refined.as_deref()) else {
            return Ok(BacklogPair::Moved);
        };
        stories.push(BacklogStory {
            id: story.id,
            sentence: story.sentence,
            status: story.status,
            phase: story.phase,
            refinement: state.as_str().into(),
            round: match state {
                Refinement::QuestionsOpen { round } => Some(round),
                _ => None,
            },
            truths: story.truths,
            size: story.size,
        });
    }
    Ok(BacklogPair::Agreed(stories))
}

/// Reads accepted story `id` from the roadmap.
pub fn story<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    id: u32,
) -> Result<Story, ReadError> {
    roadmap(store, project)?
        .stories
        .into_iter()
        .find(|story| story.id == id)
        .ok_or(ReadError::NoSuchStory)
}

/// What a scope change is judged on, read outside any transaction: whether
/// the start is approved, the `roadmap` document, and each phase's facts with
/// the sequence of the `phase` document they came from, so the change's
/// transaction can confirm none of it moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeRead {
    /// Whether the project's start is approved, read from the events.
    pub started: bool,
    /// The `roadmap` document, or none before any approval.
    pub roadmap: Option<Document>,
    /// Each phase the roadmap holds, withdrawn ones included, by number.
    pub phases: BTreeMap<u32, PhaseFacts>,
}

/// One phase's facts as PRJ-R13 judges them, and the `produced_seq` of the
/// `phase` document they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseFacts {
    /// What has happened in the phase.
    pub progress: PhaseProgress,
    /// The `phase` document's `produced_seq`.
    pub produced_seq: u64,
}

impl ScopeRead {
    /// Accepted scope as the `roadmap` document holds it, or none when the
    /// document cannot be read, which is a ledger fault.
    pub fn accepted(&self) -> Option<AcceptedScope> {
        AcceptedScope::read(self.roadmap.as_ref().map(|document| &document.body))
    }

    /// Each phase's progress by number, as the scope judgement takes it.
    pub fn progress(&self) -> BTreeMap<u32, PhaseProgress> {
        self.phases
            .iter()
            .map(|(number, facts)| (*number, facts.progress.clone()))
            .collect()
    }
}

/// Reads what a change in `project` is judged on: whether the start is
/// approved, from the events (D-05 of phase 3), then the `roadmap`
/// document, then the `phase` document of every phase it holds, in one
/// snapshot. Nothing is appended.
pub fn scope<S: Ledger + Views + ?Sized>(
    store: &S,
    project: &ProjectId,
) -> Result<ScopeRead, ReadError> {
    let started = start_approved(store, project).map_err(ReadError::Store)?;
    let roadmap = store
        .get(project, ROADMAP_VIEW, &roadmap_key())
        .map_err(ReadError::Store)?;
    let numbers = match &roadmap {
        Some(document) => phase_numbers(&document.body).ok_or_else(|| unreadable(ROADMAP_VIEW))?,
        None => Vec::new(),
    };
    let keys: Vec<DocKey> = numbers.iter().map(|number| phase_key(*number)).collect();
    let documents = store
        .get_many(project, PHASE_VIEW, &keys)
        .map_err(ReadError::Store)?;
    let mut phases = BTreeMap::new();
    for (number, document) in numbers.into_iter().zip(documents) {
        // Every phase the roadmap holds was declared, so it has a document.
        let document = document.ok_or_else(|| unreadable(PHASE_VIEW))?;
        let progress = document
            .body
            .get("status")
            .and_then(Value::as_str)
            .and_then(progress_of)
            .ok_or_else(|| unreadable(PHASE_VIEW))?;
        phases.insert(
            number,
            PhaseFacts {
                progress,
                produced_seq: document.produced_seq,
            },
        );
    }
    Ok(ScopeRead {
        started,
        roadmap,
        phases,
    })
}

/// The progress a phase's status stands for (D-13). Every status this build
/// records says neither execution nor completion, so every fact is false.
/// Builds 5 and 6 record execution and completion and add the statuses that
/// map to them here; until then an unknown status is a ledger fault.
fn progress_of(status: &str) -> Option<PhaseProgress> {
    match status {
        "declared" | "withdrawn" => Some(PhaseProgress::default()),
        _ => None,
    }
}

/// The numbers of the phases a `roadmap` document holds.
fn phase_numbers(body: &Value) -> Option<Vec<u32>> {
    body.get("phases")?
        .as_array()?
        .iter()
        .map(|phase| {
            phase
                .get("number")?
                .as_u64()
                .and_then(|number| u32::try_from(number).ok())
        })
        .collect()
}

/// Confirms inside `tx` that every document `read` named still has the
/// `produced_seq` it had, a document absent then and now counting as
/// unchanged. The first that moved is `StoreError::Stale`, so the change's
/// transaction records nothing.
pub fn confirm(tx: &mut dyn Transaction, read: &ScopeRead) -> Result<(), StoreError> {
    let seen = std::iter::once((
        ROADMAP_VIEW,
        roadmap_key(),
        read.roadmap.as_ref().map(|document| document.produced_seq),
    ))
    .chain(
        read.phases
            .iter()
            .map(|(number, facts)| (PHASE_VIEW, phase_key(*number), Some(facts.produced_seq))),
    );
    for (view, key, seen) in seen {
        let now = tx.get(view, &key)?.map(|document| document.produced_seq);
        if now != seen {
            return Err(StoreError::Stale(StaleInput::Document {
                view: view.into(),
                key,
                seen,
                now,
            }));
        }
    }
    Ok(())
}

/// One view document read as `T`, or none when the view holds no such key.
fn read<T: for<'de> Deserialize<'de>, S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    view: &str,
    key: &baley_store::DocKey,
) -> Result<Option<T>, ReadError> {
    match store.get(project, view, key).map_err(ReadError::Store)? {
        Some(document) => serde_json::from_value(document.body)
            .map(Some)
            .map_err(|_| unreadable(view)),
        None => Ok(None),
    }
}

fn unreadable(view: &str) -> ReadError {
    ReadError::Unreadable(format!("the {view} view document cannot be read"))
}

#[cfg(test)]
mod tests {
    use baley_core::scope::{
        BACKLOG_VIEW, PHASE_STATUS_INDEX, PHASE_VIEW, PhaseProgress, ROADMAP_VIEW, backlog_key,
        roadmap_key,
    };
    use baley_store::{IndexQuery, KeyValue, PageRequest};

    use super::*;
    use crate::scope::fixtures::*;

    fn digest_of(recorded: &baley_store::Recorded) -> Hash {
        Hash::from_hex(receipt(recorded)["digest"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn a_draft_lost_on_reopening_or_projected_as_accepted_scope_is_caught() {
        let (dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());

        assert!(
            store
                .get(&project(), ROADMAP_VIEW, &roadmap_key())
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get(&project(), BACKLOG_VIEW, &backlog_key())
                .unwrap()
                .is_none()
        );
        let declared = IndexQuery {
            index: PHASE_STATUS_INDEX.into(),
            equals: vec![KeyValue::Text("declared".into())],
            page: PageRequest {
                limit: 10,
                after: None,
            },
        };
        assert!(
            store
                .find(&project(), PHASE_VIEW, &declared)
                .unwrap()
                .items
                .is_empty()
        );

        drop(store);
        let reopened = crate::ledger::open::store(
            &dir.path().join("home"),
            T2,
            crate::ledger::open::options(),
        )
        .unwrap();
        let read = preview(&reopened, &project(), digest).unwrap();
        assert_eq!(read.kind, "start");
        assert_eq!(read.digest, digest);
        assert_eq!(read.recorded_at, T1);
        assert_eq!(read.submission, s1());
    }

    #[test]
    fn an_unknown_digest_read_as_an_empty_draft_is_caught() {
        let (_dir, store) = store();
        assert_eq!(
            preview(&store, &project(), Hash([0; 32])),
            Err(PreviewError::UnknownDraft)
        );
    }

    #[test]
    fn a_resubmitted_draft_read_with_its_first_time_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &m(), R1, 1, T1).unwrap());
        submit(&store, &m(), R2, 1, T2).unwrap();
        assert_eq!(preview(&store, &project(), digest).unwrap().recorded_at, T2);
    }

    /// A store holding S1, submitted at T1 and approved at T3.
    fn approved() -> (tempfile::TempDir, baley_store_sqlite::SqliteStore) {
        let (dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        approve_draft(&store, digest);
        (dir, store)
    }

    fn text(value: &str) -> String {
        value.into()
    }

    /// Approved S1's stories as the plan's Notes give them.
    fn s1_stories() -> Vec<Story> {
        let story = |id, sentence: &str, status: &str, phase| Story {
            id,
            sentence: text(sentence),
            status: text(status),
            phase,
            truths: 0,
        };
        vec![
            story(1, "Every step is recorded", "active", Some(1)),
            story(2, "The owner approves scope", "active", Some(2)),
            story(3, "Teammates see the backlog", "deferred", None),
        ]
    }

    /// Appends a `phase.reordered` giving `order` under a test command, as
    /// a later reorder would leave it.
    fn reorder(store: &baley_store_sqlite::SqliteStore, order: &[u32]) {
        use baley_core::scope::{PHASE_REORDERED, ROADMAP_STREAM, phase_reordered_payload};
        use baley_store::{Decision, NewEvent, Observed, OutcomeKind, StreamName};

        let command = baley_store::Command {
            project: project(),
            kind: baley_store::CommandKind("test.reorder".into()),
            request_id: baley_store::RequestId("4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f7a".into()),
            digest: Hash([0xdd; 32]),
            scope: vec![],
            policy_version: 0,
            recorded_at: T3.into(),
            actor: baley_store::Actor::Owner,
            caller: None,
        };
        store
            .transact(&command, &mut |tx| {
                tx.append(NewEvent {
                    stream: StreamName(ROADMAP_STREAM.into()),
                    type_name: PHASE_REORDERED.into(),
                    type_version: 1,
                    git: None,
                    payload: phase_reordered_payload(order, Hash([0x5c; 32])),
                    attachments: Vec::new(),
                })?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: serde_json::json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();
    }

    #[test]
    fn a_roadmap_read_out_of_order_or_missing_a_story_is_caught() {
        let (_dir, store) = approved();
        // The view keeps phases by number, so only an order other than
        // 1, 2 tells the recorded order from the stored one.
        reorder(&store, &[2, 1]);
        let read = roadmap(&store, &project()).unwrap();
        assert_eq!(
            read.description,
            Some(Description {
                what: text("A ledger for agent work"),
                core_value: text("Nothing moves on an unproven claim"),
                constraints: vec![text("One binary")],
            })
        );
        assert_eq!(
            read.phases,
            [
                RoadmapPhase {
                    number: 2,
                    name: text("Approval"),
                    goal: text("The owner approves"),
                    status: text("declared"),
                    depends_on: vec![1],
                    stories: vec![2],
                },
                RoadmapPhase {
                    number: 1,
                    name: text("Records"),
                    goal: text("Steps are recorded"),
                    status: text("declared"),
                    depends_on: vec![],
                    stories: vec![1],
                },
            ]
        );
        assert_eq!(read.stories, s1_stories());
    }

    #[test]
    fn a_phase_read_without_its_detail_text_or_an_unknown_phase_read_as_empty_is_caught() {
        let (_dir, store) = approved();
        assert_eq!(
            phase(&store, &project(), 2),
            Ok(Phase {
                number: 2,
                name: text("Approval"),
                goal: text("The owner approves"),
                status: text("declared"),
                detail: Detail::Text(text("Approval by digest")),
                depends_on: vec![1],
                stories: vec![2],
            })
        );
        assert_eq!(phase(&store, &project(), 3), Err(ReadError::NoSuchPhase));

        // The only reference goes, so the body is gone too: the purge is
        // still answered from the view, never as a body that went missing.
        purge_in(
            &store,
            &project(),
            detail_hash(&store, &project(), 2),
            "pasted a secret",
        );
        assert_eq!(
            phase(&store, &project(), 2).map(|read| read.detail),
            Ok(Detail::Purged(text("pasted a secret")))
        );
    }

    /// The detail body hash of accepted phase `number` in `project`.
    fn detail_hash(
        store: &baley_store_sqlite::SqliteStore,
        project: &ProjectId,
        number: u32,
    ) -> Hash {
        let held = store
            .get(project, PHASE_VIEW, &phase_key(number))
            .unwrap()
            .unwrap();
        Hash::from_hex(held.body["detail"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn a_draft_this_project_purged_opened_for_approval_from_another_projects_copy_is_caught() {
        let (_dir, store) = two_projects();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        submit_in(&store, &project_b(), &s1(), R1, 1, T1).unwrap();
        // The digest is the body's hash, and B still requires that body.
        purge_in(&store, &project(), digest, "pasted a secret");
        assert!(matches!(store.open(&digest), Ok(PayloadBody::Present(_))));

        let held = |project: &ProjectId| {
            store
                .get(project, DRAFT_VIEW, &draft_key(digest))
                .unwrap()
                .unwrap()
        };
        let Err(PreviewError::Unreadable(reason)) = preview_of(&store, &held(&project())) else {
            panic!("a draft this project purged gave its submission");
        };
        assert!(reason.contains("pasted a secret"), "{reason}");
        assert_eq!(
            preview_of(&store, &held(&project_b())).map(|read| read.submission),
            Ok(s1())
        );
    }

    #[test]
    fn a_story_read_from_a_draft_or_an_unknown_story_read_as_empty_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        assert_eq!(story(&store, &project(), 1), Err(ReadError::NoSuchStory));

        approve_draft(&store, digest);
        assert_eq!(
            story(&store, &project(), 3),
            Ok(Story {
                id: 3,
                sentence: text("Teammates see the backlog"),
                status: text("deferred"),
                phase: None,
                truths: 0,
            })
        );
        assert_eq!(story(&store, &project(), 4), Err(ReadError::NoSuchStory));
    }

    #[test]
    fn a_project_with_no_approved_scope_refused_instead_of_read_as_empty_is_caught() {
        let (_dir, store) = store();
        assert_eq!(
            roadmap(&store, &project()),
            Ok(Roadmap {
                description: None,
                phases: vec![],
                stories: vec![],
            })
        );
        assert_eq!(backlog(&store, &project()), Ok(vec![]));
    }

    #[test]
    fn a_judgement_read_taken_from_a_draft_or_missing_the_sequences_it_relied_on_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        let before = scope(&store, &project()).unwrap();
        assert!(!before.started);
        assert_eq!(before.roadmap, None);
        assert!(before.phases.is_empty());

        approve_draft(&store, digest);
        let after = scope(&store, &project()).unwrap();
        assert!(after.started);
        let held = store
            .get(&project(), ROADMAP_VIEW, &roadmap_key())
            .unwrap()
            .unwrap();
        let roadmap = after.roadmap.unwrap();
        assert_eq!(roadmap.body["order"], serde_json::json!([1, 2]));
        assert_eq!(roadmap.produced_seq, held.produced_seq);
        let numbers: Vec<u32> = after.phases.keys().copied().collect();
        assert_eq!(numbers, [1, 2]);
        for (number, facts) in &after.phases {
            assert_eq!(facts.progress, PhaseProgress::default(), "{number}");
            let document = store
                .get(&project(), PHASE_VIEW, &phase_key(*number))
                .unwrap()
                .unwrap();
            assert_eq!(facts.produced_seq, document.produced_seq, "{number}");
        }
    }

    #[test]
    fn a_confirmation_that_misses_a_moved_scope_document_is_caught() {
        use baley_store::{Decision, Observed, OutcomeKind, StoreError};

        let (_dir, store) = approved();
        let read = scope(&store, &project()).unwrap();
        let confirmed = |request_id: &str| {
            let command = command("test.confirm", request_id, 7, T3);
            store.transact(&command, &mut |tx| {
                confirm(tx, &read)?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: serde_json::json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
        };
        assert!(confirmed(R2).is_ok());

        append(&store, R3, T3, vec![story_dropped(3)]);
        let moved = confirmed("8c9d0e1f-2a3b-4c4d-9e5f-6a7b8c9d0e1f");
        assert!(matches!(moved, Err(StoreError::Stale(_))), "{moved:?}");
    }

    #[test]
    fn a_withdrawn_phase_left_out_of_the_roadmap_read_or_read_in_its_order_is_caught() {
        let (_dir, store) = approved();
        append(
            &store,
            R2,
            T4,
            vec![
                phase_withdrawn(2),
                story_reassigned(2, 1),
                phase_reordered(&[1]),
            ],
        );
        let read = roadmap(&store, &project()).unwrap();
        let listed: Vec<(u32, &str, &[u32])> = read
            .phases
            .iter()
            .map(|phase| {
                (
                    phase.number,
                    phase.status.as_str(),
                    phase.stories.as_slice(),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [(1, "declared", &[1, 2][..]), (2, "withdrawn", &[][..])]
        );
        assert_eq!(read.stories[1].id, 2);
        assert_eq!(read.stories[1].phase, Some(1));

        let withdrawn = phase(&store, &project(), 2).unwrap();
        assert_eq!(withdrawn.status, "withdrawn");
        assert_eq!(withdrawn.stories, Vec::<u32>::new());
    }

    #[test]
    fn a_phase_read_with_the_stories_of_its_old_declaration_is_caught() {
        let (_dir, store) = approved();
        append(&store, R2, T4, vec![story_reassigned(3, 2)]);
        assert_eq!(phase(&store, &project(), 2).unwrap().stories, [2, 3]);
        assert_eq!(phase(&store, &project(), 1).unwrap().stories, [1]);
    }

    #[test]
    fn a_pending_description_served_or_the_approved_one_hiding_the_earlier_is_caught() {
        use baley_core::scope::ChangeKind;

        use crate::scope::approve::{self, APPROVE_COMMAND};

        let (_dir, store) = store();
        let start = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        approve_draft(&store, start);
        let revised =
            digest_of(&submit_change(&store, ChangeKind::ProjectDescribe, &d1(), R4, T4).unwrap());
        let described = |constraints: &[&str]| {
            Ok(Some(Description {
                what: text("A ledger for agent work"),
                core_value: text("Nothing moves on an unproven claim"),
                constraints: constraints
                    .iter()
                    .map(|constraint| text(constraint))
                    .collect(),
            }))
        };
        assert_eq!(description(&store, &project()), described(&["One binary"]));

        let judged = approve::judge(&store, &project(), revised).unwrap();
        let command = command(APPROVE_COMMAND, R6, 6, T5);
        approve::record(&store, &command, Some(&judged), "Ada Owner").unwrap();
        assert_eq!(
            description(&store, &project()),
            described(&["One binary", "No server"])
        );

        let events = history(&store);
        let recorded = of_type(&events, "project.described");
        assert_eq!(recorded.len(), 2);
        assert_eq!(
            recorded[0].payload["constraints"],
            serde_json::json!(["One binary"])
        );
        assert_eq!(recorded[0].payload["submission"], start.to_hex());
        assert_eq!(
            recorded[1].payload["submission"],
            "db6830acb9a086805f20c56b6dbec837c2fa59938cfabcec77d3ed5336ec1213"
        );
    }

    #[test]
    fn a_backlog_read_out_of_priority_order_or_without_its_phases_is_caught() {
        let (_dir, store) = approved();
        let listed = |id, sentence: &str, status: &str, phase| BacklogStory {
            id,
            sentence: text(sentence),
            status: text(status),
            phase,
            refinement: text("unrefined"),
            round: None,
            truths: 0,
            size: None,
        };
        assert_eq!(
            backlog(&store, &project()),
            Ok(vec![
                listed(1, "Every step is recorded", "active", Some(1)),
                listed(2, "The owner approves scope", "active", Some(2)),
                listed(3, "Teammates see the backlog", "deferred", None),
            ])
        );
    }
    #[test]
    fn a_backlog_read_showing_a_stale_or_stored_refinement_state_is_caught() {
        use baley_core::questions::{
            QUESTION_ANSWERED, QUESTION_ANSWERED_VERSION, answered_payload, judge_set,
        };
        use baley_core::scope::ROADMAP_STREAM;
        use baley_core::truth::{STORY_REFINED, STORY_REFINED_VERSION};
        use baley_store::{NewEvent, StreamName};
        use serde_json::json;

        use crate::questions::record::tests::{open_set, q1, story};

        let (_dir, store) = approved();
        let first = open_set(&store, R2, &q1(), &story(1));
        let empty = open_set(&store, R3, &judge_set(&json!([])).unwrap(), &story(2));
        let on_roadmap = |type_name: &str, type_version, payload| NewEvent {
            stream: StreamName(ROADMAP_STREAM.into()),
            type_name: type_name.into(),
            type_version,
            git: None,
            payload,
            attachments: Vec::new(),
        };
        let refined = json!({"story": 2, "set": empty, "submission": "5c".repeat(32),
            "owner": "Ada Owner", "at": T4, "truths": [{"id": "t1", "form": "when",
                "trigger": "the owner approves a draft by a stale digest",
                "observer": "the owner", "verb": "is refused",
                "outcome": "stale-draft naming the first differing part", "kind": "literal",
                "observable": true, "fixed_oracle": true, "version": 1}]});
        append(
            &store,
            R4,
            T4,
            vec![on_roadmap(STORY_REFINED, STORY_REFINED_VERSION, refined)],
        );

        let states =
            |store: &baley_store_sqlite::SqliteStore| -> Vec<(u32, String, Option<u32>, u64)> {
                backlog(store, &project())
                    .unwrap()
                    .into_iter()
                    .map(|story| (story.id, story.refinement, story.round, story.truths))
                    .collect()
            };
        assert_eq!(
            states(&store),
            [
                (1, text("questions-open"), Some(1), 0),
                (2, text("refined"), None, 1),
                (3, text("unrefined"), None, 0),
            ]
        );

        let answered = answered_payload(&first, "q1", 1, "SQLite", true, "Ada Owner", T6);
        append(
            &store,
            R6,
            T6,
            vec![on_roadmap(
                QUESTION_ANSWERED,
                QUESTION_ANSWERED_VERSION,
                answered,
            )],
        );
        assert_eq!(states(&store)[0], (1, text("questions-open"), Some(2), 0));
    }

    #[test]
    fn a_backlog_read_across_a_replaced_set_answered_unreadable_or_a_missing_set_read_again_is_caught()
     {
        use serde_json::json;

        const S1: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
        let backlog = || -> BacklogDocument {
            serde_json::from_value(json!({"stories": [
                {"id": 1, "sentence": "Every step is recorded", "status": "active", "phase": 1,
                    "set": S1, "refined": null, "truths": 0, "size": null},
                {"id": 2, "sentence": "The owner approves scope", "status": "active",
                    "phase": 2, "set": null, "refined": null, "truths": 0, "size": null}
            ]}))
            .unwrap()
        };
        let set = |abandoned: Value| {
            json!({"questions": [{"id": "q1", "question": "Which store backs the ledger?",
                "recommended": "SQLite", "depends_on": []}],
                "answers": [], "deferrals": [], "abandoned": abandoned})
        };
        let superseded = json!({"cause": "superseded", "at": T4});

        // Set S1 replaced between the reads: the backlog moved, read again.
        assert_eq!(
            backlog_of(backlog(), vec![Some(set(superseded))]),
            Ok(BacklogPair::Moved)
        );
        assert_eq!(
            backlog_of(backlog(), vec![Some(set(Value::Null))]),
            Ok(BacklogPair::Agreed(vec![
                BacklogStory {
                    id: 1,
                    sentence: text("Every step is recorded"),
                    status: text("active"),
                    phase: Some(1),
                    refinement: text("questions-open"),
                    round: Some(1),
                    truths: 0,
                    size: None,
                },
                BacklogStory {
                    id: 2,
                    sentence: text("The owner approves scope"),
                    status: text("active"),
                    phase: Some(2),
                    refinement: text("unrefined"),
                    round: None,
                    truths: 0,
                    size: None,
                },
            ]))
        );
        // A set the backlog names is held by the same transaction, so its
        // absence is a fault, never a reason to read again.
        assert_eq!(
            backlog_of(backlog(), vec![None]),
            Err(ReadError::Unreadable(text(
                "the questions view document cannot be read"
            )))
        );
    }
}
