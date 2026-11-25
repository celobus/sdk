// Static types: handlers get typed input and must return typed output. Compiled (not run) by `npm test`;
// each `@ts-expect-error` must be an error.
import { Decimal, daysAgo, endpoint, entity, historyOf, job, lakeTable, layer, plugin, remote, role, t, testCtx, view, type Ctx, type Dependency, type Enqueued, type Row, type SqlRow, type ValueOf } from '@celobus/sdk';
import * as platform from '@celobus/sdk/platform';
import { alias, avg, count, exists, label, max, sum, type Label } from '@celobus/sdk/query';

const mood = t.enum('Mood', ['happy', 'sad']);
const line = t.message('Line', { sku: t.text(), quantity: t.int(), price: t.decimal() });
const request = t.message('Req', {
  name: t.text(),
  mood: mood.optional(),
  lines: t.array(line),
  count: t.bigint(),
  at: t.timestamp().optional(),
  ownerId: t.ref(platform.user),
});
const note = entity({ key: 'i64', columns: { title: t.text(), body: t.text().optional() } });

endpoint({
  input: request,
  output: { total: t.decimal(), mood },
  handler(_ctx, input) {
    const name: string = input.name;
    const m: 'happy' | 'sad' | undefined = input.mood;
    const n: bigint = input.count;
    const d: Date | undefined = input.at;
    const owner: string = input.ownerId;
    // @ts-expect-error an optional field may be undefined
    const at: Date = input.at;
    // @ts-expect-error no such field
    input.nope;
    void [name, m, n, d, owner, at];
    return { total: Decimal.sum(input.lines.map((l) => l.price.mul(l.quantity))), mood: 'happy' as const };
  },
});

endpoint({
  input: { id: t.ref(() => note) },
  output: note,
  async handler(ctx, { id }) {
    const row: Row<typeof note> = await ctx.db(note).get(id);
    return row;
  },
});

endpoint({
  input: {},
  output: { text: t.text() },
  // @ts-expect-error the output must have `text`
  handler: () => ({ txt: 'x' }),
});

// the runs of the plugin's agents are typed
endpoint({
  input: { question: t.text() },
  output: { runId: t.text(), answer: t.text() },
  async handler(ctx, { question }) {
    const run = await ctx.agents.run('assistant', question, { engine: 'pi', session: true, timeout: '10m' });
    const status: 'starting' | 'running' | 'idle' | 'succeeded' | 'failed' | 'cancelled' | 'timed_out' = run.status;
    const page = await ctx.agents.events(run.id, { after: run.lastSeq, limit: 50 });
    const seq: number = page.events[0]?.seq ?? (await ctx.agents.send(run.id, 'and coffee?'));
    const text: unknown = page.events[0]?.text;
    const runs = await ctx.agents.list({ agent: 'assistant', limit: 10 });
    // a run joins the group of another; a run says its group, what started it and for whom
    const joined = await ctx.agents.run('packer', question, { group: run.group });
    const inGroup = await ctx.agents.list({ group: run.group });
    const parent: string | undefined = joined.parent ?? joined.parentAgent;
    const root: string = joined.onBehalfOf.userId + joined.onBehalfOf.displayName;
    // @ts-expect-error a group is the id of its first run
    await ctx.agents.run('packer', question, { group: 7 });
    // @ts-expect-error not an engine
    await ctx.agents.run('assistant', question, { engine: 'gpt' });
    // @ts-expect-error a timeout is '90s', '15m' or '2h'
    await ctx.agents.run('assistant', question, { timeout: 600 });
    // @ts-expect-error no such field of a run
    run.nope;
    void [status, seq, text, runs, inGroup, parent, root, await ctx.agents.cancel(run.id), await ctx.agents.close(run.id)];
    return { runId: run.id, answer: (await ctx.agents.get(run.id)).result ?? page.partial.text };
  },
});

endpoint({
  input: {},
  output: { mood },
  // @ts-expect-error not a value of the enum
  handler: () => ({ mood: 'angry' }),
});

