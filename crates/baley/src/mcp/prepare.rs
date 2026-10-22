//! Prepares a session's project read or write from the session's own
//! `CLAUDE_PROJECT_DIR`, in the order the command line already uses.
//!
//! The plan is pure: from what has been observed so far it gives the next
//! step to perform, the `failed` answer, or the prepared result. The entry
//! performs each step and asks again, so a refusal always comes before the
//! step after it. Every refusal of preparation's own is a `failed` answer
//! with `recorded: false`: preparation records nothing itself. A write may
//! also be prepared with a check of its own, asked after the replay lookup
//! and before the settings, which can end the call first with its own
//! answer, a refusal included, and records nothing either: a scope refusal
//! judged against the ledger cannot come after the steps that record
//! (design 0004 section 5). A check that ends the call is followed by a
//! second replay lookup, so a request recorded since the first is replayed
//! rather than refused. Checkout admission and the policy step
//! are separate transactions, as on the command line, so a store failure at
//! the step can follow a checkout admission that already recorded
//! `checkout.seen`. That record stands. A read may also read and judge the
//! settings as a write does, for the `route` query, and stops there with the
//! host's policy, so it too records nothing. Preparation creates no project, no
//! `project.initialized` event, no catalog seed and runs no detection: the
//! owner sets those up with `baley init` and `baley config`.

use std::path::{Path, PathBuf};

use baley_core::checkout::PROJECT_ID_CONFLICT;
use baley_core::policy::recorded::{RecordedPolicy, recorded_policy};
use baley_core::policy::{CONFIG_UNAVAILABLE, EffectivePolicy, Host, SettingsFile, Unavailable};
use baley_store::{
    Actor, Admin, Caller, Command, CommandKind, DocKey, Hash, Ledger, ProjectId, REQUEST_VIEW,
    RequestId, ServerCaller, StoreError, Views, request_key, request_state,
};
use serde_json::Value;

use crate::checkout::{self, EntryError, EntryRefusal, Site};
use crate::discovery::{self, Ancestor, Discovery, PROJECT_FILE};
use crate::envelope::{Envelope, LEDGER_BUSY};
use crate::ledger::commands::new_request_id;
use crate::policy_step::{self, Reads};
use crate::process::Process;
use crate::{init, settings};

/// The code for a project directory that cannot be walked, such as one
/// removed after the server started.
pub const PROJECT_CONTEXT_INVALID: &str = "project-context-invalid";

/// The code for a project directory that no repository with a `baley.toml`
/// holds.
pub const NOT_A_PROJECT: &str = "not-a-project";

/// The code for a project whose id the ledger does not list.
pub const PROJECT_NOT_IN_LEDGER: &str = "project-not-in-ledger";

/// The code for a ledger that cannot take the call and is not just busy.
pub const LEDGER_UNAVAILABLE: &str = "ledger-unavailable";

/// The code for a checkout whose git facts could not be gathered.
pub const CHECKOUT_FACTS_UNAVAILABLE: &str = "checkout-facts-unavailable";

const PLACE_PROJECT_DIR: &str = "CLAUDE_PROJECT_DIR";
const PLACE_SETTINGS: &str = "settings";
const PLACE_PROJECT: &str = "project";
const PLACE_LEDGER: &str = "ledger";
const PLACE_GIT: &str = "git";
const PLACE_CHECKOUT: &str = "checkout";

/// A project read or write ready to run.
///
/// Project start is the reader of the project file and of what the write's
/// steps found: it resolves its route from the host's policy at the step's
/// version (D-11) and classifies the checkout from the root commit and the
/// project file (D-20). The `route` query is the reader of the policy a
/// settings-reading read judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The project the working tree's `baley.toml` names, known to the ledger.
    pub project: ProjectId,
    /// The canonical repository root discovery found. The caller keeps the
    /// project directory text exactly as it was given.
    pub root: PathBuf,
    /// The working-tree `baley.toml` discovery found, in the discovered folder
    /// under `root`.
    pub project_file: PathBuf,
    /// The domain command to run. `Some` exactly when a write was requested.
    pub command: Option<Command>,
    /// What the write's own steps found. `Some` exactly on a write that is not
    /// a replay: a read gathers nothing, and a replay runs no step.
    pub stepped: Option<Stepped>,
    /// The host's effective policy a settings-reading read judged. `Some`
    /// exactly on that read, which records nothing, so no version comes with
    /// it (D-11).
    pub policy: Option<EffectivePolicy>,
}

/// What a write's checkout admission and policy step found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stepped {
    /// The host's effective policy, the one the policy step recorded at the
    /// command's policy version.
    pub policy: EffectivePolicy,
    /// The root commit checkout admission gathered, `None` when HEAD is unborn.
    pub root_commit: Option<String>,
}

/// The identity a served write supplies, which preparation carries into the
/// domain command unchanged.
///
/// The digest covers the operation and its meaningful input and leaves out
/// the policy version, which travels only in the command's `policy_version`.
/// So a retry after a policy edit is still a replay of the same request. The
/// command line's digests keep the version, which is safe only because its
/// request ids are always fresh. The request id is an operation argument,
/// never derived from the JSON-RPC id, which repeats after a restart and
/// across subagents sharing one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    /// The command kind, such as `capture.record`.
    pub kind: CommandKind,
    /// The request id the operation was given.
    pub request_id: RequestId,
    /// The operation's digest, with no policy version in it.
    pub digest: Hash,
}

/// The domain command a prepared write runs: the supplied kind, request id
/// and digest, an empty scope, the policy version the step returned, the
/// server time, Baley as the actor and the server caller. It never computes
/// or extends the digest.
pub fn prepared_command(
    project: &ProjectId,
    request: &WriteRequest,
    policy_version: u64,
    at: &str,
    caller: &ServerCaller,
) -> Command {
    Command {
        project: project.clone(),
        kind: request.kind.clone(),
        request_id: request.request_id.clone(),
        digest: request.digest,
        scope: Vec::new(),
        policy_version,
        recorded_at: at.into(),
        actor: Actor::Baley,
        caller: Some(Caller::Server(caller.clone())),
    }
}

/// The policies a write is judged under.
#[derive(Debug)]
struct WritePolicies {
    /// The policy built with no host. Its `git.remote` names the remote the
    /// checkout's facts come from, the one the command line uses, so the
    /// server and the command line record one remote for one checkout.
    facts: EffectivePolicy,
    /// The policy built for the call's host, which the prepared write hands
    /// to the operation.
    host: EffectivePolicy,
    /// The host's policy, recorded at the root. The policy step records it as
    /// `policy.effective` under (checkout, host), so the host's section values
    /// apply.
    recorded: RecordedPolicy,
}

/// Judges the gathered settings of a write for the call's `host` and the
/// discovered `root`. The recorded checkout is the root's text, the path
/// checkout admission keys its hostless version by. A fault, which includes a
/// root that is not UTF-8, is the `config-unavailable` text, in `build`'s
/// order.
fn judge_settings(reads: &Reads, host: Host, root: &Path) -> Result<WritePolicies, String> {
    let facts = policy_step::build(reads, None).map_err(|e| e.to_string())?;
    let with_host = policy_step::build(reads, Some(host)).map_err(|e| e.to_string())?;
    let recorded = recorded_policy(root, &with_host).map_err(|e| e.to_string())?;
    Ok(WritePolicies {
        facts,
        host: with_host,
        recorded,
    })
}

