// The SDK conformance plugin `jobsdemo`, TypeScript: background jobs as one small plugin, the same as the Rust and C#
// versions. A task is created and its processing enqueued in one transaction (a request that asks to fail rolls both
// back); a daily report is enqueued once per day (a singleton); a heartbeat runs every minute and logs a few lines; a
// flaky job fails its first attempt and succeeds on the second; a doomed job fails for good; a slow job works in steps
// of a second and stops when it is aborted (its timeout is 10 s). `--describe` must print
// sdk/conformance/jobs/descriptor.json.
import { setTimeout as sleep } from 'node:timers/promises';
import { Decimal, Filter, LoadTicket, daysAgo, endpoint, entity, historyOf, invalid, job, lakeTable, layer, permanent, plugin, role, t, unavailable, view } from '@celobus/sdk';
import { count, sum, type Query } from '@celobus/sdk/query';
import type { LoadTicketStatus } from '@celobus/sdk';

export const member = role({ title: 'Member', defaultFor: 'member' });

export const raw = layer({ title: 'As it arrived' });
export const mart = layer({ title: 'For reports' });
export const audit = layer({ title: 'History of records' });

// Its history keeps the changes of its status
export const task = entity({
  columns: {
    title: t.text(),
    status: t.text().history(), // new, then processed
  },
  access: (a) => {
    a.read(member);
    a.create(member);
    a.update(member);
  },
  history: { layer: audit, read: [member, member], systemRead: true, export: true },
});

// A stage of the work: the title of a task's status (`new`, `processed`)
export const stage = entity({
  columns: { name: t.text(), title: t.text() },
  access: (a) => {
    a.read(member);
  },
});

// An estimate of some work, in hours (a decimal)
export const estimate = entity({
  columns: { title: t.text(), hours: t.decimal() },
  access: (a) => {
    a.read(member);
  },
});

// The hours planned for some work: its history keeps every column but the list, the encrypted and the masked one
export const budget = entity({
  columns: {
    title: t.text(),
    hours: t.decimal().optional(),
    tags: t.array(t.text()),
    secret: t.text().optional().encrypted(),
    notes: t.text().optional(),
  },
  access: (a) => {
    a.read(member);
    a.mask('notes', [member]);
  },
  history: { layer: audit, all: true, read: [member] },
});

// --- views ----------------------------------------------------------------------------------------

/** A task's day, its title and its stage's title (the stage joined on its name, a plain column). */
export const recentTask = view({
  description: 'The titled tasks of the last 30 days, a row each, with the title of their stage.',
  columns: { day: t.date(), title: t.text(), stage: t.text().optional() },
  query: (q) =>
    q
      .from(task)
      .leftJoinOn(stage, stage.f.name.eqCol(task.f.status))
      .where(task.f.createdAt.geCol(daysAgo(30)), task.f.title.ne(''))
      .columns({ day: task.f.createdAt.day(), title: task.f.title, stage: stage.f.title }),
});

/** The recent tasks counted by week and stage: a view of a view, grouped. */
export const tasksByWeek = view({
  description: 'The recent tasks by week and stage, where a stage had more than one.',
  columns: { week: t.date(), stage: t.text().optional(), tasks: t.bigint() },
  query: (q) =>
    q
      .from(recentTask)
      .groupBy(recentTask.f.day.week(), recentTask.f.stage)
      .having(count().gt(1))
      .columns({ week: recentTask.f.day.week(), stage: recentTask.f.stage, tasks: count() }),
});

/** The estimates over 12.50 hours: a decimal column compared with a decimal (a string in the declaration). */
export const longEstimate = view({
  description: 'The estimates of more than 12.50 hours.',
  columns: { title: t.text(), hours: t.decimal() },
  query: (q) =>
    q
      .from(estimate)
      .where(estimate.f.hours.gt(Decimal.from('12.50')))
      .columns({ title: estimate.f.title, hours: estimate.f.hours }),
});