endpoint({
  input: {},
  output: note,
  // @ts-expect-error a row needs its id and required columns
  handler: () => ({ body: 'x' }),
});

// --- a dependency's entity, as `celobus plugin add` generates it, and SQL sent as text ----------------
export const PLUGIN = 'customers';
/** `customers.customer`: read with `ctx.query(customers.customer)`. */
export const customer = entity({
  plugin: PLUGIN,
  name: 'customer',
  key: 'uuid',
  softDelete: true,
  version: true,
  columns: {
    name: t.text(),
    email: t.text().optional(),
  },
});
export const dependency = {
  plugin: PLUGIN,
  calls: ['GetCustomer'],
  reads: [{ entity: 'customer', fields: [{ name: 'id', type: 'ref:customers.customer' }, { name: 'name', type: 'string' }, { name: 'email', type: 'optional string' }] }],
};
const getCustomer = remote(PLUGIN, 'GetCustomer', t.message('GetCustomerRequest', { id: t.text() }), t.message('C', { id: t.text() }));
type Customer = ValueOf<typeof customer.columns.email>;
const noEmail: Customer = 'a@b.c';
const dep: Dependency = dependency;
void [getCustomer, dep, noEmail];

plugin({ id: 'typed', dependencies: [dependency], entities: { note } });

endpoint({
  input: {},
  output: { n: t.int() },
  async handler(ctx) {
    const customers = ctx.query(customer);
    const c = await customers.get('x', { withDeleted: true });
    const name: string = c.name;
    const email: string | undefined = c.email;
    const page = await customers.find(customer.f.name.eq('a')).orderBy(customer.f.name.asc()).limit(5).pageToken('p').withDeleted().page();
    const n: number = await customers.count(customer.f.email.isNull());
    const any: boolean = await customers.exists(customer.f.name.eq('b'));
    const all = await customers.all();
    // @ts-expect-error a dependency's entity has no writes
    customers.create({ name: 'x' });
    // @ts-expect-error a filter on another entity
    customers.find(note.f.title.eq('a'));
    // @ts-expect-error a dependency's entity is read with ctx.query
    ctx.db(customer);
    // an own entity is the root of a query too
    const ownNotes: number = await ctx.query(note).count();
    void ownNotes;

    const rows = await ctx.sql('select owner_id, count(*) as notes from typed_note group by 1 having count(*) >= $1', 2n);
    const cell: unknown = rows[0]!['notes'];
    const typed = await ctx.sql<{ owner_id: string; notes: number }>('select owner_id, count(*) as notes from typed_note group by 1');
    const notes: number = typed[0]!.notes;
    const owner: string = typed[0]!.owner_id;
    const plain = await ctx.sql('select 1 as n');
    await ctx.transaction(async (tx) => tx.sql('select $1::int as n', 1, 'two', null, new Date()));
    const written: SqlRow[] = await ctx.unsafeSql('update typed_note set title = $1 returning id', 'x');
    // @ts-expect-error the query is text
    await ctx.sql(1);
    void [cell, written];
    void [name, email, page, any, all, notes, owner, plain];
    return { n };
  },
});