/// Why a store step could not answer.
#[derive(Debug)]
enum LedgerFault {
    /// The ledger was not opened when the server started.
    NoStore,
    /// The store returned this error.
    Store(StoreError),
    /// Gathering or admitting the checkout refused it.
    Checkout(EntryError),
}

/// What preparation has observed so far.
#[derive(Debug, Default)]
struct Seen {
    /// The project directory's ancestors, or the walk's fault text.
    ancestors: Option<Result<Vec<Ancestor>, String>>,
    /// The working-tree `baley.toml` in the discovered folder.
    working: Option<Result<Option<SettingsFile>, Unavailable>>,
    /// The ledger's project ids.
    projects: Option<Result<Vec<ProjectId>, LedgerFault>>,
    /// The body of the request document for the write's kind and request id,
    /// `None` inside when the ledger holds none.
    replay: Option<Result<Option<Value>, LedgerFault>>,
    /// The write's own check: on, or the answer the call ends with.
    checked: Option<Result<(), Value>>,
    /// The request document read again after the check refused, `None`
    /// inside when the ledger still holds none.
    relooked: Option<Result<Option<Value>, LedgerFault>>,
    /// The global file and HEAD's copy of the project file.
    reads: Option<Reads>,
    /// Checkout facts gathered and the checkout admitted: the root commit
    /// admission gathered, `None` inside for an unborn HEAD.
    admitted: Option<Result<Option<String>, LedgerFault>>,
    /// The version the policy step returned.
    stepped: Option<Result<u64, LedgerFault>>,
}

/// A write's own check, given the project preparation found: `Ok` lets
/// preparation go on, and `Err` is the answer the call ends with.
pub type Check<'a> = dyn Fn(&ProjectId) -> Result<(), Value> + 'a;

/// What the call is, as the plan needs it.
struct Call<'a> {
    caller: &'a ServerCaller,
    host: Host,
    at: &'a str,
    write: Option<&'a WriteRequest>,
    /// Whether a read also reads and judges the settings.
    reads_settings: bool,
    check: Option<&'a Check<'a>>,
}

/// One step the entry performs for the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Walk the ancestors of this directory.
    Discover(String),
    /// Read the working-tree project file at this path.
    ReadProjectFile(PathBuf),
    /// List the ledger's projects.
    ListProjects,
    /// Read the request document under this key, to see whether the write is a
    /// retry of a request the ledger already holds.
    LookUpRequest { project: ProjectId, key: DocKey },
    /// Run the write's own check for this project.
    Check { project: ProjectId },
    /// Read the request document under this key again, once the check
    /// refused, to see whether the request was recorded since the first look.
    LookUpRequestAgain { project: ProjectId, key: DocKey },
    /// Read the global file and HEAD's copy of this working-tree file.
    ReadSettings {
        root: PathBuf,
        working: SettingsFile,
    },
    /// Gather the checkout's facts and run checkout admission.
    AdmitCheckout {
        project: ProjectId,
        root: PathBuf,
        policy: EffectivePolicy,
        path: String,
    },
    /// Run the policy step for the host's recorded policy.
    RecordPolicy {
        project: ProjectId,
        recorded: RecordedPolicy,
    },
}

/// What the plan wants next.
#[derive(Debug)]
enum Next {
    Do(Step),
    Failed(Envelope<Value>),
    /// The write's own check ended the call with this answer.
    Checked(Value),
    /// Boxed, since a prepared write carries its whole command.
    Ready(Box<Prepared>),
}

fn failed(code: &str, reason: impl Into<String>, place: &str) -> Next {
    Next::Failed(Envelope::failed(code, reason.into(), place))
}

/// The answer for a store step that could not answer. Only a busy ledger is
/// retryable.
fn store_failed(error: &StoreError) -> Next {
    match error {
        StoreError::Busy => failed(
            LEDGER_BUSY,
            "the ledger is busy, so the call was not taken. Try it again",
            PLACE_LEDGER,
        ),
        error => failed(LEDGER_UNAVAILABLE, error.to_string(), PLACE_LEDGER),
    }
}

/// The answer for a step that failed. A fork and a git refusal are read
/// through the same [`EntryError::refusal`] the command line uses, and the
/// fork's text names the owner's step rather than telling the reader to run it.
fn fault_failed(fault: &LedgerFault) -> Next {
    match fault {
        LedgerFault::NoStore => failed(
            LEDGER_UNAVAILABLE,
            "the ledger could not be opened when this server started, so no project call can be taken",
            PLACE_LEDGER,
        ),
        LedgerFault::Store(error) => store_failed(error),
        LedgerFault::Checkout(error) => match error.refusal() {
            EntryRefusal::Git(text) => failed(CHECKOUT_FACTS_UNAVAILABLE, text, PLACE_GIT),
            EntryRefusal::Fork(conflict) => failed(
                PROJECT_ID_CONFLICT,
                format!(
                    "the checkout at {} has the remote {}, but the project already has a checkout at {} with the remote {}, so this one is a fork. The owner gives this checkout its own project by running `baley init --new-id` in it",
                    conflict.path,
                    conflict.remote_url,
                    conflict.other_path,
                    conflict.other_remote_url
                ),
                PLACE_CHECKOUT,
            ),
            EntryRefusal::Store(error) => store_failed(error),
        },
    }
}

/// The next step for the call, or its answer. Discovery starts from the
/// project directory the caller holds and from nothing else. A read stops at
/// the known-project check, and a settings-reading read after the settings
/// are judged. A write goes on, in the command line's order, to
/// the replay lookup, its own check when it has one, then the settings,
/// checkout admission and the policy step.
fn next(call: &Call, seen: &Seen) -> Next {
    let caller = call.caller;
    let Some(ancestors) = &seen.ancestors else {
        return Next::Do(Step::Discover(caller.project_directory().into()));
    };
    let ancestors = match ancestors {
        Ok(ancestors) => ancestors,
        Err(fault) => {
            return failed(
                PROJECT_CONTEXT_INVALID,
                format!(
                    "{PLACE_PROJECT_DIR} {} cannot be read as the project: {fault}",
                    caller.project_directory()
                ),
                PLACE_PROJECT_DIR,
            );
        }
    };
    let (folder, root) = match discovery::discover(ancestors) {
        Discovery::Managed { folder, root } => (folder, root),
        Discovery::Unmanaged { .. } | Discovery::Outside => {
            return failed(
                NOT_A_PROJECT,
                format!(
                    "{PLACE_PROJECT_DIR} {} is not in a project: no {PROJECT_FILE} was found in its repository",
                    caller.project_directory()
                ),
                PLACE_PROJECT_DIR,
            );
        }
    };
    let file = folder.join(PROJECT_FILE);
    let Some(working) = &seen.working else {
        return Next::Do(Step::ReadProjectFile(file));
    };
    let project = match init::observe_file(working.clone()) {
        Ok(Some(identity)) => ProjectId(identity.id),
        Ok(None) => return unavailable_file(&file),
        Err(fault) => return failed(CONFIG_UNAVAILABLE, fault.to_string(), PLACE_SETTINGS),
    };
    let Some(projects) = &seen.projects else {
        return Next::Do(Step::ListProjects);
    };
    match projects {
        Err(fault) => return fault_failed(fault),
        Ok(known) if !known.contains(&project) => {
            return failed(
                PROJECT_NOT_IN_LEDGER,
                format!(
                    "project {} is not in this machine's ledger. The owner ties this checkout to the ledger by running `baley init` in it",
                    project.0
                ),
                PLACE_PROJECT,
            );
        }
        Ok(_) => {}
    }
    if call.write.is_none() && !call.reads_settings {
        return Next::Ready(Box::new(Prepared {
            project,
            root,
            project_file: file,
            command: None,
            stepped: None,
            policy: None,
        }));
    }
    let Some(working) = working.as_ref().ok().and_then(Option::as_ref) else {
        return unavailable_file(&file);
    };
    let found = Found {
        project,
        root,
        file,
    };
    let Some(request) = call.write else {
        return match judged_settings(call, seen, &found, working) {
            Ok(policies) => Next::Ready(Box::new(Prepared {
                project: found.project,
                root: found.root,
                project_file: found.file,
                command: None,
                stepped: None,
                policy: Some(policies.host),
            })),
            Err(next) => *next,
        };
    };
    write_next(call, request, seen, found, working)
}