// --- jobs -----------------------------------------------------------------------------------------

export const processTask = job({
  description: 'Marks a task processed',
  payload: { task: t.ref(() => task) },
  async handler(ctx, p) {
    const row = await ctx.db(task).get(p.task);
    ctx.log.info(`processing task ${row.title}`);
    await ctx.db(task).update(p.task, { status: 'processed' });
  },
});

export const dailyReport = job({
  description: "Writes the day's report",
  payload: { day: t.text() },
  async handler(ctx, p) {
    ctx.log.info(`report for ${p.day} (run ${ctx.job.runId})`);
  },
});

export const heartbeat = job({
  description: 'Logs a heartbeat every minute',
  payload: {},
  cron: '* * * * *',
  as: 'system',
  retry: { retries: 0, delay: 0, backoff: 'fixed' },
  async handler(ctx) {
    ctx.log.info(`heartbeat at ${ctx.job.scheduledAt.toISOString()}`);
    ctx.log.info(`attempt ${ctx.job.attempt} of ${ctx.job.maxAttempts}`);
    ctx.log.warn('nothing to do');
  },
});

export const flaky = job({
  description: 'Fails its first attempt',
  payload: {},
  retry: { retries: 2, delay: '5s', backoff: 'fixed' },
  async handler(ctx) {
    if (ctx.job.attempt === 1) {
      ctx.log.warn('first attempt: failing');
      throw unavailable('the first attempt always fails');
    }
    ctx.log.info(`attempt ${ctx.job.attempt}: done`);
  },
});

export const doomed = job({
  description: 'Fails for good',
  payload: {},
  retry: { retries: 3, delay: '10s', backoff: 'exponential' },
  async handler(ctx) {
    ctx.log.error('giving up');
    throw permanent('this job always fails for good');
  },
});

export const slow = job({
  description: 'Works in steps until it is stopped',
  payload: { seconds: t.int() }, // 0: 30, past the timeout
  timeout: '10s',
  concurrency: 2,
  async handler(ctx, p) {
    const seconds = p.seconds > 0 ? p.seconds : 30;
    for (let step = 1; step <= seconds; step++) {
      // an abort stops the handler here
      await sleep(1000, undefined, { signal: ctx.signal });
      ctx.log.info(`step ${step} of ${seconds}`);
    }
  },
});

// --- the Data Lake ----------------------------------------------------------------------------------
// Two layers, a table of tasks as an outside tracker sends them (partitioned by the day of the load; its message is
// named apart from the table), a table of the tasks of a day by stage (partitioned and keyed, read by members and by
// the plugin's system context, exported), and another plugin's exported table it reads. `LoadTasks` writes them in
// each mode, `TaskDays` reads one back grouped, and the job `task_batch` appends with the run's default keys. What
// each SDK sends for them is sdk/conformance/jobs/lake.json.

export const taskLoad = lakeTable({
  layer: raw,
  message: 'TaskLoadRow',
  description: 'Tasks as an outside tracker sends them, a load a day.',
  columns: {
    loadedOn: t.date(),
    title: t.text(),
    hours: t.text(), // as the tracker wrote it
  },
  partition: ['loadedOn'],
  systemRead: true,
});

export const taskDay = lakeTable({
  layer: mart,
  description: 'The tasks of a day by stage.',
  columns: { day: t.date(), stage: t.text(), tasks: t.bigint(), hours: t.decimal(), note: t.text().optional() },
  partition: ['day'],
  key: ['day', 'stage'],
  read: [member, member],
  systemRead: true,
  export: true,
  exportToModels: true,
});

/** The tracker's issues by day, a table the plugin `tracker` exports. */
export const trackerIssueDay = lakeTable({ plugin: 'tracker', name: 'issue_day', columns: { day: t.date(), issues: t.bigint() } });