// search and contains are for text columns; row locks are for a transaction's repositories
const item = entity({ key: 'string', columns: { name: t.text().searchable(), qty: t.int(), price: t.decimal(), ownerId: t.ref(platform.user) } });
endpoint({
  input: {},
  output: { n: t.int() },
  async handler(ctx) {
    ctx.db(item).find(item.f.name.search('red').and(item.f.name.contains('re')));
    // @ts-expect-error search is a filter of text columns
    item.f.qty.search('1');
    // a decimal compares with a Decimal (sent as its text, exact), never a binary number
    ctx.db(item).find(item.f.price.gt(Decimal.from('1.50')));
    // @ts-expect-error a number is not a Decimal: Decimal.from('1.50')
    item.f.price.gt(1.5);
    // @ts-expect-error row locks exist only inside a transaction
    ctx.db(item).find().forUpdate();
    const locked = await ctx.transaction(async (tx) => tx.db(item).find().forUpdate().skipLocked().limit(5).page());
    const one = await ctx.transaction(async (tx) => tx.db(item).get('A', { lock: 'share', wait: 'nowait', include: ['owner'] }));
    const byOwner = await ctx.db(item).aggregate({ groupBy: 'ownerId', count: true });
    const owner: string | undefined = byOwner[0]?.ownerId;
    const count: number | undefined = byOwner[0]?.count;
    const total = await ctx.db(item).aggregate({ sum: 'price' });
    const sum: Decimal | undefined = total[0]?.sum;
    // @ts-expect-error groupBy names a column
    await ctx.db(item).aggregate({ groupBy: 'nope', count: true });
    const changed: number = await ctx.db(item).updateWhere(item.f.qty.eq(0), { qty: 1 });
    const saved = await ctx.db(item).upsert('name', { id: 'A', name: 'a', qty: 1, price: Decimal.from(1), ownerId: '' });
    const inc = (await ctx.db(item).find().include('owner').all())[0]?.included?.owner;
    void [locked, one, owner, count, sum, changed, saved, inc];
    return { n: 0 };
  },
});

// a relation through a junction's rows names the junction's own columns
const member = entity({ columns: { noteId: t.ref(() => note), userId: t.ref(platform.user) } });
entity({
  columns: { title: t.text() },
  access: (a) => {
    a.read(a.relation('member', a.rows(member, 'noteId', 'userId')));
    // @ts-expect-error not a column of the junction
    a.rows(member, 'noteId', 'ownerId');
  },
});

// typed queries across entities: joins along refs of the right entity only, columns of the query's entities only, a
// left-joined entity's columns may be undefined, columns compare with columns of their type
const buyer = entity({ plugin: 'crm', name: 'buyer', key: 'uuid', columns: { name: t.text(), ownerId: t.ref(platform.user) } });
const sale = entity({
  columns: { number: t.text(), total: t.decimal(), qty: t.int(), buyerId: t.ref(buyer).optional(), createdBy: t.ref(platform.user) },
});
const saleLine = entity({ columns: { saleId: t.ref(sale), sku: t.text(), amount: t.decimal() } });
endpoint({
  input: {},
  output: { n: t.int() },
  async handler(ctx) {
    const manager = alias(platform.user, 'manager').left();
    const rows = await ctx
      .query(saleLine)
      .join(sale, saleLine.f.saleId)
      .leftJoin(buyer, sale.f.buyerId)
      .joinAs(manager, buyer.f.ownerId)
      .where(sale.f.qty.gt(1), exists(platform.roleAssignment, platform.roleAssignment.f.userId.eqCol(sale.f.createdBy)))
      .groupBy(sale.f.id)
      .having(count().ge(2))
      .select({
        number: sale.f.number,
        buyer: buyer.f.name,
        manager: manager.f.displayName,
        by: label(sale.f.createdBy),
        revenue: sum(saleLine.f.amount),
        lines: count(),
        biggest: max(saleLine.f.amount),
        avgQty: avg(sale.f.qty),
      })
      .orderBy({ revenue: 'desc' })
      .limit(20)
      .all();
    const number: string = rows[0]!.number;
    const name: string | undefined = rows[0]!.buyer;
    const managerName: string | undefined = rows[0]!.manager;
    const by: Label | undefined = rows[0]!.by;
    const revenue: Decimal = rows[0]!.revenue;
    const lines: number = rows[0]!.lines;
    const biggest: Decimal | undefined = rows[0]!.biggest;
    // @ts-expect-error a left-joined entity's column may be undefined
    const sure: string = rows[0]!.buyer;
    // @ts-expect-error `buyer.f.ownerId` is not a ref to a sale line
    ctx.query(sale).join(saleLine, buyer.f.ownerId);
    // @ts-expect-error the query has no buyer
    ctx.query(sale).select({ n: buyer.f.name });
    // @ts-expect-error a filter of an entity the query does not have
    ctx.query(sale).where(buyer.f.name.eq('a'));
    // @ts-expect-error a sale's id is not a buyer's
    ctx.query(sale).joinOn(buyer, sale.f.id.eqCol(buyer.f.id));
    const page = await ctx.query(platform.groupMember).join(platform.user, platform.groupMember.f.userId).select({ name: platform.user.f.displayName }).page();
    const first: string | undefined = page.rows[0]?.name;
    // the analytic copy: the same builder, and answers that say when their data is from
    const sales = await ctx.analytics().query(saleLine).join(sale, saleLine.f.saleId).groupBy(sale.f.number)
      .select({ number: sale.f.number, revenue: sum(saleLine.f.amount) }).allAsOf();
    const salesRevenue: Decimal = sales.rows[0]!.revenue;
    const salesAt: Date | undefined = sales.asOf;
    const counted: { rows: number; asOf?: Date } = await ctx.analytics().query(sale).countAsOf();
    void [number, name, managerName, by, revenue, lines, biggest, sure, first, salesRevenue, salesAt, counted];
    return { n: 0 };
  },
});

