//! jobsdemo's part of the Data Lake, the same in the three SDKs: two layers, a table of tasks as an outside tracker
//! sends them (partitioned by the day of the load; its message is named apart from the table), a table of the tasks
//! of a day by stage (partitioned and keyed, read by members and by the plugin's system context, exported), and
//! another plugin's exported table it reads. `LoadTasks` writes them in each mode, `TaskDays` reads one back grouped,
//! and the job `task_batch` appends with the run's default keys. It reads the history of the tracker's issues too. Five tables are computed by models, one of each kind
//! of source: the plugin's own table (a text parsed), another plugin's table (grouped by week), an entity (a masked
//! column copied as it is), an analytic view, and the history of tasks per day; `RefreshTables` refreshes them, whole
//! or by partition, and `TaskTrail` reads the history of tasks. What each SDK sends for them is `../../lake.json`
//! (`scripts/sdk-conformance.sh`). An entity stored in OLAP, the tasks of a day by stage, is loaded from `task_day`;
//! `LoadStageDays` asks for its snapshot and follows the ticket, `StageDays` reads it from the analytic copy.

use celobus_sdk::lake::history::{self, HistoryRow};
use celobus_sdk::prelude::*;
use celobus_sdk::query::{count, sum};
use celobus_sdk::{Ticket, TicketState, TicketStatus};

use crate::{Budget, LongEstimate, MEMBER, Task};

pub const RAW: Layer = Layer::new("raw").title("As it arrived");
pub const MART: Layer = Layer::new("mart").title("For reports");
pub const AUDIT: Layer = Layer::new("audit").title("History of records");

/// Tasks as an outside tracker sends them, a load a day.
#[derive(LakeTable, Clone, Debug)]
#[lake(layer = RAW, name = "task_load", partition(loaded_on))]
pub struct TaskLoadRow {
    pub loaded_on: Date,
    pub title: String,
    // as the tracker wrote it
    pub hours: String,
}

/// The tasks of a day by stage.
#[derive(LakeTable, Clone, Debug)]
#[lake(layer = MART, partition(day), key(day, stage))]
pub struct TaskDay {
    pub day: Date,
    pub stage: String,
    pub tasks: i64,
    pub hours: Decimal,
    pub note: Option<String>,
}

/// The tracker's issues by day, a table the plugin `tracker` exports.
#[derive(LakeTable, Clone, Debug)]
#[lake(plugin = "tracker", name = "issue_day")]
pub struct TrackerIssueDay {
    pub day: Date,
    pub issues: i64,
}

/// The tracker's issues, whose history the plugin `tracker` exports.
#[derive(Entity, Clone, Debug)]
#[entity(plugin = "tracker")]
pub struct Issue {
    pub id: Id<Issue>,
    pub title: String,
}

/// The tasks of a day by stage with tasks, kept in the analytic copy only: loaded from `task_day`.
#[derive(Entity, Clone, Debug)]
#[entity(storage = olap, key(day, stage))]
pub struct StageDay {
    pub id: Id<StageDay>,
    pub day: Date,
    pub stage: String,
    pub tasks: i64,
    pub hours: Option<Decimal>,
}

/// The days of `task_day` with tasks.
fn stage_day_load() -> Load<StageDay> {
    Load::<StageDay>::from::<TaskDay>()
        .filter(TaskDay::tasks.gt(0))
        .col(StageDay::day, TaskDay::day)
        .col(StageDay::stage, TaskDay::stage)
        .col(StageDay::tasks, TaskDay::tasks)
        .col(StageDay::hours, TaskDay::hours)
}

/// The hours of the loaded tasks, as numbers: computed from `task_load`.
#[derive(LakeTable, Clone, Debug)]
#[lake(layer = MART, partition(day))]
pub struct TaskHours {
    pub day: Date,
    pub title: String,
    pub hours: Decimal,
}

/// The tracker's issues by week: computed from its exported `issue_day`.
#[derive(LakeTable, Clone, Debug)]
#[lake(layer = MART, partition(week))]
pub struct IssueWeek {
    pub week: Date,
    pub issues: i64,
}

