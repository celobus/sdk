//! Background jobs: a payload message and a handler, declared with [`Job`] and `Plugin::job`; started with
//! `ctx.jobs().enqueue(payload)` (in a declared transaction `tx.jobs()`, so the run exists only if it commits) or by
//! a schedule; cancelled with `ctx.jobs().cancel(..)`. In the handler, `ctx.job()` is the run.
//!
//! ```no_run
//! use std::time::Duration;
//! use celobus_sdk::prelude::*;
//!
//! /// Remind the clerks of orders still unpaid.
//! #[derive(Message, Clone, Debug, Default)]
//! #[message(job)]
//! pub struct RemindUnpaid { pub older_than_days: i32 }
//!
//! pub fn job() -> Job<RemindUnpaid> {
//!     Job::new(run)
//!         .describe("Remind unpaid orders")
//!         .cron("0 9 * * 1-5")
//!         .as_system()
//!         .retry(Retry::exponential(3, Duration::from_secs(60)))
//!         .timeout(Duration::from_secs(120))
//! }
//!
//! async fn run(ctx: Ctx, p: RemindUnpaid) -> Result<()> {
//!     tracing::info!(attempt = ctx.job().attempt(), days = p.older_than_days, "reminding");
//!     Ok(())
//! }
//!
//! fn main() {
//!     Plugin::new("orders", env!("CARGO_PKG_VERSION")).job(job()).serve()
//! }
//! ```

use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use celobus_proto as pb;
use futures::future::BoxFuture;
use pb::{syscall, syscall_result};
use serde_json::Value;

use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::message::{Message, Registry};
use crate::types::Timestamp;

/// The most a payload may weigh, as JSON.
pub const MAX_PAYLOAD: usize = 64 << 10;
/// The runs a process starts at once by default, and the most it may announce.
pub const DEFAULT_JOB_SLOTS: u32 = 16;
pub const MAX_JOB_SLOTS: u32 = 64;

pub use celobus_descriptor::Backoff;

/// How a job's failed attempts are retried: `Retry::exponential(3, 60 s)` waits 60 s, 120 s, 240 s (each ±20 %,
/// at most an hour); `Retry::fixed(5, 30 s)` waits 30 s each time; `Retry::none()` never retries. Unset: 3 retries,
/// 30 s apart, exponential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retry {
    retries: u32,
    delay: Duration,
    backoff: Backoff,
}

impl Retry {
    /// `retries` attempts after the first (at most 20), the delay doubling from `delay` (at most an hour).
    pub fn exponential(retries: u32, delay: Duration) -> Self {
        Self {
            retries,
            delay,
            backoff: Backoff::Exponential,
        }
    }

    /// `retries` attempts after the first (at most 20), `delay` apart (at most an hour).
    pub fn fixed(retries: u32, delay: Duration) -> Self {
        Self {
            retries,
            delay,
            backoff: Backoff::Fixed,
        }
    }

    /// One attempt only.
    pub fn none() -> Self {
        Self::fixed(0, Duration::ZERO)
    }

    fn decl(&self) -> celobus_descriptor::JobRetry {
        celobus_descriptor::JobRetry {
            retries: self.retries,
            delay_ms: self.delay.as_millis() as u64,
            backoff: self.backoff,
        }
    }
}

pub(crate) type JobHandler =
    Arc<dyn Fn(Ctx, Value) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// A background job: `Job::new(handler)`, where the handler is `async fn(Ctx, P) -> Result<()>` and `P` the
/// payload message (`#[derive(Message)] #[message(job)]`). Its key is the payload's name in snake_case
/// (`RemindUnpaid` → `remind_unpaid`; `.name(..)` to choose another); its full name in logs and on screens is
/// `<plugin>.<key>`. Enqueueing picks the job by the payload's type, so no two jobs share a payload message.
///
/// Defaults: started only when enqueued (`.cron(..)` adds a schedule), as the caller who enqueued it
/// (`.as_system()`: as the plugin's system principal), 3 retries 30 s apart backing off exponentially (`.retry(..)`),
/// a timeout of 5 minutes (1 s to 1 h, `.timeout(..)`), one run at a time (1 to 64, `.concurrency(..)`).
pub struct Job<P> {
    pub(crate) decl: JobDecl,
    _p: PhantomData<fn() -> P>,
}

/// A job, its payload type erased.
#[derive(Clone)]
pub(crate) struct JobDecl {
    pub(crate) name: String,
    pub(crate) payload: &'static str,
    pub(crate) register: fn(&mut Registry),
    pub(crate) handler: JobHandler,
    pub(crate) description: String,
    pub(crate) cron: String,
    pub(crate) as_system: bool,
    pub(crate) retry: Option<Retry>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) concurrency: Option<u32>,
}

impl JobDecl {
    pub(crate) fn descriptor(&self) -> celobus_descriptor::Job {
        celobus_descriptor::Job {
            name: self.name.clone(),
            payload: self.payload.to_string(),
            description: self.description.clone(),
            cron: self.cron.clone(),
            as_system: self.as_system,
            retry: self.retry.map(|r| r.decl()),
            timeout_ms: self.timeout.map(|t| t.as_millis() as u64),
            concurrency: self.concurrency,
        }
    }
}