// Background jobs: the handler gets the payload's type; an enqueue takes it, and answers the run.
const remind = job({
  payload: { olderThanDays: t.int(), since: t.timestamp().optional() },
  cron: '0 9 * * 1-5',
  as: 'system',
  retry: { retries: 3, delay: '1m', backoff: 'exponential' },
  timeout: '2m',
  async handler(ctx, p) {
    const days: number = p.olderThanDays;
    const at: Date = ctx.job.scheduledAt;
    // @ts-expect-error the payload has no such field
    void p.orderId;
    // @ts-expect-error a number of days is not a string
    const wrong: string = p.olderThanDays;
    const run: Enqueued = await ctx.jobs.enqueue(remind, { olderThanDays: days }, { delay: '1h', singleton: 'x' });
    await ctx.jobs.cancel(run);
    // @ts-expect-error the payload's field is missing
    await ctx.jobs.enqueue(remind, {});
    // @ts-expect-error the backoff is fixed or exponential
    job({ payload: {}, retry: { backoff: 'linear' }, handler() {} });
    void [at, wrong];
  },
});
const reindex = job({ payload: t.message('Reindex', { full: t.bool() }), handler: (_ctx, p) => void (p.full satisfies boolean) });
plugin({ id: 'typed', jobs: { remind, reindex }, jobSlots: 8 });

// testCtx: a context of the plugin without the platform
const auditor = role({ title: 'Auditor' });
const tested = plugin({ id: 'tested', roles: { auditor } });
const asAuditor: Ctx = testCtx(tested, { caller: { userId: 'u1', roles: [auditor, 'workspace.member'], scopes: ['read'] } });
const inRun: Ctx = testCtx(tested, { run: { runId: 'run-1', attempt: 1, maxAttempts: 4 }, idempotencyKey: 'k1' });
const key: string | undefined = inRun.idempotencyKey;
// @ts-expect-error a test caller is a person or a service account
testCtx(tested, { caller: { userId: 'u1', kind: 'system' } });
// @ts-expect-error a run needs its attempts
testCtx(tested, { run: { runId: 'run-1' } });
void [asAuditor, key];

// analytic views: a row type of their own, read never written, typed columns, buckets and relative days
const visit = entity({ columns: { page: t.text(), at: t.timestamp(), ms: t.int() } });
const visits = view({
  description: 'One row per visit.',
  columns: { day: t.date(), page: t.text(), ms: t.int() },
  query: (q) =>
    q.from(visit).where(visit.f.at.geCol(daysAgo(30))).columns({ day: visit.f.at.day(), page: visit.f.page, ms: visit.f.ms }),
});
view({
  columns: { day: t.date(), ms: t.int() },
  // @ts-expect-error every column of the row is given
  query: (q) => q.from(visit).columns({ day: visit.f.at.day() }),
});
view({
  columns: { page: t.text() },
  // @ts-expect-error a column of another type than its field's
  query: (q) => q.from(visit).columns({ page: visit.f.ms }),
});
// @ts-expect-error a number column has no buckets
visit.f.ms.month();
// @ts-expect-error a text column does not compare with a number column
visit.f.page.eqCol(visit.f.ms);
plugin({ id: 'viewed', entities: { visit }, views: { visits } });
endpoint({
  input: {},
  output: { days: t.array(t.date()) },
  async handler(ctx) {
    const rows = await ctx.analytics().query(visits).where(visits.f.day.between('2026-01-01', '2026-12-31')).allAsOf();
    const day: string = rows.rows[0]!.day;
    const months = await ctx.query(visits).groupBy(visits.f.day.month()).select({ month: visits.f.day.month(), n: count() }).all();
    const month: string = months[0]!.month;
    // @ts-expect-error a view is read, never written
    ctx.db(visits);
    return { days: [day, month] };
  },
});