/// The settings step a write and a settings-reading read share: the read of
/// both files, then their judgement for the call's host. `Err` is the step to
/// perform or the `config-unavailable` answer, boxed since a step is large.
fn judged_settings(
    call: &Call,
    seen: &Seen,
    found: &Found,
    working: &SettingsFile,
) -> Result<WritePolicies, Box<Next>> {
    let Some(reads) = &seen.reads else {
        return Err(Box::new(Next::Do(Step::ReadSettings {
            root: found.root.clone(),
            working: working.clone(),
        })));
    };
    judge_settings(reads, call.host, &found.root)
        .map_err(|fault| Box::new(failed(CONFIG_UNAVAILABLE, fault, PLACE_SETTINGS)))
}

/// Where discovery found the project a write is prepared for.
#[derive(Clone)]
struct Found {
    project: ProjectId,
    root: PathBuf,
    file: PathBuf,
}

impl Found {
    /// The prepared call for `command`, carrying `stepped` when the write's
    /// steps ran.
    fn ready(self, command: Command, stepped: Option<Stepped>) -> Next {
        Next::Ready(Box::new(Prepared {
            project: self.project,
            root: self.root,
            project_file: self.file,
            command: Some(command),
            stepped,
            policy: None,
        }))
    }
}

/// The write's steps after the known-project check: the replay lookup, the
/// write's own check when it has one, then the settings, checkout admission
/// and the policy step.
fn write_next(
    call: &Call,
    request: &WriteRequest,
    seen: &Seen,
    found: Found,
    working: &SettingsFile,
) -> Next {
    // A request the ledger already holds is answered by its own transaction, so
    // no implicit write is made for it and the settings are not read, which
    // keeps a retry after the owner broke `baley.toml` a replay. The write's
    // own check is skipped too, so a retry of a completed request gets its
    // recorded answer whatever the check would say now. The replay's
    // command carries version 0: it appends nothing, and the version stays out
    // of the digest.
    let replay = prepared_command(&found.project, request, 0, call.at, call.caller);
    let Some(looked_up) = &seen.replay else {
        return Next::Do(Step::LookUpRequest {
            key: request_key(&replay),
            project: found.project,
        });
    };
    if let Some(end) = held(looked_up, &found, &replay) {
        return end;
    }
    // Checked before anything records, so a scope refusal judged against the
    // ledger leaves no checkout admission or policy step behind it.
    if call.check.is_some() {
        match &seen.checked {
            None => {
                return Next::Do(Step::Check {
                    project: found.project,
                });
            }
            Some(Err(answer)) => {
                // The original of a retry can commit between the lookup and
                // the check, and the check may refuse because of it, such as
                // its own start approved since. So the request is looked up
                // again before a refusal is answered. A passing check needs
                // no second look: the operation's transaction replays a
                // request recorded since.
                let Some(relooked) = &seen.relooked else {
                    return Next::Do(Step::LookUpRequestAgain {
                        key: request_key(&replay),
                        project: found.project,
                    });
                };
                return held(relooked, &found, &replay)
                    .unwrap_or_else(|| Next::Checked(answer.clone()));
            }
            Some(Ok(())) => {}
        }
    }
    let policies = match judged_settings(call, seen, &found, working) {
        Ok(policies) => policies,
        Err(next) => return *next,
    };
    let root_commit = match &seen.admitted {
        None => {
            return Next::Do(Step::AdmitCheckout {
                project: found.project,
                path: policies.recorded.checkout.clone(),
                policy: policies.facts,
                root: found.root,
            });
        }
        Some(Err(fault)) => return fault_failed(fault),
        Some(Ok(root_commit)) => root_commit,
    };
    match &seen.stepped {
        None => Next::Do(Step::RecordPolicy {
            project: found.project,
            recorded: policies.recorded,
        }),
        Some(Err(fault)) => fault_failed(fault),
        Some(Ok(version)) => {
            let command = prepared_command(&found.project, request, *version, call.at, call.caller);
            let stepped = Stepped {
                policy: policies.host,
                root_commit: root_commit.clone(),
            };
            found.ready(command, Some(stepped))
        }
    }
}

/// What a replay lookup found decides the call when the ledger holds a record
/// of the request: its replay, or a fault. `None` when it holds none. A replay
/// carries nothing a step found, since it runs none.
fn held(
    looked_up: &Result<Option<Value>, LedgerFault>,
    found: &Found,
    replay: &Command,
) -> Option<Next> {
    match looked_up {
        Err(fault) => Some(fault_failed(fault)),
        Ok(Some(body)) if request_state(body).is_some() => {
            Some(found.clone().ready(replay.clone(), None))
        }
        Ok(Some(_)) => Some(failed(
            LEDGER_UNAVAILABLE,
            "the ledger holds a record of this request that cannot be read",
            PLACE_LEDGER,
        )),
        Ok(None) => None,
    }
}

/// The answer for a project file that was there at the walk and is gone.
fn unavailable_file(file: &Path) -> Next {
    failed(
        CONFIG_UNAVAILABLE,
        format!("{CONFIG_UNAVAILABLE}: {} was not found", file.display()),
        PLACE_SETTINGS,
    )
}

/// Prepares a project read, or a write when `write` is given, for the session
/// whose caller is given.
///
/// It finds the project fresh on every call from the caller's project
/// directory, reads the working-tree project file for its id and checks that
/// the ledger lists it. A read stops there: it validates no settings, gathers
/// no checkout facts and appends nothing. A write then looks its request up in
/// the ledger, and a request the ledger already holds is returned at once with
/// policy version 0, whatever the settings now are, since the domain
/// transaction keeps the final say on a replay. Otherwise it reads and validates
/// the settings, gathers the checkout's facts through `process` and admits
/// the checkout, runs the policy step for `host`, and returns the domain
/// command carrying the version the step returned. Checkout admission and the
/// policy step each record as Baley with the server caller at `at`, in their
/// own transactions. `config` is the config folder, where the global file is.
/// `store` is `None` when the ledger was not opened at startup.
///
/// Every prepared call carries the project file discovery found. A write that
/// ran its steps also carries the host's policy the step recorded and the root
/// commit admission gathered, which project start resolves its route from at
/// the step's version (D-11) and classifies the checkout by (D-20).
///
/// A refusal is the `failed` answer, boxed to keep the result small.
pub fn prepare<S: Admin + Views + Ledger>(
    store: Option<&S>,
    caller: &ServerCaller,
    write: Option<&WriteRequest>,
    config: &Path,
    host: Host,
    at: &str,
    process: &mut dyn Process,
) -> Result<Prepared, Box<Envelope<Value>>> {
    let call = Call {
        caller,
        host,
        at,
        write,
        check: None,
        reads_settings: false,
    };
    match drive(store, &call, config, process) {
        Ok(prepared) => Ok(prepared),
        Err(Ended::Failed(answer)) => Err(answer),
        // With no check there is no check step to end the call.
        Err(Ended::Checked(_)) => unreachable!("a write with no check was ended by one"),
    }
}