impl<P: Message> Job<P> {
    pub fn new<F, Fut>(handler: F) -> Self
    where
        F: Fn(Ctx, P) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let handler: JobHandler = Arc::new(move |ctx: Ctx, payload: Value| {
            let handler = handler.clone();
            Box::pin(async move {
                let payload = P::from_json(&payload, "").map_err(|e| {
                    Error::new(
                        400,
                        "payload_invalid",
                        format!("the payload is not a {}: {}", P::NAME, e.message()),
                    )
                })?;
                handler(ctx, payload).await
            })
        });
        Self {
            decl: JobDecl {
                name: job_key(P::NAME),
                payload: P::NAME,
                register: P::register,
                handler,
                description: String::new(),
                cron: String::new(),
                as_system: false,
                retry: None,
                timeout: None,
                concurrency: None,
            },
            _p: PhantomData,
        }
    }

    /// The job's key, instead of the payload's name in snake_case: snake_case, at most 32 characters.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.decl.name = name.into();
        self
    }

    /// What the job does, for the workspace's list of jobs: plain text or a key of the UI's text catalog.
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.decl.description = description.into();
        self
    }

    /// A schedule: five fields in UTC, `minute hour day-of-month month day-of-week` (`0 9 * * 1-5`: weekdays at
    /// 09:00 UTC). Each occurrence starts a run with the payload's defaults; an occurrence that comes due while the
    /// previous scheduled run is still pending or running is not started.
    pub fn cron(mut self, cron: impl Into<String>) -> Self {
        self.decl.cron = cron.into();
        self
    }

    /// Its runs run as the plugin's system principal, whoever enqueued them.
    pub fn as_system(mut self) -> Self {
        self.decl.as_system = true;
        self
    }

    pub fn retry(mut self, retry: Retry) -> Self {
        self.decl.retry = Some(retry);
        self
    }

    /// How long an attempt may run (1 s to 1 h); past it the handler is stopped and the attempt fails as `timeout`.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.decl.timeout = Some(timeout);
        self
    }

    /// How many runs of the job run at once in the workspace (1 to 64).
    pub fn concurrency(mut self, n: u32) -> Self {
        self.decl.concurrency = Some(n);
        self
    }
}

/// `RemindUnpaid` → `remind_unpaid`.
pub(crate) fn job_key(message: &str) -> String {
    let mut out = String::with_capacity(message.len() + 4);
    for (i, c) in message.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The run a job's handler is in: `ctx.job()`.
#[derive(Debug, Clone, Default)]
pub struct JobInfo {
    pub(crate) job: String,
    pub(crate) run_id: String,
    pub(crate) attempt: u32,
    pub(crate) max_attempts: u32,
    pub(crate) scheduled_at: Timestamp,
}

static NO_RUN: LazyLock<JobInfo> = LazyLock::new(JobInfo::default);

impl JobInfo {
    pub(crate) fn none() -> &'static JobInfo {
        &NO_RUN
    }

    /// Whether this is a job's run (outside one: an empty run id and attempt 0).
    pub fn is_run(&self) -> bool {
        !self.run_id.is_empty()
    }

    /// The job's key (`remind_unpaid`).
    pub fn job(&self) -> &str {
        &self.job
    }

    /// The run's id, the same in every attempt: the idempotency key of the handler's outside effects.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// 1 on the first attempt.
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// The attempts the run may take; `attempt() == max_attempts()` on the last one.
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Whether a failure of this attempt ends the run as failed.
    pub fn is_last_attempt(&self) -> bool {
        self.attempt >= self.max_attempts
    }

    /// The occurrence (or due time) the run stands for, the same in every attempt.
    pub fn scheduled_at(&self) -> Timestamp {
        self.scheduled_at
    }
}

/// The state of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Pending,
    Running,
    Done,
    /// Failed for good: its retries used up, or it failed as permanent.
    Failed,
    Cancelled,
    /// A state this SDK does not know.
    Unknown,
}

impl RunState {
    fn from_wire(v: i32) -> Self {
        match pb::RunState::try_from(v) {
            Ok(pb::RunState::Pending) => Self::Pending,
            Ok(pb::RunState::Running) => Self::Running,
            Ok(pb::RunState::Done) => Self::Done,
            Ok(pb::RunState::Failed) => Self::Failed,
            Ok(pb::RunState::Cancelled) => Self::Cancelled,
            _ => Self::Unknown,
        }
    }
}

/// A run started by an enqueue: `run_id`, and `existing` when its singleton key was taken by an earlier run (whose
/// id and state these are; its payload is kept).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enqueued {
    pub run_id: String,
    pub existing: bool,
    pub state: RunState,
}

impl AsRef<str> for Enqueued {
    fn as_ref(&self) -> &str {
        &self.run_id
    }
}

/// The plugin's background jobs: `ctx.jobs()`, `tx.jobs()` in a declared transaction.
pub struct Jobs {
    ctx: Ctx,
}