// --- the Data Lake: an append outside a job's run names its key; a run's has a default ---------------------------
const lakeRaw = layer({ title: 'As it arrived' });
const loads = lakeTable({ layer: lakeRaw, columns: { day: t.date(), sku: t.text(), qty: t.bigint().optional() }, partition: ['day'] });
const theirs = lakeTable({ plugin: 'erp', name: 'deliveries', columns: { day: t.date() } });
// @ts-expect-error a partition column is one of the columns
lakeTable({ layer: lakeRaw, columns: { day: t.date() }, partition: ['nope'] });

endpoint({
  input: {},
  output: { rows: t.int() },
  async handler(ctx) {
    const rows = [{ day: '2026-11-20', sku: 'a' }];
    // @ts-expect-error outside a job's run an append names its write key
    await ctx.lake().table(loads).append(rows);
    const w = await ctx.lake().table(loads).append(rows, { key: 'erp-2026-11-20' });
    await ctx.lake().table(loads).replace(loads.f.day.eq('2026-11-20'), rows);
    // @ts-expect-error a row has the table's columns
    await ctx.lake().table(loads).merge([{ day: '2026-11-20', nope: 1 }]);
    // @ts-expect-error a plugin writes only its own tables
    ctx.lake().table(theirs);
    // @ts-expect-error a lake table is read with ctx.lake().query()
    ctx.query(loads);
    const read: { day: string; sku: string; qty?: bigint }[] = await ctx.lake().query(loads).where(loads.f.sku.eq('a')).all();
    const grouped: { month: string; n: number }[] = await ctx.lake().query(loads).groupBy(loads.f.day.month()).select({ month: loads.f.day.month(), n: count() }).all();
    void [read, grouped, theirs];
    return { rows: w.rows };
  },
});

job({
  payload: {},
  async handler(ctx) {
    // in a run: the run's id and the append's number
    const w: { rows: number; skipped: boolean } = await ctx.lake().table(loads).append([{ day: '2026-11-20', sku: 'a' }]);
    void w;
  },
});

// a job's handler under test: testCtx with a run is the run's context
{
  const p = plugin({ id: 'jobtest', version: '1.0.0' });
  const nightly = job({ payload: {}, async handler(ctx) { await ctx.lake().table(loads).append([]); } });
  void nightly.options.handler(testCtx(p, { run: { runId: 'run-1', attempt: 1, maxAttempts: 3 } }), {});
}