/// Prepares `write` as [`prepare`] does, with `check` asked once the replay
/// lookup finds no record of the request and before the settings are read.
/// So a check's answer comes before checkout admission and the policy step,
/// and the call records nothing. A request the ledger already holds skips
/// the check and is answered from its record by the operation's
/// transaction, whatever the check would answer now (D-18). When the check
/// ends the call, the request is looked up again first, and one recorded
/// since the first lookup is replayed the same way.
///
/// Any end is the answer's JSON value: a check's answer as it is, refusals
/// keeping their slot and details, and preparation's own faults as the
/// `failed` answers [`prepare`] gives.
#[allow(clippy::too_many_arguments)]
pub fn prepare_checked<S: Admin + Views + Ledger>(
    store: Option<&S>,
    caller: &ServerCaller,
    write: &WriteRequest,
    config: &Path,
    host: Host,
    at: &str,
    process: &mut dyn Process,
    check: &Check<'_>,
) -> Result<Prepared, Value> {
    let call = Call {
        caller,
        host,
        at,
        write: Some(write),
        check: Some(check),
        reads_settings: false,
    };
    drive(store, &call, config, process).map_err(|ended| match ended {
        Ended::Failed(answer) => serde_json::to_value(*answer).expect("a failed answer serializes"),
        Ended::Checked(answer) => answer,
    })
}

/// Prepares a project read that also reads and judges the settings, by the
/// same settings read and judgement a write runs, and is ready with the
/// host's effective policy in [`Prepared::policy`]. It never looks up a
/// request, runs a check, admits the checkout or runs the policy step, so it
/// records nothing and carries no command (D-11). Any end is preparation's
/// own `failed` answer as a JSON value.
pub fn prepare_settings_read<S: Admin + Views + Ledger>(
    store: Option<&S>,
    caller: &ServerCaller,
    config: &Path,
    host: Host,
    at: &str,
    process: &mut dyn Process,
) -> Result<Prepared, Value> {
    let call = Call {
        caller,
        host,
        at,
        write: None,
        reads_settings: true,
        check: None,
    };
    drive(store, &call, config, process).map_err(|ended| match ended {
        Ended::Failed(answer) => serde_json::to_value(*answer).expect("a failed answer serializes"),
        // A read has no check step to end the call.
        Ended::Checked(answer) => answer,
    })
}

/// How a preparation ended without a result.
enum Ended {
    /// Preparation's own `failed` answer.
    Failed(Box<Envelope<Value>>),
    /// The write's own check's answer.
    Checked(Value),
}

/// Performs the plan's steps until it is ready or ends.
fn drive<S: Admin + Views + Ledger>(
    store: Option<&S>,
    call: &Call,
    config: &Path,
    process: &mut dyn Process,
) -> Result<Prepared, Ended> {
    let mut seen = Seen::default();
    loop {
        match next(call, &seen) {
            Next::Ready(prepared) => return Ok(*prepared),
            Next::Failed(answer) => return Err(Ended::Failed(Box::new(answer))),
            Next::Checked(answer) => return Err(Ended::Checked(answer)),
            Next::Do(step) => perform(step, store, call, config, process, &mut seen),
        }
    }
}

/// Performs one step and records what it observed. It owns no policy.
fn perform<S: Admin + Views + Ledger>(
    step: Step,
    store: Option<&S>,
    call: &Call,
    config: &Path,
    process: &mut dyn Process,
    seen: &mut Seen,
) {
    let server = || Some(Caller::Server(call.caller.clone()));
    match step {
        Step::Discover(directory) => {
            seen.ancestors =
                Some(discovery::ancestors(Path::new(&directory)).map_err(|e| e.to_string()));
        }
        Step::ReadProjectFile(path) => seen.working = Some(settings::read(&path)),
        Step::ListProjects => {
            seen.projects = Some(match store {
                None => Err(LedgerFault::NoStore),
                Some(store) => store
                    .projects()
                    .map(|listed| listed.into_iter().map(|(id, _)| id).collect())
                    .map_err(LedgerFault::Store),
            });
        }
        Step::LookUpRequest { project, key } => {
            seen.replay = Some(look_up(store, &project, &key));
        }
        Step::Check { project } => {
            seen.checked = Some(call.check.map_or(Ok(()), |check| check(&project)));
        }
        Step::LookUpRequestAgain { project, key } => {
            seen.relooked = Some(look_up(store, &project, &key));
        }
        Step::ReadSettings { root, working } => {
            seen.reads = Some(policy_step::gather(config, &root, Some(&working), process));
        }
        Step::AdmitCheckout {
            project,
            root,
            policy,
            path,
        } => {
            // Only the root commit is kept: project start classifies the
            // checkout from it.
            seen.admitted = Some(match store {
                None => Err(LedgerFault::NoStore),
                Some(store) => {
                    let site = Site {
                        root: &root,
                        policy: &policy,
                        path: &path,
                    };
                    checkout::gather_and_admit(
                        store,
                        &project,
                        &site,
                        process,
                        new_request_id(),
                        call.at,
                        server(),
                    )
                    .map(|facts| facts.root_commit)
                    .map_err(LedgerFault::Checkout)
                }
            });
        }
        Step::RecordPolicy { project, recorded } => {
            seen.stepped = Some(match store {
                None => Err(LedgerFault::NoStore),
                Some(store) => policy_step::step(
                    store,
                    &project,
                    &recorded,
                    new_request_id(),
                    call.at,
                    server(),
                )
                .map_err(LedgerFault::Store),
            });
        }
    }
}

