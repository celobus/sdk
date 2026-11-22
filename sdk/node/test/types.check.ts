// Static types: handlers get typed input and must return typed output. Compiled (not run) by `npm test`;
// each `@ts-expect-error` must be an error.
import { Decimal, endpoint, entity, job, plugin, remote, t, type Dependency, type Enqueued, type Row, type SqlRow, type ValueOf } from '@celobus/sdk';
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
    void [number, name, managerName, by, revenue, lines, biggest, sure, first];
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