/// The budgets with their notes: computed from the entity, its masked column as it is.
#[derive(LakeTable, Clone, Debug)]
#[lake(layer = MART)]
pub struct BudgetCopy {
    pub id: Id<Budget>,
    pub title: String,
    pub notes: Option<String>,
}

/// The long estimates: computed from the analytic view.
#[derive(LakeTable, Clone, Debug)]
#[lake(layer = MART)]
pub struct LongEstimateCopy {
    pub title: String,
    pub hours: Decimal,
}

/// The tasks not processed at the end of each day, by status: computed from the history of tasks.
#[derive(LakeTable, Clone, Debug)]
#[lake(layer = MART, partition(day))]
pub struct OpenByDay {
    pub day: Date,
    pub status: Option<String>,
    pub tasks: i64,
}

/// The loads' tasks with a title, their hours parsed.
fn task_hours() -> Model<TaskHours> {
    Model::<TaskHours>::from::<TaskLoadRow>()
        .filter(TaskLoadRow::title.ne(String::new()))
        .col(TaskHours::day, TaskLoadRow::loaded_on)
        .col(TaskHours::title, TaskLoadRow::title)
        .col(TaskHours::hours, TaskLoadRow::hours.parse::<Decimal>())
}

/// The tracker's issues summed by week.
fn issue_week() -> Model<IssueWeek> {
    Model::<IssueWeek>::from::<TrackerIssueDay>()
        .group_by(TrackerIssueDay::day.week())
        .col(IssueWeek::week, TrackerIssueDay::day.week())
        .col(IssueWeek::issues, sum(TrackerIssueDay::issues))
}

/// The budgets, their masked notes as they are.
fn budget_copy() -> Model<BudgetCopy> {
    Model::<BudgetCopy>::from::<Budget>()
        .col(BudgetCopy::id, Budget::id)
        .col(BudgetCopy::title, Budget::title)
        .col(BudgetCopy::notes, Budget::notes)
        .unmasked()
}

/// The rows of the view `long_estimate`.
fn long_estimate_copy() -> Model<LongEstimateCopy> {
    Model::<LongEstimateCopy>::from::<LongEstimate>()
        .col(LongEstimateCopy::title, LongEstimate::title)
        .col(LongEstimateCopy::hours, LongEstimate::hours)
}

/// The tasks not processed at the end of each day, counted by status.
fn open_by_day() -> Model<OpenByDay> {
    Model::<OpenByDay>::from_history(history::history::<Task>().each_day())
        .filter(Task::status.ne("processed".to_string()))
        .group_by((history::day::<Task>(), Task::status))
        .col(OpenByDay::day, history::day::<Task>())
        .col(OpenByDay::status, Task::status)
        .col(OpenByDay::tasks, count())
}

/// The day every conformance row is of.
fn day() -> Date {
    "2026-11-20".parse().unwrap()
}

/// `n` rows of `task_load`: `t<i>` followed by `width` x's, 1.5 hours.
fn loads(n: i32, width: i32) -> Vec<TaskLoadRow> {
    let pad = "x".repeat(width.max(0) as usize);
    (0..n)
        .map(|i| TaskLoadRow {
            loaded_on: day(),
            title: format!("t{i}{pad}"),
            hours: "1.5".into(),
        })
        .collect()
}

/// `n` rows of `task_day`: stage `s<i>`, `i` tasks, 2.25 hours, no note.
fn days(n: i32) -> Vec<TaskDay> {
    (0..n)
        .map(|i| TaskDay {
            day: day(),
            stage: format!("s{i}"),
            tasks: i64::from(i),
            hours: "2.25".parse().unwrap(),
            note: None,
        })
        .collect()
}

#[derive(Message, Clone, Debug)]
pub struct LoadTasksRequest {
    // replace, append or merge
    pub mode: String,
    pub rows: i32,
    // the x's after each title
    pub width: i32,
    // append: the write key (none: refused outside a job's run)
    pub key: String,
}

#[derive(Message, Clone, Debug)]
pub struct LoadTasksReply {
    pub rows: i64,
    pub skipped: bool,
}