/** The tasks of a day by stage with tasks, kept in the analytic copy only: loaded from `task_day`. */
export const stageDay = entity({
  description: 'The tasks of a day by stage with tasks, kept in the analytic copy only: loaded from `task_day`.',
  storage: 'olap',
  olapKey: ['day', 'stage'],
  columns: { day: t.date(), stage: t.text(), tasks: t.bigint(), hours: t.decimal().optional() },
  access: (a) => {
    a.read(member);
  },
  load: (l) =>
    l
      .from(taskDay)
      .where(taskDay.f.tasks.gt(0n))
      .columns({ day: taskDay.f.day, stage: taskDay.f.stage, tasks: taskDay.f.tasks, hours: taskDay.f.hours }),
});

/** The tracker's issues, whose history the plugin `tracker` exports. */
export const trackerIssue = entity({ plugin: 'tracker', name: 'issue', columns: { title: t.text() } });

/** The loads' tasks with a title, their hours parsed. */
export const taskHours = lakeTable({
  layer: mart,
  description: 'The hours of the loaded tasks, as numbers: computed from `task_load`.',
  columns: { day: t.date(), title: t.text(), hours: t.decimal() },
  partition: ['day'],
  read: [member],
  model: (m) =>
    m
      .from(taskLoad)
      .where(taskLoad.f.title.ne(''))
      .columns({ day: taskLoad.f.loadedOn, title: taskLoad.f.title, hours: taskLoad.f.hours.parse('decimal') }),
});

/** The tracker's issues summed by week. */
export const issueWeek = lakeTable({
  layer: mart,
  description: "The tracker's issues by week: computed from its exported `issue_day`.",
  columns: { week: t.date(), issues: t.bigint() },
  partition: ['week'],
  model: (m) =>
    m
      .from(trackerIssueDay)
      .groupBy(trackerIssueDay.f.day.week())
      .columns({ week: trackerIssueDay.f.day.week(), issues: sum(trackerIssueDay.f.issues) }),
});

/** The budgets, their masked notes as they are. */
export const budgetCopy = lakeTable({
  layer: mart,
  description: 'The budgets with their notes: computed from the entity, its masked column as it is.',
  columns: { id: t.ref(budget), title: t.text(), notes: t.text().optional() },
  model: (m) => m.from(budget).unmasked().columns({ id: budget.f.id, title: budget.f.title, notes: budget.f.notes }),
});

/** The rows of the view `long_estimate`. */
export const longEstimateCopy = lakeTable({
  layer: mart,
  description: 'The long estimates: computed from the analytic view.',
  columns: { title: t.text(), hours: t.decimal() },
  model: (m) => m.from(longEstimate).columns({ title: longEstimate.f.title, hours: longEstimate.f.hours }),
});

/** The tasks not processed at the end of each day, counted by status. */
const taskHistory = historyOf(task);
export const openByDay = lakeTable({
  layer: mart,
  description: 'The tasks not processed at the end of each day, by status: computed from the history of tasks.',
  columns: { day: t.date(), status: t.text().optional(), tasks: t.bigint() },
  partition: ['day'],
  read: [member],
  model: (m) =>
    m
      .fromHistory(task)
      .eachDay()
      .where(task.f.status.ne('processed'))
      .groupBy(taskHistory.f.day, task.f.status)
      .columns({ day: taskHistory.f.day, status: task.f.status, tasks: count() }),
});

/** The day every conformance row is of. */
const DAY = '2026-11-20';

/** `n` rows of `task_load`: `t<i>` followed by `width` x's, 1.5 hours. */
function* loads(n: number, width: number) {
  const pad = 'x'.repeat(Math.max(0, width));
  for (let i = 0; i < n; i++) yield { loadedOn: DAY, title: `t${i}${pad}`, hours: '1.5' };
}