impl Jobs {
    pub(crate) fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Starts a run of the job whose payload this is, as the caller of the request or run in progress (a job
    /// declared `.as_system()` runs as the system); `.delay(..)`, `.at(..)` and `.singleton(..)` before `.await`.
    /// In a declared transaction (`tx.jobs()`) the run is written in it and exists only if it commits; a singleton
    /// enqueue there may wait for a concurrent transaction enqueuing the same key. Errors: `invalid` (no job takes
    /// this payload, `at` more than a year ahead), `payload_too_large` (over 64 KiB: put larger input in an entity
    /// and pass its id), `queue_full` (429: 10,000 pending runs of the plugin).
    pub fn enqueue<P: Message>(&self, payload: P) -> Enqueue {
        Enqueue {
            ctx: self.ctx.clone(),
            job: self.ctx.plugin().job_for(P::NAME),
            payload: payload.to_json(),
            when: None,
            singleton: String::new(),
        }
    }

    /// Cancels one of the plugin's runs: a pending run never starts, a running one is stopped. `false` when it had
    /// already ended; `not_found` when the plugin has no such run.
    pub async fn cancel(&self, run: &(impl AsRef<str> + ?Sized)) -> Result<bool> {
        let r = self
            .ctx
            .syscall(syscall::Call::JobCancel(pb::JobCancel {
                run_id: run.as_ref().to_string(),
                request_id: self.ctx.request().id(),
            }))
            .await?;
        match r {
            Some(syscall_result::Result::JobCancelled(c)) => Ok(c.cancelled),
            _ => Err(Error::internal(
                "the platform answered a cancel with something else",
            )),
        }
    }
}

/// An enqueue in the making; `.await` it.
#[must_use = "an enqueue does nothing until it is awaited"]
pub struct Enqueue {
    ctx: Ctx,
    job: Result<String>,
    payload: Value,
    when: Option<pb::job_enqueue::When>,
    singleton: String,
}

impl Enqueue {
    /// Due after `delay` (default: now).
    pub fn delay(mut self, delay: Duration) -> Self {
        self.when = Some(pb::job_enqueue::When::DelayMs(delay.as_millis() as u64));
        self
    }

    /// Due at `at` (at most a year ahead; a time past is now).
    pub fn at(mut self, at: Timestamp) -> Self {
        self.when = Some(pb::job_enqueue::When::AtUnixMs(at.timestamp_millis()));
        self
    }

    /// At most one run per key and job while that run is kept (7 days after it ends, 30 if it failed): an enqueue
    /// with a key taken answers the earlier run (`existing`) and keeps its payload.
    pub fn singleton(mut self, key: impl Into<String>) -> Self {
        self.singleton = key.into();
        self
    }

    pub(crate) fn message(&self) -> Result<pb::JobEnqueue> {
        let job = self.job.clone()?;
        let payload = serde_json::to_string(&self.payload)?;
        if payload.len() > MAX_PAYLOAD {
            return Err(Error::new(
                413,
                "payload_too_large",
                format!(
                    "the payload of {job} is {} bytes, over 64 KiB: put larger input in an entity and pass its id",
                    payload.len()
                ),
            ));
        }
        Ok(pb::JobEnqueue {
            job,
            payload,
            when: self.when,
            singleton: self.singleton.clone(),
            request_id: self.ctx.request().id(),
            tx: self.ctx.tx(),
        })
    }
}

impl IntoFuture for Enqueue {
    type Output = Result<Enqueued>;
    type IntoFuture = BoxFuture<'static, Result<Enqueued>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let msg = self.message()?;
            match self.ctx.syscall(syscall::Call::JobEnqueue(msg)).await? {
                Some(syscall_result::Result::JobEnqueued(e)) => Ok(Enqueued {
                    run_id: e.run_id,
                    existing: e.existing,
                    state: RunState::from_wire(e.state),
                }),
                _ => Err(Error::internal(
                    "the platform answered an enqueue with something else",
                )),
            }
        })
    }
}

/// The error code of an abort's reason (an unknown reason is a cancel).
pub(crate) fn abort_code(reason: i32) -> &'static str {
    use pb::job_abort::Reason;
    match Reason::try_from(reason) {
        Ok(Reason::Timeout) => "timeout",
        Ok(Reason::Shutdown) => "shutdown",
        Ok(Reason::LeaseLost) => "lease_lost",
        Ok(Reason::AuthChanged) => "auth_changed",
        Ok(Reason::Removed) => "job_removed",
        _ => "cancelled",
    }
}

/// At most 2 KiB of `s`, cut at a character boundary.
pub(crate) fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// The outcome of an attempt as its `JobDone`.
pub(crate) fn job_done(run_id: String, attempt: u32, result: Result<()>) -> pb::JobDone {
    use pb::job_done::Outcome;
    let outcome = match result {
        Ok(()) => Outcome::Ok(pb::JobOk {}),
        Err(e) => Outcome::Error(pb::JobError {
            code: e.code().to_string(),
            message: cut(e.message(), 2048),
            permanent: e.is_permanent(),
        }),
    };
    pb::JobDone {
        run_id,
        attempt,
        outcome: Some(outcome),
    }
}