/// Writes `rows` generated rows in one write: `replace` and `append` into `task_load`, `merge` into `task_day`.
pub async fn load_tasks(ctx: Ctx, req: LoadTasksRequest) -> Result<LoadTasksReply> {
    let written = match req.mode.as_str() {
        "replace" => {
            ctx.lake()
                .table::<TaskLoadRow>()
                .replace(TaskLoadRow::loaded_on.eq(day()), loads(req.rows, req.width))
                .await?
        }
        "append" => {
            let append = ctx
                .lake()
                .table::<TaskLoadRow>()
                .append(loads(req.rows, req.width));
            if req.key.is_empty() {
                append.await?
            } else {
                append.key(req.key.clone()).await?
            }
        }
        "merge" => ctx.lake().table::<TaskDay>().merge(days(req.rows)).await?,
        other => {
            return Err(Error::invalid(format!(
                "{other} is not replace, append or merge"
            )));
        }
    };
    Ok(LoadTasksReply {
        rows: written.rows as i64,
        skipped: written.skipped,
    })
}

#[derive(Message, Clone, Debug)]
pub struct TaskDaysRequest {}

#[derive(Message, Clone, Debug)]
pub struct TaskDaysReply {
    // the groups the lake answered
    pub groups: i32,
}

/// The conformance lake query: the tasks and hours of November 2026 with tasks, by month and stage.
pub fn task_days(ctx: &Ctx) -> celobus_sdk::query::Selected<(Date, String, i64, Decimal)> {
    ctx.lake()
        .query::<TaskDay>()
        .filter(
            TaskDay::day.between("2026-11-01".parse().unwrap(), "2026-11-30".parse().unwrap())
                & TaskDay::tasks.gt(0),
        )
        .group_by((TaskDay::day.month(), TaskDay::stage))
        .select((
            TaskDay::day.month(),
            TaskDay::stage,
            sum(TaskDay::tasks),
            sum(TaskDay::hours),
        ))
        .order_by(TaskDay::stage.asc())
        .limit(50)
}

pub async fn read_task_days(ctx: Ctx, _req: TaskDaysRequest) -> Result<TaskDaysReply> {
    Ok(TaskDaysReply {
        groups: task_days(&ctx).all().await?.len() as i32,
    })
}

#[derive(Message, Clone, Debug)]
pub struct RefreshTablesRequest {
    // task_hours, issue_week, budget_copy, long_estimate_copy or open_by_day
    pub table: String,
    // only the conformance day's partition
    pub day: bool,
    // task_hours: a refresh of partitions naming none (refused)
    pub every: bool,
}

#[derive(Message, Clone, Debug)]
pub struct RefreshTablesReply {
    pub rows: i64,
    // when the data read is from, unix milliseconds (0: a model over lake tables)
    pub as_of: i64,
    // the partitions still holding an earlier declaration's rows
    pub stale_partitions: i64,
}

/// Refreshes one of the computed tables, whole or the conformance day's partition.
pub async fn refresh_tables(ctx: Ctx, req: RefreshTablesRequest) -> Result<RefreshTablesReply> {
    let lake = ctx.lake();
    let d = day();
    let done = match (req.table.as_str(), req.day) {
        ("task_hours", _) if req.every => {
            lake.refresh::<TaskHours>().partition(Filter::all()).await?
        }
        ("task_hours", false) => lake.refresh::<TaskHours>().await?,
        ("task_hours", true) => {
            lake.refresh::<TaskHours>()
                .partition(TaskHours::day.eq(d))
                .await?
        }
        ("issue_week", false) => lake.refresh::<IssueWeek>().await?,
        ("issue_week", true) => {
            lake.refresh::<IssueWeek>()
                .partition(IssueWeek::week.eq(d))
                .await?
        }
        ("budget_copy", _) => lake.refresh::<BudgetCopy>().await?,
        ("long_estimate_copy", _) => lake.refresh::<LongEstimateCopy>().await?,
        ("open_by_day", false) => lake.refresh::<OpenByDay>().await?,
        ("open_by_day", true) => {
            lake.refresh::<OpenByDay>()
                .partition(OpenByDay::day.eq(d))
                .await?
        }
        (other, _) => return Err(Error::invalid(format!("{other} has no model"))),
    };
    Ok(RefreshTablesReply {
        rows: done.rows as i64,
        as_of: done.as_of.map(|t| t.timestamp_millis()).unwrap_or(0),
        stale_partitions: done.stale_partitions as i64,
    })
}