/** `n` rows of `task_day`: stage `s<i>`, `i` tasks, 2.25 hours, no note. */
function* days(n: number) {
  for (let i = 0; i < n; i++) yield { day: DAY, stage: `s${i}`, tasks: BigInt(i), hours: Decimal.from('2.25') };
}

/** Writes `rows` generated rows in one write: `replace` and `append` into `task_load`, `merge` into `task_day`. */
export const loadTasks = endpoint({
  description: 'Writes generated rows to the lake.',
  input: t.message('LoadTasksRequest', {
    mode: t.text(), // replace, append or merge
    rows: t.int(),
    width: t.int(), // the x's after each title
    key: t.text(), // append: the write key (none: refused outside a job's run)
  }),
  output: t.message('LoadTasksReply', { rows: t.bigint(), skipped: t.bool() }),
  roles: [member],
  async handler(ctx, req) {
    const lake = ctx.lake();
    let written;
    switch (req.mode) {
      case 'replace':
        written = await lake.table(taskLoad).replace(taskLoad.f.loadedOn.eq(DAY), loads(req.rows, req.width));
        break;
      case 'append':
        written = await lake.table(taskLoad).append(loads(req.rows, req.width), { key: req.key });
        break;
      case 'merge':
        written = await lake.table(taskDay).merge(days(req.rows));
        break;
      default:
        throw invalid(`${req.mode} is not replace, append or merge`);
    }
    return { rows: BigInt(written.rows), skipped: written.skipped };
  },
});

/** The conformance lake query: the tasks and hours of November 2026 with tasks, by month and stage. */
export function taskDays(q: Query<typeof taskDay>) {
  return q
    .where(taskDay.f.day.between('2026-11-01', '2026-11-30'), taskDay.f.tasks.gt(0n))
    .groupBy(taskDay.f.day.month(), taskDay.f.stage)
    .select({ month: taskDay.f.day.month(), stage: taskDay.f.stage, tasks: sum(taskDay.f.tasks), hours: sum(taskDay.f.hours) })
    .orderBy(taskDay.f.stage.asc())
    .limit(50);
}

export const readTaskDays = endpoint({
  description: 'Reads the tasks of the month back from the lake.',
  input: t.message('TaskDaysRequest', {}),
  output: t.message('TaskDaysReply', {
    groups: t.int(), // the groups the lake answered
  }),
  roles: [member],
  async handler(ctx) {
    return { groups: (await taskDays(ctx.lake().query(taskDay)).all()).length };
  },
});

/** Refreshes one of the computed tables, whole or the conformance day's partition. */
export const refreshTables = endpoint({
  description: 'Refreshes a table computed by a model.',
  input: t.message('RefreshTablesRequest', {
    table: t.text(), // task_hours, issue_week, budget_copy, long_estimate_copy or open_by_day
    day: t.bool(), // only the conformance day's partition
    every: t.bool(), // task_hours: a refresh of partitions naming none (refused)
  }),
  output: t.message('RefreshTablesReply', {
    rows: t.bigint(),
    asOf: t.bigint(), // when the data read is from, unix milliseconds (0: a model over lake tables)
    stalePartitions: t.bigint(), // the partitions still holding an earlier declaration's rows
  }),
  roles: [member],
  async handler(ctx, req) {
    const lake = ctx.lake();
    let done;
    switch (req.table) {
      case 'task_hours':
        done = await lake.refresh(taskHours, req.every ? { partition: Filter.all() } : req.day ? { partition: taskHours.f.day.eq(DAY) } : {});
        break;
      case 'issue_week':
        done = await lake.refresh(issueWeek, req.day ? { partition: issueWeek.f.week.eq(DAY) } : {});
        break;
      case 'budget_copy':
        done = await lake.refresh(budgetCopy);
        break;
      case 'long_estimate_copy':
        done = await lake.refresh(longEstimateCopy);
        break;
      case 'open_by_day':
        done = await lake.refresh(openByDay, req.day ? { partition: openByDay.f.day.eq(DAY) } : {});
        break;
      default:
        throw invalid(`${req.table} has no model`);
    }
    return { rows: BigInt(done.rows), asOf: BigInt(done.asOf?.getTime() ?? 0), stalePartitions: BigInt(done.stalePartitions) };
  },
});

