//! The SDK conformance plugin `jobsdemo`: background jobs as one small plugin, the same in Rust, TypeScript and C#.
//! A task is created and its processing enqueued in one transaction (a request that asks to fail rolls both back);
//! a daily report is enqueued once per day (a singleton); a heartbeat runs every minute and logs a few lines; a flaky
//! job fails its first attempt and succeeds on the second; a doomed job fails for good; a slow job works in steps of
//! a second and stops when it is aborted (its timeout is 10 s). Its `--describe` must equal `../descriptor.json`, as
//! the TypeScript and C# versions' must (`scripts/sdk-conformance.sh`).

use std::time::Duration;

use celobus_sdk::prelude::*;

pub const MEMBER: Role = Role::new("member")
    .title("Member")
    .default_for(DefaultFor::Member);

#[derive(Entity, Clone, Debug, PartialEq)]
pub struct Task {
    pub id: Id<Task>,
    pub title: String,
    // new, then processed
    pub status: String,
}

#[derive(Message, Clone, Debug)]
pub struct CreateTaskRequest {
    pub title: String,
    // true: the request fails after the enqueue, and the task and its run roll back together
    pub fail: bool,
}

#[derive(Message, Clone, Debug)]
pub struct CreateTaskReply {
    pub task: Id<Task>,
    pub run_id: String,
}

#[derive(Message, Clone, Debug)]
pub struct EnqueueReportRequest {
    // 2026-11-20
    pub day: String,
}

#[derive(Message, Clone, Debug)]
pub struct EnqueueReportReply {
    pub run_id: String,
    // the day's report was enqueued before
    pub existing: bool,
}

#[derive(Message, Clone, Debug)]
pub struct CancelRunRequest {
    pub run_id: String,
}

#[derive(Message, Clone, Debug)]
pub struct CancelRunReply {
    // false: the run had already ended
    pub cancelled: bool,
}

#[derive(Message, Clone, Debug)]
#[message(job)]
pub struct ProcessTask {
    pub task: Id<Task>,
}

#[derive(Message, Clone, Debug, Default)]
#[message(job)]
pub struct DailyReport {
    pub day: String,
}

#[derive(Message, Clone, Debug, Default)]
#[message(job)]
pub struct Heartbeat {}

#[derive(Message, Clone, Debug, Default)]
#[message(job)]
pub struct Flaky {}

#[derive(Message, Clone, Debug, Default)]
#[message(job)]
pub struct Doomed {}

#[derive(Message, Clone, Debug, Default)]
#[message(job)]
pub struct Slow {
    // 0: 30, past the timeout
    pub seconds: i32,
}

/// Creates a task and enqueues its processing in one transaction: both exist only if it commits.
pub async fn create_task(ctx: Ctx, req: CreateTaskRequest) -> Result<CreateTaskReply> {
    ctx.transaction(async |tx| {
        let task = tx
            .db::<Task>()
            .create(Task {
                id: Id::none(),
                title: req.title.clone(),
                status: "new".into(),
            })
            .await?;
        let run = tx.jobs().enqueue(ProcessTask { task: task.id }).await?;
        if req.fail {
            return Err(Error::invalid(
                "asked to fail: the task and its run are rolled back",
            ));
        }
        Ok(CreateTaskReply {
            task: task.id,
            run_id: run.run_id,
        })
    })
    .await
}

/// Enqueues the day's report once: a second request for the same day answers the first run.
pub async fn enqueue_report(ctx: Ctx, req: EnqueueReportRequest) -> Result<EnqueueReportReply> {
    let run = ctx
        .jobs()
        .enqueue(DailyReport {
            day: req.day.clone(),
        })
        .singleton(format!("report:{}", req.day))
        .await?;
    Ok(EnqueueReportReply {
        run_id: run.run_id,
        existing: run.existing,
    })
}

pub async fn cancel_run(ctx: Ctx, req: CancelRunRequest) -> Result<CancelRunReply> {
    Ok(CancelRunReply {
        cancelled: ctx.jobs().cancel(&req.run_id).await?,
    })
}

async fn process_task(ctx: Ctx, p: ProcessTask) -> Result<()> {
    let task = ctx.db::<Task>().get(&p.task).await?;
    tracing::info!("processing task {}", task.title);
    ctx.db::<Task>()
        .update(
            &p.task,
            Patch::new().set(Task::status, "processed".to_string()),
        )
        .await?;
    Ok(())
}

async fn daily_report(ctx: Ctx, p: DailyReport) -> Result<()> {
    tracing::info!("report for {} (run {})", p.day, ctx.job().run_id());
    Ok(())
}

async fn heartbeat(ctx: Ctx, _p: Heartbeat) -> Result<()> {
    let at = ctx.job().scheduled_at();
    tracing::info!("heartbeat at {}", at.to_rfc3339());
    tracing::info!(
        "attempt {} of {}",
        ctx.job().attempt(),
        ctx.job().max_attempts()
    );
    tracing::warn!("nothing to do");
    Ok(())
}

async fn flaky(ctx: Ctx, _p: Flaky) -> Result<()> {
    if ctx.job().attempt() == 1 {
        tracing::warn!("first attempt: failing");
        return Err(Error::unavailable("the first attempt always fails"));
    }
    tracing::info!("attempt {}: done", ctx.job().attempt());
    Ok(())
}

async fn doomed(_ctx: Ctx, _p: Doomed) -> Result<()> {
    tracing::error!("giving up");
    Err(Error::permanent("this job always fails for good"))
}

async fn slow(_ctx: Ctx, p: Slow) -> Result<()> {
    let seconds = if p.seconds > 0 { p.seconds } else { 30 };
    for step in 1..=seconds {
        // an abort stops the handler at this await
        tokio::time::sleep(Duration::from_secs(1)).await;
        tracing::info!("step {step} of {seconds}");
    }
    Ok(())
}

pub fn plugin() -> Plugin {
    Plugin::new("jobsdemo", env!("CARGO_PKG_VERSION"))
        .title("Jobs demo")
        .describe("The SDK conformance plugin of background jobs.")
        .role(MEMBER)
        .entity::<Task>()
        .access::<Task>(Access::build(|a| {
            a.read(MEMBER);
            a.create(MEMBER);
            a.update(MEMBER);
        }))
        .endpoint(
            Endpoint::new(create_task)
                .describe("Creates a task and enqueues its processing.")
                .roles([MEMBER]),
        )
        .endpoint(
            Endpoint::new(enqueue_report)
                .describe("Enqueues the day's report once.")
                .roles([MEMBER]),
        )
        .endpoint(
            Endpoint::new(cancel_run)
                .describe("Cancels a run.")
                .roles([MEMBER]),
        )
        .job(Job::new(process_task).describe("Marks a task processed"))
        .job(Job::new(daily_report).describe("Writes the day's report"))
        .job(
            Job::new(heartbeat)
                .describe("Logs a heartbeat every minute")
                .cron("* * * * *")
                .as_system()
                .retry(Retry::none()),
        )
        .job(
            Job::new(flaky)
                .describe("Fails its first attempt")
                .retry(Retry::fixed(2, Duration::from_secs(5))),
        )
        .job(
            Job::new(doomed)
                .describe("Fails for good")
                .retry(Retry::exponential(3, Duration::from_secs(10))),
        )
        .job(
            Job::new(slow)
                .describe("Works in steps until it is stopped")
                .timeout(Duration::from_secs(10))
                .concurrency(2),
        )
}

fn main() {
    plugin().serve()
}