#[derive(Message, Clone, Debug)]
pub struct TaskTrailRequest {
    // one row per day instead of one per change
    pub each_day: bool,
}

#[derive(Message, Clone, Debug)]
pub struct TaskTrailReply {
    pub rows: i32,
    // the first row's change, its author's kind and plugin, its day ("" for none) and status ("" for none)
    pub change: String,
    pub changed_by_kind: String,
    pub changed_by_plugin: String,
    pub day: String,
    pub status: String,
    // when the first row's version was made and ended, unix milliseconds (0: none)
    pub valid_from: i64,
    pub valid_to: i64,
}

/// The conformance history read: the changes of tasks whose status is not empty, by when they were made (or, per
/// day, by day), at most 20.
pub async fn task_trail(ctx: Ctx, req: TaskTrailRequest) -> Result<TaskTrailReply> {
    let q = ctx.lake().history::<Task>();
    let q = if req.each_day { q.each_day() } else { q };
    let rows: Vec<HistoryRow<Task>> = q
        .filter(Task::status.ne(String::new()))
        .order_by(if req.each_day {
            history::day::<Task>().asc()
        } else {
            history::valid_from::<Task>().asc()
        })
        .limit(20)
        .all()
        .await?;
    let Some(first) = rows.first() else {
        return Ok(TaskTrailReply {
            rows: 0,
            change: String::new(),
            changed_by_kind: String::new(),
            changed_by_plugin: String::new(),
            day: String::new(),
            status: String::new(),
            valid_from: 0,
            valid_to: 0,
        });
    };
    Ok(TaskTrailReply {
        rows: rows.len() as i32,
        change: first.change.clone(),
        changed_by_kind: first.changed_by_kind.clone(),
        changed_by_plugin: first.changed_by_plugin.clone().unwrap_or_default(),
        day: first.day.map(|d| d.to_string()).unwrap_or_default(),
        status: first.get(Task::status)?.unwrap_or_default(),
        valid_from: first.valid_from.timestamp_millis(),
        valid_to: first.valid_to.map(|t| t.timestamp_millis()).unwrap_or(0),
    })
}

#[derive(Message, Clone, Debug)]
pub struct LoadStageDaysRequest {
    // request, status or wait
    pub step: String,
    // status and wait: the ticket asked about
    pub ticket: String,
}

#[derive(Message, Clone, Debug)]
pub struct LoadStageDaysReply {
    pub ticket: String,
    // pending, building, done or failed
    pub state: String,
    pub generation: i64,
    // the entities it loads, and the rows of its first load (0 before)
    pub entities: Vec<String>,
    pub rows: i64,
    // a failed ticket's first failure
    pub code: String,
}

fn ticket_reply(t: &TicketStatus) -> LoadStageDaysReply {
    LoadStageDaysReply {
        ticket: t.id.clone(),
        state: match t.state {
            TicketState::Pending => "pending",
            TicketState::Building => "building",
            TicketState::Done => "done",
            TicketState::Failed => "failed",
        }
        .into(),
        generation: t.generation as i64,
        entities: t.entities.clone(),
        rows: t.loads.first().map(|l| l.rows as i64).unwrap_or(0),
        code: t.code.clone(),
    }
}

/// Asks for a snapshot that loads `stage_day`, asks how its ticket stands, or waits for its end.
pub async fn load_stage_days(ctx: Ctx, req: LoadStageDaysRequest) -> Result<LoadStageDaysReply> {
    match req.step.as_str() {
        "request" => {
            let t = ctx
                .analytics()
                .snapshot()
                .load::<StageDay>()
                .request()
                .await?;
            Ok(ticket_reply(t.last()))
        }
        "status" => {
            let mut t = Ticket::of(&ctx, req.ticket.clone());
            Ok(ticket_reply(t.status().await?))
        }
        "wait" => Ok(ticket_reply(
            &Ticket::of(&ctx, req.ticket.clone()).wait().await?,
        )),
        other => Err(Error::invalid(format!(
            "{other} is not request, status or wait"
        ))),
    }
}