/**
 * The conformance history read: the changes of tasks whose status is not empty, by when they were made (or, per day,
 * by day), at most 20.
 */
export const taskTrail = endpoint({
  description: 'Reads the history of tasks.',
  input: t.message('TaskTrailRequest', {
    eachDay: t.bool(), // one row per day instead of one per change
  }),
  output: t.message('TaskTrailReply', {
    rows: t.int(),
    // the first row's change, its author's kind and plugin, its day ("" for none) and status ("" for none)
    change: t.text(),
    changedByKind: t.text(),
    changedByPlugin: t.text(),
    day: t.text(),
    status: t.text(),
    // when the first row's version was made and ended, unix milliseconds (0: none)
    validFrom: t.bigint(),
    validTo: t.bigint(),
  }),
  roles: [member],
  async handler(ctx, req) {
    const q = ctx.lake().history(task);
    const rows = await (req.eachDay ? q.eachDay() : q)
      .where(task.f.status.ne(''))
      .orderBy(req.eachDay ? taskHistory.f.day.asc() : taskHistory.f.validFrom.asc())
      .limit(20)
      .all();
    const first = rows[0];
    if (!first) {
      return { rows: 0, change: '', changedByKind: '', changedByPlugin: '', day: '', status: '', validFrom: 0n, validTo: 0n };
    }
    return {
      rows: rows.length,
      change: first.change,
      changedByKind: first.changedByKind,
      changedByPlugin: first.changedByPlugin ?? '',
      day: first.day ?? '',
      status: first.status ?? '',
      validFrom: BigInt(first.validFrom.getTime()),
      validTo: BigInt(first.validTo?.getTime() ?? 0),
    };
  },
});

const ticketReply = (t: LoadTicketStatus) => ({
  ticket: t.id,
  state: t.state,
  generation: BigInt(t.generation),
  entities: t.entities,
  rows: BigInt(t.loads[0]?.rows ?? 0),
  code: t.code,
});

/** Asks for a snapshot that loads `stage_day`, asks how its ticket stands, or waits for its end. */
export const loadStageDays = endpoint({
  description: 'Asks for the load of the days by stage, or follows its ticket.',
  input: t.message('LoadStageDaysRequest', {
    step: t.text(), // request, status or wait
    ticket: t.text(), // status and wait: the ticket asked about
  }),
  output: t.message('LoadStageDaysReply', {
    ticket: t.text(),
    state: t.text(), // pending, building, done or failed
    generation: t.bigint(),
    // the entities it loads, and the rows of its first load (0 before)
    entities: t.array(t.text()),
    rows: t.bigint(),
    code: t.text(), // a failed ticket's first failure
  }),
  roles: [member],
  async handler(ctx, req) {
    switch (req.step) {
      case 'request':
        return ticketReply((await ctx.analytics().snapshot().load(stageDay).request()).last);
      case 'status':
        return ticketReply(await LoadTicket.of(ctx, req.ticket).status());
      case 'wait':
        return ticketReply(await LoadTicket.of(ctx, req.ticket).wait());
      default:
        throw invalid(`${req.step} is not request, status or wait`);
    }
  },
});

/** Reads the loaded days of `stage_day` from the analytic copy. */
export const readStageDays = endpoint({
  description: 'Reads the loaded days by stage.',
  input: t.message('StageDaysRequest', {}),
  output: t.message('StageDaysReply', {
    rows: t.int(), // the rows the analytic copy answered
  }),
  roles: [member],
  async handler(ctx) {
    const rows = await ctx
      .analytics()
      .query(stageDay)
      .where(stageDay.f.tasks.gt(0n))
      .select({ day: stageDay.f.day, stage: stageDay.f.stage, tasks: stageDay.f.tasks, hours: stageDay.f.hours })
      .orderBy(stageDay.f.day.asc())
      .all();
    return { rows: rows.length };
  },
});