// --- models of the lake and entities' histories -----------------------------------------------------------------
{
  const audit = layer({ title: 'History of records' });
  const task = entity({ columns: { title: t.text(), status: t.text().history(), hours: t.decimal().optional() }, history: { layer: audit } });
  const h = historyOf(task);
  const file = lakeTable({ layer: lakeRaw, columns: { day: t.date(), qty: t.text() }, partition: ['day'] });
  lakeTable({
    layer: lakeRaw,
    columns: { day: t.date(), qty: t.decimal() },
    model: (m) => m.from(file).columns({ day: file.f.day, qty: file.f.qty.parse('decimal') }),
  });
  lakeTable({
    layer: lakeRaw,
    columns: { day: t.date(), qty: t.decimal() },
    // @ts-expect-error a parsed column is of the type it is parsed as: an int64 is no decimal
    model: (m) => m.from(file).columns({ day: file.f.day, qty: file.f.qty.parse('int64') }),
  });
  lakeTable({
    layer: lakeRaw,
    columns: { day: t.date(), qty: t.decimal() },
    // @ts-expect-error every column of the table is given
    model: (m) => m.from(file).columns({ day: file.f.day }),
  });
  // @ts-expect-error only a text is parsed
  task.f.hours.parse('decimal');
  // @ts-expect-error a parse names one of the lake's types
  file.f.qty.parse('money');
  lakeTable({
    layer: lakeRaw,
    columns: { day: t.date(), status: t.text().optional(), n: t.bigint() },
    model: (m) =>
      m
        .fromHistory(task)
        .eachDay()
        .where(task.f.status.ne('done'), h.f.change.ne('delete'))
        .groupBy(h.f.day, task.f.status)
        .columns({ day: h.f.day, status: task.f.status, n: count() }),
  });

  endpoint({
    input: {},
    output: { rows: t.int() },
    async handler(ctx) {
      const r: { rows: number; asOf?: Date } = await ctx.lake().refresh(file, { partition: file.f.day.eq('2026-11-20') });
      // @ts-expect-error a plugin refreshes only its own tables
      await ctx.lake().refresh(theirs);
      // @ts-expect-error the partition filter is of the table's columns
      await ctx.lake().refresh(file, { partition: loads.f.day.eq('2026-11-20') });
      const rows = await ctx.lake().history(task).where(task.f.status.eq('open')).orderBy(h.f.validFrom.desc()).all();
      const first = rows[0]!;
      // a kept column may be absent in a row of the history, whatever the field says
      const status: string | undefined = first.status;
      // @ts-expect-error not a string: it may be absent
      const required: string = first.status;
      const when: Date = first.validFrom;
      const to: Date | undefined = first.validTo;
      const change: 'baseline' | 'insert' | 'update' | 'delete' | 'restore' = first.change;
      const perDay: { day: string; n: number }[] = await ctx.lake().history(task).eachDay().groupBy(h.f.day).select({ day: h.f.day, n: count() }).all();
      // @ts-expect-error eachDay reads a history only
      ctx.lake().query(file).eachDay();
      // @ts-expect-error a history is not a lake table
      ctx.lake().query(task);
      void [status, required, when, to, change, perDay];
      return { rows: r.rows };
    },
  });
  // @ts-expect-error a dependency's history is another plugin's entity
  plugin({ id: 'hist', version: '1.0.0', historyDependencies: [task] });
}

// --- entities stored in OLAP: read with ctx.analytics() only ---------------------------------------------------------
{
  const mart = layer({ title: 'For reports' });
  const facts = lakeTable({ layer: mart, columns: { day: t.date(), visits: t.bigint(), note: t.text() }, partition: ['day'] });
  const daily = entity({
    storage: 'olap',
    olapKey: ['day'],
    columns: { day: t.date(), visits: t.bigint(), note: t.text().optional() },
    load: (l) => l.from(facts).columns({ day: facts.f.day, visits: facts.f.visits }),
  });
  entity({
    storage: 'olap',
    olapKey: ['day'],
    columns: { day: t.date(), visits: t.bigint() },
    // @ts-expect-error a required column is given
    load: (l) => l.from(facts).columns({ day: facts.f.day }),
  });
  entity({
    storage: 'olap',
    // @ts-expect-error the key names columns
    olapKey: ['nope'],
    columns: { day: t.date() },
  });
  endpoint({
    input: {},
    output: { rows: t.int() },
    async handler(ctx) {
      // @ts-expect-error an entity stored in OLAP is never read nor written with ctx.db
      ctx.db(daily);
      // @ts-expect-error nor read with ctx.query
      ctx.query(daily);
      const rows: { day: string; visits: bigint; note?: string }[] = await ctx.analytics().query(daily).all();
      const ticket = await ctx.analytics().snapshot().load(daily).request();
      // @ts-expect-error only an entity stored in OLAP is loaded
      ctx.analytics().snapshot().load(note);
      const done: 'pending' | 'building' | 'done' | 'failed' = (await ticket.wait()).state;
      void done;
      return { rows: rows.length };
    },
  });
}