#[derive(Message, Clone, Debug)]
pub struct StageDaysRequest {}

#[derive(Message, Clone, Debug)]
pub struct StageDaysReply {
    // the rows the analytic copy answered
    pub rows: i32,
}

/// Reads the loaded days of `stage_day` from the analytic copy.
pub async fn read_stage_days(ctx: Ctx, _req: StageDaysRequest) -> Result<StageDaysReply> {
    let rows = ctx
        .analytics()
        .query::<StageDay>()
        .filter(StageDay::tasks.gt(0))
        .select((
            StageDay::day,
            StageDay::stage,
            StageDay::tasks,
            StageDay::hours,
        ))
        .order_by(StageDay::day.asc())
        .all()
        .await?;
    Ok(StageDaysReply {
        rows: rows.len() as i32,
    })
}

#[derive(Message, Clone, Debug, Default)]
#[message(job)]
pub struct TaskBatch {
    pub rows: i32,
}

/// Appends a batch three times: with the run's default key, with a key of its own, and from the system context with
/// the next default key (`<run>#1`, `batch-<run>`, `<run>#3`): an attempt run again appends nothing twice.
pub async fn load_task_batch(ctx: Ctx, p: TaskBatch) -> Result<()> {
    let table = ctx.lake().table::<TaskLoadRow>();
    table.append(loads(p.rows, 0)).await?;
    table
        .append(loads(p.rows, 0))
        .key(format!("batch-{}", ctx.job().run_id()))
        .await?;
    ctx.as_system("the batch's last part")
        .lake()
        .table::<TaskLoadRow>()
        .append(loads(p.rows, 0))
        .await?;
    Ok(())
}

/// The layers, the tables, who reads them, the models, the other plugin's table read, the histories.
pub fn declare(p: Plugin) -> Plugin {
    p.layer(RAW)
        .layer(MART)
        .layer(AUDIT)
        .lake_table::<TaskLoadRow>()
        .lake_read_as_system::<TaskLoadRow>()
        .lake_table::<TaskDay>()
        .lake_read::<TaskDay>([MEMBER, MEMBER])
        .lake_read_as_system::<TaskDay>()
        .lake_export::<TaskDay>()
        .lake_export_to_models::<TaskDay>()
        .lake_table::<TaskHours>()
        .lake_read::<TaskHours>([MEMBER])
        .model(task_hours())
        .lake_table::<IssueWeek>()
        .model(issue_week())
        .lake_table::<BudgetCopy>()
        .model(budget_copy())
        .lake_table::<LongEstimateCopy>()
        .model(long_estimate_copy())
        .lake_table::<OpenByDay>()
        .lake_read::<OpenByDay>([MEMBER])
        .model(open_by_day())
        .lake_dependency::<TrackerIssueDay>()
        .history_dependency::<Issue>()
        .history_read::<Task>([MEMBER])
        .history_read::<Task>([MEMBER])
        .history_read_as_system::<Task>()
        .history_export::<Task>()
        .history_read::<Budget>([MEMBER])
        .entity::<StageDay>()
        .access::<StageDay>(Access::build(|a| a.read(MEMBER)))
        .load(stage_day_load())
        .endpoint(
            Endpoint::new(load_tasks)
                .describe("Writes generated rows to the lake.")
                .roles([MEMBER]),
        )
        .endpoint(
            Endpoint::new(read_task_days)
                .describe("Reads the tasks of the month back from the lake.")
                .roles([MEMBER]),
        )
        .endpoint(
            Endpoint::new(refresh_tables)
                .describe("Refreshes a table computed by a model.")
                .roles([MEMBER]),
        )
        .endpoint(
            Endpoint::new(task_trail)
                .describe("Reads the history of tasks.")
                .roles([MEMBER]),
        )
        .endpoint(
            Endpoint::new(load_stage_days)
                .describe("Asks for the load of the days by stage, or follows its ticket.")
                .roles([MEMBER]),
        )
        .endpoint(
            Endpoint::new(read_stage_days)
                .describe("Reads the loaded days by stage.")
                .roles([MEMBER]),
        )
        .job(Job::new(load_task_batch).describe("Appends a batch of tasks to the lake"))
}