/**
 * Appends a batch three times: with the run's default key, with a key of its own, and from the system context with
 * the next default key (`<run>#1`, `batch-<run>`, `<run>#3`): an attempt run again appends nothing twice.
 */
export const taskBatch = job({
  description: 'Appends a batch of tasks to the lake',
  payload: { rows: t.int() },
  async handler(ctx, p) {
    const table = ctx.lake().table(taskLoad);
    await table.append(loads(p.rows, 0));
    await table.append(loads(p.rows, 0), { key: `batch-${ctx.job.runId}` });
    await ctx.asSystem("the batch's last part").lake().table(taskLoad).append(loads(p.rows, 0));
  },
});

// --- endpoints ------------------------------------------------------------------------------------

/** Creates a task and enqueues its processing in one transaction: both exist only if it commits. */
export const createTask = endpoint({
  description: 'Creates a task and enqueues its processing.',
  input: t.message('CreateTaskRequest', {
    title: t.text(),
    fail: t.bool(), // true: the request fails after the enqueue, and the task and its run roll back together
  }),
  output: t.message('CreateTaskReply', { task: t.ref(() => task), runId: t.text() }),
  roles: [member],
  handler: (ctx, req) =>
    ctx.transaction(async (tx) => {
      const created = await tx.db(task).create({ title: req.title, status: 'new' });
      const run = await tx.jobs.enqueue(processTask, { task: created.id });
      if (req.fail) throw invalid('asked to fail: the task and its run are rolled back');
      return { task: created.id, runId: run.runId };
    }),
});

/** Enqueues the day's report once: a second request for the same day answers the first run. */
export const enqueueReport = endpoint({
  description: "Enqueues the day's report once.",
  input: t.message('EnqueueReportRequest', { day: t.text() }), // 2026-11-20
  output: t.message('EnqueueReportReply', {
    runId: t.text(),
    existing: t.bool(), // the day's report was enqueued before
  }),
  roles: [member],
  async handler(ctx, req) {
    const run = await ctx.jobs.enqueue(dailyReport, { day: req.day }, { singleton: `report:${req.day}` });
    return { runId: run.runId, existing: run.existing };
  },
});

export const cancelRun = endpoint({
  description: 'Cancels a run.',
  input: t.message('CancelRunRequest', { runId: t.text() }),
  output: t.message('CancelRunReply', {
    cancelled: t.bool(), // false: the run had already ended
  }),
  roles: [member],
  async handler(ctx, req) {
    return { cancelled: await ctx.jobs.cancel(req.runId) };
  },
});

export default plugin({
  id: 'jobsdemo',
  title: 'Jobs demo',
  description: 'The SDK conformance plugin of background jobs.',
  roles: { member },
  capabilities: {
    'authz:bypass': {
      reason: "The lake's models copy the estimates and the budgets",
      on: [
        { entity: estimate, ops: ['read'] },
        { entity: budget, ops: ['read'] },
      ],
    },
  },
  entities: { task, stage, estimate, budget, stageDay },
  views: { recentTask, tasksByWeek, longEstimate },
  endpoints: { createTask, enqueueReport, cancelRun, loadTasks, taskDays: readTaskDays, refreshTables, taskTrail, loadStageDays, stageDays: readStageDays },
  jobs: { processTask, dailyReport, heartbeat, flaky, doomed, slow, taskBatch },
  layers: { raw, mart, audit },
  lakeTables: { taskLoad, taskDay, taskHours, issueWeek, budgetCopy, longEstimateCopy, openByDay },
  lakeDependencies: [trackerIssueDay],
  historyDependencies: [trackerIssue],
});