/// Reads the request document under `key`, its body or `None`.
fn look_up<S: Views>(
    store: Option<&S>,
    project: &ProjectId,
    key: &DocKey,
) -> Result<Option<Value>, LedgerFault> {
    match store {
        None => Err(LedgerFault::NoStore),
        Some(store) => store
            .get(project, REQUEST_VIEW, key)
            .map(|found| found.map(|document| document.body))
            .map_err(LedgerFault::Store),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SESSION: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";
    const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";

    fn caller(project: &str, working: &str) -> ServerCaller {
        ServerCaller::new(project, working, "claude-code", SESSION, &json!(7)).unwrap()
    }

    fn at(path: &str, has_project_file: bool, has_git: bool) -> Ancestor {
        Ancestor {
            path: path.into(),
            has_project_file,
            has_git,
        }
    }

    /// A project at `/real/r`, reached through the nested folder `/real/r/p`.
    fn managed() -> Vec<Ancestor> {
        vec![
            at("/real/r/p", false, false),
            at("/real/r", true, true),
            at("/", false, false),
        ]
    }

    fn project_file(id: &str) -> Result<Option<SettingsFile>, Unavailable> {
        let text = format!("[project]\nid = \"{id}\"\nname = \"r\"\n");
        Ok(Some(settings::file(
            Path::new("/real/r/baley.toml"),
            text.into_bytes(),
        )))
    }

    fn walked() -> Seen {
        Seen {
            ancestors: Some(Ok(managed())),
            ..Seen::default()
        }
    }

    fn with_file(mut seen: Seen) -> Seen {
        seen.working = Some(project_file(ID));
        seen
    }

    fn known(mut seen: Seen, ids: &[&str]) -> Seen {
        seen.projects = Some(Ok(ids.iter().map(|id| ProjectId((*id).into())).collect()));
        seen
    }

    fn answer(next: Next) -> Value {
        match next {
            Next::Failed(answer) => serde_json::to_value(answer).unwrap(),
            other => panic!("expected a failed answer, got {other:?}"),
        }
    }

    fn assert_failed(next: Next, code: &str, place: &str, retryable: bool) -> Value {
        let value = answer(next);
        assert_eq!(value["status"], "failed", "{value}");
        assert_eq!(value["code"], code, "{value}");
        assert_eq!(value["place"], place, "{value}");
        assert_eq!(value["recorded"], false, "{value}");
        assert_eq!(value["retryable"], retryable, "{value}");
        value
    }

    #[test]
    fn discovery_that_starts_from_the_working_directory_instead_of_the_project_directory() {
        let next = next(&read_call(&caller("/p", "/w")), &Seen::default());
        assert!(
            matches!(next, Next::Do(Step::Discover(ref d)) if d == "/p"),
            "{next:?}"
        );
    }

    #[test]
    fn a_read_asks_for_the_file_then_the_ledger_and_nothing_else() {
        let caller = caller("/real/r/p", "/w");
        let next_step = |seen: &Seen| match next(&read_call(&caller), seen) {
            Next::Do(step) => step,
            other => panic!("expected a step, got {other:?}"),
        };
        assert_eq!(
            next_step(&walked()),
            Step::ReadProjectFile("/real/r/baley.toml".into())
        );
        assert_eq!(next_step(&with_file(walked())), Step::ListProjects);
        assert!(matches!(
            next(&read_call(&caller), &known(with_file(walked()), &[ID])),
            Next::Ready(_)
        ));
    }

    #[test]
    fn a_prepared_read_whose_root_is_the_callers_text_instead_of_the_canonical_root() {
        let caller = caller("/link/to/r/p", "/w");
        let next = next(&read_call(&caller), &known(with_file(walked()), &[ID]));
        match next {
            Next::Ready(prepared) => {
                assert_eq!(prepared.root, Path::new("/real/r"));
                assert_eq!(prepared.project, ProjectId(ID.into()));
            }
            other => panic!("expected a prepared read, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_project_that_is_prepared_or_answered_as_anything_but_not_in_ledger() {
        let seen = known(
            with_file(walked()),
            &["00000000-0000-4000-8000-000000000000"],
        );
        let value = assert_failed(
            next(&read_call(&caller("/real/r/p", "/w")), &seen),
            "project-not-in-ledger",
            "project",
            false,
        );
        let reason = value["reason"].as_str().unwrap();
        assert!(reason.contains("baley init"), "{reason}");
        assert!(reason.contains(ID), "{reason}");
    }

    #[test]
    fn a_walk_fault_that_is_not_project_context_invalid_at_the_project_directory() {
        let seen = Seen {
            ancestors: Some(Err("No such file or directory (os error 2)".into())),
            ..Seen::default()
        };
        assert_failed(
            next(&read_call(&caller("/gone", "/w")), &seen),
            "project-context-invalid",
            "CLAUDE_PROJECT_DIR",
            false,
        );
    }

    #[test]
    fn an_unmanaged_or_outside_directory_that_is_not_answered_as_not_a_project() {
        for ancestors in [
            vec![at("/r/p", false, false), at("/r", false, true)],
            vec![at("/r/p", false, false), at("/", false, false)],
        ] {
            let seen = Seen {
                ancestors: Some(Ok(ancestors)),
                ..Seen::default()
            };
            assert_failed(
                next(&read_call(&caller("/r/p", "/w")), &seen),
                "not-a-project",
                "CLAUDE_PROJECT_DIR",
                false,
            );
        }
    }

    #[test]
    fn a_project_file_fault_that_is_not_config_unavailable_at_settings() {
        let unreadable = Err(Unavailable {
            path: "/real/r/baley.toml".into(),
            fault: baley_core::policy::Fault::Unreadable {
                cause: "Permission denied (os error 13)".into(),
            },
        });
        let no_id = Ok(Some(settings::file(
            Path::new("/real/r/baley.toml"),
            b"[project]\nname = \"r\"\n".to_vec(),
        )));
        for read in [unreadable, no_id, Ok(None)] {
            let mut seen = walked();
            seen.working = Some(read);
            let value = assert_failed(
                next(&read_call(&caller("/real/r/p", "/w")), &seen),
                "config-unavailable",
                "settings",
                false,
            );
            assert!(
                value["reason"]
                    .as_str()
                    .unwrap()
                    .contains("/real/r/baley.toml"),
                "{value}"
            );
        }
    }

    #[test]
    fn a_busy_store_that_is_not_retryable_or_another_fault_that_is() {
        let ask = |fault: LedgerFault| {
            let mut seen = with_file(walked());
            seen.projects = Some(Err(fault));
            next(&read_call(&caller("/real/r/p", "/w")), &seen)
        };
        assert_failed(
            ask(LedgerFault::Store(StoreError::Busy)),
            "ledger-busy",
            "ledger",
            true,
        );
        let unavailable = StoreError::Unavailable("the ledger is fenced".into());
        let value = assert_failed(
            ask(LedgerFault::Store(unavailable)),
            "ledger-unavailable",
            "ledger",
            false,
        );
        assert!(
            value["reason"].as_str().unwrap().contains("fenced"),
            "{value}"
        );
        assert_failed(
            ask(LedgerFault::NoStore),
            "ledger-unavailable",
            "ledger",
            false,
        );
    }

    const HEAD_PATH: &str = "/real/r/baley.toml";

    fn reads(head_text: &str) -> Reads {
        Reads {
            global: Ok(None),
            head: Some(Ok(crate::committed::Committed {
                layer: Some(settings::file(
                    Path::new(HEAD_PATH),
                    head_text.as_bytes().to_vec(),
                )),
                pending: None,
            })),
        }
    }

    /// The file sets a remote and a value at the top level, and the host
    /// section sets another of each.
    const SECTIONED: &str = "escalate_on_failure = false\n[git]\nremote = \"upstream\"\n[host.claude-code]\nescalate_on_failure = true\ngit.remote = \"other\"\n";

    fn judged() -> WritePolicies {
        judge_settings(&reads(SECTIONED), Host::ClaudeCode, Path::new("/real/r")).unwrap()
    }

    fn remote(policy: &EffectivePolicy) -> Option<&baley_core::policy::Value> {
        policy.settings["git.remote"].value.as_ref()
    }

    #[test]
    fn a_facts_policy_built_with_the_host_names_the_hosts_remote_instead_of_the_files() {
        let judged = judged();
        assert_eq!(
            remote(&judged.facts),
            Some(&baley_core::policy::Value::RemoteName("upstream".into()))
        );
        assert_eq!(judged.facts.host, None);
    }

    #[test]
    fn a_recorded_policy_that_is_not_keyed_to_the_host_or_lacks_its_section_values() {
        use baley_core::policy::recorded::effective_payload;
        let payload = effective_payload(&judged().recorded, "p", 0);
        assert_eq!(payload["host"], "claude-code");
        assert_eq!(payload["values"]["escalate_on_failure"], true);
        assert_eq!(payload["values"]["git.remote"], "other");
    }

    #[test]
    fn a_recorded_checkout_that_differs_from_the_discovered_root() {
        assert_eq!(judged().recorded.checkout, "/real/r");
    }

    #[test]
    fn an_invalid_heads_copy_that_is_not_config_unavailable() {
        let refusal = judge_settings(
            &reads("escalate_on_failure = \"yes\"\n"),
            Host::ClaudeCode,
            Path::new("/real/r"),
        )
        .unwrap_err();
        assert!(refusal.starts_with("config-unavailable: "), "{refusal}");
        assert!(
            refusal.contains("HEAD's copy of /real/r/baley.toml"),
            "{refusal}"
        );
    }

    #[test]
    fn a_root_that_is_not_utf8_that_is_not_config_unavailable() {
        use std::os::unix::ffi::OsStrExt;
        let root = Path::new(std::ffi::OsStr::from_bytes(b"/real/\xff"));
        let refusal = judge_settings(&reads(SECTIONED), Host::ClaudeCode, root).unwrap_err();
        assert!(refusal.starts_with("config-unavailable: "), "{refusal}");
        assert!(refusal.contains("is not UTF-8"), "{refusal}");
    }

    const AT: &str = "2026-10-04T09:30:00Z";
    const REQUEST: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";

    fn write_request(digest: u8) -> WriteRequest {
        WriteRequest {
            kind: CommandKind("capture.record".into()),
            request_id: RequestId(REQUEST.into()),
            digest: Hash::from_hex(&format!("{digest:02x}").repeat(32)).unwrap(),
        }
    }

    fn command(request: &WriteRequest, version: u64) -> Command {
        prepared_command(
            &ProjectId(ID.into()),
            request,
            version,
            AT,
            &caller("/real/r/p", "/w"),
        )
    }

    #[test]
    fn a_prepared_command_attributed_to_the_owner_instead_of_baley() {
        assert_eq!(command(&write_request(1), 3).actor, Actor::Baley);
    }

    #[test]
    fn a_prepared_command_that_carries_another_version_or_digest_than_supplied() {
        let request = write_request(1);
        let first = command(&request, 3);
        let second = command(&request, 9);
        assert_eq!(first.policy_version, 3);
        assert_eq!(second.policy_version, 9);
        assert_eq!(first.digest, request.digest);
        assert_eq!(
            second.digest, first.digest,
            "the version leaked into the digest"
        );
        assert_ne!(command(&write_request(2), 3).digest, first.digest);
    }

    #[test]
    fn a_prepared_command_recorded_at_another_time_than_the_supplied_one() {
        assert_eq!(command(&write_request(1), 3).recorded_at, AT);
    }

    #[test]
    fn a_prepared_command_without_the_server_caller() {
        let prepared = command(&write_request(1), 3);
        assert_eq!(
            prepared.caller,
            Some(Caller::Server(caller("/real/r/p", "/w")))
        );
    }

    #[test]
    fn a_prepared_command_whose_request_id_is_the_calls_identity_instead_of_the_supplied_id() {
        let prepared = command(&write_request(1), 3);
        assert_eq!(prepared.request_id, RequestId(REQUEST.into()));
        assert_eq!(prepared.kind, CommandKind("capture.record".into()));
        assert!(prepared.scope.is_empty());
    }

    fn read_call(caller: &ServerCaller) -> Call<'_> {
        Call {
            caller,
            host: Host::ClaudeCode,
            at: AT,
            write: None,
            check: None,
            reads_settings: false,
        }
    }

    fn write_call<'a>(caller: &'a ServerCaller, request: &'a WriteRequest) -> Call<'a> {
        Call {
            write: Some(request),
            ..read_call(caller)
        }
    }

    /// The next step of a write for the known project, given what was seen.
    fn write_step(seen: &Seen) -> Next {
        let caller = caller("/real/r/p", "/w");
        let request = write_request(1);
        next(&write_call(&caller, &request), seen)
    }

    fn known_project() -> Seen {
        known(with_file(walked()), &[ID])
    }

    /// A known project whose write found no request document in the ledger.
    fn looked_up() -> Seen {
        let mut seen = known_project();
        seen.replay = Some(Ok(None));
        seen
    }

    fn with_reads(mut seen: Seen, text: &str) -> Seen {
        seen.reads = Some(reads(text));
        seen
    }

    /// The root commit admission gathered, A.
    const ROOT_COMMIT: &str = "1111111111111111111111111111111111111111";

    fn admitted(mut seen: Seen) -> Seen {
        seen.admitted = Some(Ok(Some(ROOT_COMMIT.into())));
        seen
    }

    fn git_fault() -> LedgerFault {
        LedgerFault::Checkout(EntryError::Gather(
            "git remote get-url upstream exited with code 2".into(),
        ))
    }

    fn fork_fault() -> LedgerFault {
        LedgerFault::Checkout(EntryError::Admit(checkout::AdmitError::Fork(
            baley_core::checkout::ProjectIdConflict {
                path: "/real/r".into(),
                remote_url: "https://example.test/mine".into(),
                other_path: "/other/r".into(),
                other_remote_url: "https://example.test/theirs".into(),
            },
        )))
    }

    #[test]
    fn a_write_that_requests_settings_checkout_admission_and_the_step_out_of_order() {
        let Next::Do(Step::ReadSettings { root, .. }) = write_step(&looked_up()) else {
            panic!("an unknown request reads the settings first");
        };
        assert_eq!(root, Path::new("/real/r"));

        let Next::Do(Step::AdmitCheckout {
            policy,
            path,
            root,
            project,
        }) = write_step(&with_reads(looked_up(), SECTIONED))
        else {
            panic!("settings read, so checkout admission is next");
        };
        assert_eq!(policy.host, None, "facts come from the no-host policy");
        assert_eq!(path, "/real/r");
        assert_eq!(root, Path::new("/real/r"));
        assert_eq!(project, ProjectId(ID.into()));

        let Next::Do(Step::RecordPolicy { recorded, .. }) =
            write_step(&admitted(with_reads(looked_up(), SECTIONED)))
        else {
            panic!("checkout admitted, so the policy step is next, not the command");
        };
        assert_eq!(recorded.host, Some(Host::ClaudeCode));
    }

    #[test]
    fn a_read_that_requests_the_settings_checkout_admission_or_the_step() {
        let caller = caller("/real/r/p", "/w");
        let Next::Ready(prepared) = next(&read_call(&caller), &known_project()) else {
            panic!("a read is ready at the known-project check");
        };
        assert_eq!(prepared.command, None);
        assert_eq!(prepared.root, Path::new("/real/r"));
    }

    #[test]
    fn a_settings_refusal_that_still_requests_checkout_admission() {
        let seen = with_reads(looked_up(), "escalate_on_failure = \"yes\"\n");
        assert_failed(write_step(&seen), "config-unavailable", "settings", false);
    }

    #[test]
    fn a_fork_or_git_refusal_that_still_requests_the_step() {
        for fault in [git_fault(), fork_fault()] {
            let mut seen = with_reads(looked_up(), SECTIONED);
            seen.admitted = Some(Err(fault));
            assert!(
                matches!(write_step(&seen), Next::Failed(_)),
                "a refused checkout admission must end the preparation"
            );
        }
    }

    #[test]
    fn a_command_that_does_not_carry_the_version_the_step_returned() {
        let mut seen = admitted(with_reads(looked_up(), SECTIONED));
        seen.stepped = Some(Ok(5));
        let Next::Ready(prepared) = write_step(&seen) else {
            panic!("the step's version is observed, so the command is ready");
        };
        let command = prepared.command.unwrap();
        assert_eq!(command.policy_version, 5);
        assert_eq!(command.request_id, RequestId(REQUEST.into()));
        assert_eq!(command.recorded_at, AT);
    }

    #[test]
    fn a_git_refusal_that_has_another_code_or_place_or_is_retryable() {
        let mut seen = with_reads(looked_up(), SECTIONED);
        seen.admitted = Some(Err(git_fault()));
        let value = assert_failed(
            write_step(&seen),
            "checkout-facts-unavailable",
            "git",
            false,
        );
        assert_eq!(
            value["reason"],
            "git remote get-url upstream exited with code 2"
        );
    }

    #[test]
    fn a_fork_answer_that_tells_the_model_to_run_init_instead_of_naming_the_owners_step() {
        let mut seen = with_reads(looked_up(), SECTIONED);
        seen.admitted = Some(Err(fork_fault()));
        let value = assert_failed(write_step(&seen), "project-id-conflict", "checkout", false);
        let reason = value["reason"].as_str().unwrap();
        for part in [
            "/real/r",
            "https://example.test/mine",
            "/other/r",
            "https://example.test/theirs",
            "The owner gives this checkout its own project by running `baley init --new-id`",
        ] {
            assert!(reason.contains(part), "{part} missing from {reason}");
        }
        assert!(!reason.contains("Run `baley init"), "{reason}");
    }

    #[test]
    fn a_store_failure_at_checkout_admission_or_the_step_that_is_not_failed_or_misses_busy() {
        let busy = LedgerFault::Checkout(EntryError::Admit(checkout::AdmitError::Store(
            StoreError::Busy,
        )));
        let mut seen = with_reads(looked_up(), SECTIONED);
        seen.admitted = Some(Err(busy));
        assert_failed(write_step(&seen), "ledger-busy", "ledger", true);

        let mut seen = admitted(with_reads(looked_up(), SECTIONED));
        seen.stepped = Some(Err(LedgerFault::Store(StoreError::Unavailable(
            "the ledger is fenced".into(),
        ))));
        assert_failed(write_step(&seen), "ledger-unavailable", "ledger", false);
        seen.stepped = Some(Err(LedgerFault::Store(StoreError::Busy)));
        assert_failed(write_step(&seen), "ledger-busy", "ledger", true);
    }

    /// The body of a `request` document for a completed request.
    fn completed_request() -> Value {
        json!({"state": "completed", "scope": [], "digest": "ab".repeat(32),
            "outcome": "done", "answer": {"inline": {"captured": true}}})
    }

    fn with_replay(mut seen: Seen, found: Result<Option<Value>, LedgerFault>) -> Seen {
        seen.replay = Some(found);
        seen
    }

    #[test]
    fn a_write_that_skips_the_replay_lookup_before_the_settings_read() {
        let Next::Do(Step::LookUpRequest { project, key }) = write_step(&known_project()) else {
            panic!("a known project's write looks its request up first");
        };
        assert_eq!(project, ProjectId(ID.into()));
        assert_eq!(
            key,
            baley_store::DocKey(vec![
                baley_store::KeyValue::Text("capture.record".into()),
                baley_store::KeyValue::Text(REQUEST.into()),
            ])
        );
        assert!(matches!(
            write_step(&with_replay(known_project(), Ok(None))),
            Next::Do(Step::ReadSettings { .. })
        ));
    }

    #[test]
    fn a_known_request_that_requests_the_settings_checkout_admission_or_the_step() {
        let seen = with_replay(known_project(), Ok(Some(completed_request())));
        let Next::Ready(prepared) = write_step(&seen) else {
            panic!("a known request is ready with no other step");
        };
        let command = prepared.command.unwrap();
        assert_eq!(command.request_id, RequestId(REQUEST.into()));
        assert_eq!(command.digest, write_request(1).digest);
        assert_eq!(command.policy_version, 0);
    }

    #[test]
    fn a_known_request_turned_into_config_unavailable_by_an_invalid_heads_copy() {
        let seen = with_reads(
            with_replay(known_project(), Ok(Some(completed_request()))),
            "escalate_on_failure = \"yes\"\n",
        );
        assert!(matches!(write_step(&seen), Next::Ready(_)));
    }

    #[test]
    fn a_read_that_requests_the_replay_lookup() {
        let caller = caller("/real/r/p", "/w");
        assert!(matches!(
            next(&read_call(&caller), &known_project()),
            Next::Ready(prepared) if prepared.command.is_none()
        ));
    }

    #[test]
    fn a_stateless_request_document_that_is_not_ledger_unavailable() {
        for body in [
            json!({}),
            json!({"state": "unknown"}),
            json!({"state": "completed"}),
        ] {
            let seen = with_replay(known_project(), Ok(Some(body)));
            assert_failed(write_step(&seen), "ledger-unavailable", "ledger", false);
        }
    }

    #[test]
    fn a_write_check_run_after_the_settings_or_asked_on_a_replay_is_caught() {
        let caller = caller("/real/r/p", "/w");
        let request = write_request(1);
        // The plan only asks for the check, so this one is never run.
        let check: &Check = &|_| panic!("the pure plan ran the check");
        let checked = Call {
            check: Some(check),
            ..write_call(&caller, &request)
        };

        let Next::Do(Step::Check { project }) = next(&checked, &looked_up()) else {
            panic!("a request the ledger lacks is checked before the settings are read");
        };
        assert_eq!(project, ProjectId(ID.into()));

        let refusal =
            crate::envelope::Refusal::new("project-already-started", "a start is approved")
                .slot("submission")
                .value();
        let mut seen = looked_up();
        seen.checked = Some(Err(refusal.clone()));
        seen.relooked = Some(Ok(None));
        match next(&checked, &seen) {
            Next::Checked(answer) => {
                assert_eq!(answer, refusal);
                assert_eq!(answer["status"], "refused");
                assert_eq!(answer["slot"], "submission");
            }
            other => panic!("the check's refusal ends the call, got {other:?}"),
        }

        seen.checked = Some(Ok(()));
        assert!(matches!(
            next(&checked, &seen),
            Next::Do(Step::ReadSettings { .. })
        ));

        let held = with_replay(known_project(), Ok(Some(completed_request())));
        let Next::Ready(prepared) = next(&checked, &held) else {
            panic!("a request the ledger holds is ready without the check");
        };
        assert_eq!(prepared.command.unwrap().policy_version, 0);

        assert!(matches!(
            next(&write_call(&caller, &request), &looked_up()),
            Next::Do(Step::ReadSettings { .. })
        ));
    }

    #[test]
    fn a_retry_recorded_after_its_lookup_and_refused_by_the_check_is_caught() {
        let caller = caller("/real/r/p", "/w");
        let request = write_request(1);
        let check: &Check = &|_| panic!("the pure plan ran the check");
        let checked = Call {
            check: Some(check),
            ..write_call(&caller, &request)
        };
        let refusal =
            crate::envelope::Refusal::new("project-already-started", "a start is approved")
                .slot("submission")
                .value();
        let mut seen = looked_up();
        seen.checked = Some(Err(refusal));

        // The same key as the first lookup, asked before the refusal is given.
        let Next::Do(Step::LookUpRequestAgain { project, key }) = next(&checked, &seen) else {
            panic!("a refusing check is followed by a second lookup");
        };
        assert_eq!(project, ProjectId(ID.into()));
        assert_eq!(
            key,
            baley_store::DocKey(vec![
                baley_store::KeyValue::Text("capture.record".into()),
                baley_store::KeyValue::Text(REQUEST.into()),
            ])
        );

        seen.relooked = Some(Ok(Some(completed_request())));
        let Next::Ready(prepared) = next(&checked, &seen) else {
            panic!("a request recorded since the first lookup is replayed, not refused");
        };
        let command = prepared.command.unwrap();
        assert_eq!(command.request_id, RequestId(REQUEST.into()));
        assert_eq!(command.policy_version, 0);

        // Unable to tell, the call fails retryably rather than refusing.
        seen.relooked = Some(Err(LedgerFault::Store(StoreError::Busy)));
        assert_failed(next(&checked, &seen), "ledger-busy", "ledger", true);
    }

    #[test]
    fn a_prepared_write_without_the_host_policy_or_the_admitted_root_commit_is_caught() {
        use baley_core::policy::{Layer, Rung, Value as Setting};
        let mut seen = admitted(with_reads(looked_up(), "[project]\n"));
        let global = settings::file(
            Path::new("/c/config.toml"),
            b"[roles.planner]\neffort = \"max\"\n".to_vec(),
        );
        seen.reads.as_mut().unwrap().global = Ok(Some(global));
        seen.stepped = Some(Ok(7));
        let Next::Ready(prepared) = write_step(&seen) else {
            panic!("the step's version is observed, so the write is ready");
        };
        assert_eq!(prepared.command.as_ref().unwrap().policy_version, 7);
        let stepped = prepared.stepped.expect("a write that ran its steps");
        assert_eq!(stepped.policy.host, Some(Host::ClaudeCode));
        let effort = &stepped.policy.settings["roles.planner.effort"];
        assert_eq!(effort.value, Some(Setting::Rung(Rung::Max)));
        assert_eq!(effort.source.layer, Layer::Global);
        assert_eq!(stepped.root_commit.as_deref(), Some(ROOT_COMMIT));
        assert_eq!(prepared.project_file, Path::new("/real/r/baley.toml"));
    }

    #[test]
    fn a_route_read_that_admits_the_checkout_or_runs_the_policy_step_is_caught() {
        use baley_core::policy::{Rung, Value as Setting};
        let caller = caller("/real/r/p", "/w");
        let call = Call {
            reads_settings: true,
            ..read_call(&caller)
        };
        let step = |seen: &Seen| match next(&call, seen) {
            Next::Do(step) => step,
            other => panic!("expected a step, got {other:?}"),
        };
        assert_eq!(step(&Seen::default()), Step::Discover("/real/r/p".into()));
        assert_eq!(
            step(&walked()),
            Step::ReadProjectFile("/real/r/baley.toml".into())
        );
        assert_eq!(step(&with_file(walked())), Step::ListProjects);
        let Step::ReadSettings { root, .. } = step(&known_project()) else {
            panic!("a known project's settings are read next");
        };
        assert_eq!(root, Path::new("/real/r"));

        let with_global = |text: &str| {
            let mut seen = with_reads(known_project(), "[project]\n");
            let global = settings::file(Path::new("/c/config.toml"), text.as_bytes().to_vec());
            seen.reads.as_mut().unwrap().global = Ok(Some(global));
            seen
        };
        let Next::Ready(prepared) =
            next(&call, &with_global("[roles.planner]\neffort = \"max\"\n"))
        else {
            panic!("the settings are read, so the read is ready");
        };
        assert_eq!(prepared.command, None);
        assert_eq!(prepared.stepped, None);
        let policy = prepared.policy.expect("the host's policy");
        assert_eq!(policy.host, Some(Host::ClaudeCode));
        assert_eq!(
            policy.settings["roles.planner.effort"].value,
            Some(Setting::Rung(Rung::Max))
        );

        assert_failed(
            next(&call, &with_global("[roles.planner\n")),
            "config-unavailable",
            "settings",
            false,
        );
    }

    #[test]
    fn a_replay_or_a_read_carrying_a_policy_or_a_root_commit_is_caught() {
        let seen = with_replay(known_project(), Ok(Some(completed_request())));
        let Next::Ready(replayed) = write_step(&seen) else {
            panic!("a known request is ready with no other step");
        };
        let caller = caller("/real/r/p", "/w");
        let Next::Ready(read) = next(&read_call(&caller), &known_project()) else {
            panic!("a read is ready at the known-project check");
        };
        for prepared in [replayed, read] {
            assert_eq!(prepared.stepped, None, "{prepared:?}");
            assert_eq!(prepared.project_file, Path::new("/real/r/baley.toml"));
        }
    }

    #[test]
    fn a_project_start_that_records_on_a_refusal_or_hides_its_settings_line_is_caught() {
        use crate::mcp::work_order::tests::{B1, I0, issue_start};
        use crate::mcp::work_order::{judge_start_arguments, settings_lines, start_check};
        use crate::scope::fixtures;

        let start = |brief: &str| json!({"operation": "project-start", "request_id": REQUEST, "brief": brief});
        let judged = judge_start_arguments(&start(B1)).unwrap();
        let caller = caller("/real/r/p", "/w");
        // The plan only asks for the check, so this one is never run.
        let check: &Check = &|_| panic!("the pure plan ran the check");
        let call = Call {
            check: Some(check),
            ..write_call(&caller, &judged.request)
        };

        // (a) A project the ledger does not list.
        let other = known(
            with_file(walked()),
            &["00000000-0000-4000-8000-000000000000"],
        );
        let value = assert_failed(
            next(&call, &other),
            "project-not-in-ledger",
            "project",
            false,
        );
        let reason = value["reason"].as_str().unwrap();
        assert!(reason.contains("baley init"), "{reason}");

        let (_dir, store) = fixtures::store();
        issue_start(&store, &fixtures::project(), I0, fixtures::R1, fixtures::T1);
        let before = fixtures::head(&store);

        // (b) A blank or oversized brief.
        for (brief, code) in [
            (String::new(), "brief-blank"),
            ("x".repeat(65_537), "brief-too-large"),
        ] {
            let refusal = judge_start_arguments(&start(&brief)).unwrap_err();
            assert_eq!(refusal["status"], "refused", "{refusal}");
            assert_eq!(refusal["code"], code, "{refusal}");
            assert_eq!(refusal["slot"], "brief", "{refusal}");
        }

        // (c) A start while I0 is pending ends the call with the check's answer.
        let refusal = start_check(&store, &ProjectId(ID.into())).unwrap_err();
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "start-order-pending", "{refusal}");
        assert_eq!(refusal["details"], json!({"work_order": I0}), "{refusal}");
        assert!(
            refusal["reason"].as_str().unwrap().contains(I0),
            "{refusal}"
        );
        let mut seen = looked_up();
        seen.checked = Some(Err(refusal.clone()));
        seen.relooked = Some(Ok(None));
        match next(&call, &seen) {
            Next::Checked(answer) => assert_eq!(answer, refusal),
            other => panic!("the check's refusal ends the call, got {other:?}"),
        }
        assert_eq!(fixtures::head(&store), before, "a refused start recorded");

        // (d) No global file and a project file of `[project]` only.
        let mut seen = admitted(with_reads(looked_up(), "[project]\n"));
        seen.checked = Some(Ok(()));
        seen.stepped = Some(Ok(3));
        let Next::Ready(prepared) = next(&call, &seen) else {
            panic!("a start with missing settings goes on");
        };
        let lines = settings_lines(&prepared.stepped.expect("the steps ran").policy);
        assert_eq!(lines.len(), 2, "{lines:?}");
        for line in &lines {
            assert!(line.contains("baley config interview"), "{line}");
        }
    }

    #[test]
    fn a_lookup_store_error_that_is_not_busy_or_unavailable() {
        let busy = with_replay(known_project(), Err(LedgerFault::Store(StoreError::Busy)));
        assert_failed(write_step(&busy), "ledger-busy", "ledger", true);
        let down = with_replay(
            known_project(),
            Err(LedgerFault::Store(StoreError::Unavailable("fenced".into()))),
        );
        assert_failed(write_step(&down), "ledger-unavailable", "ledger", false);
    }
}
