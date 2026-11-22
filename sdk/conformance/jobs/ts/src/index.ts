// The SDK conformance plugin `jobsdemo`, TypeScript: background jobs as one small plugin, the same as the Rust and C#
// versions. A task is created and its processing enqueued in one transaction (a request that asks to fail rolls both
// back); a daily report is enqueued once per day (a singleton); a heartbeat runs every minute and logs a few lines; a
// flaky job fails its first attempt and succeeds on the second; a doomed job fails for good; a slow job works in steps
// of a second and stops when it is aborted (its timeout is 10 s). `--describe` must print
// sdk/conformance/jobs/descriptor.json.
import { setTimeout as sleep } from 'node:timers/promises';
import { endpoint, entity, invalid, job, permanent, plugin, role, t, unavailable } from '@celobus/sdk';

export const member = role({ title: 'Member', defaultFor: 'member' });

export const task = entity({
  columns: {
    title: t.text(),
    status: t.text(), // new, then processed
  },
  access: (a) => {
    a.read(member);
    a.create(member);
    a.update(member);
  },
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
  entities: { task },
  endpoints: { createTask, enqueueReport, cancelRun },
  jobs: { processTask, dailyReport, heartbeat, flaky, doomed, slow },
});
