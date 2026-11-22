// The conformance plugin of sdk/conformance/shop/ts/src/index.ts, in JavaScript against the build, with the
// version written out (the tests do not run from the plugin's package).
import { Decimal, endpoint, plugin, role, t, topic, ui } from '../../dist/index.js';

// --- identity -------------------------------------------------------------------------------------

export const clerk = role({ title: 'Order clerk' });

// --- data -----------------------------------------------------------------------------------------

export const orderStatus = t.enum('OrderStatus', ['placed', 'paid', 'cancelled']);

export const order = t.message('Order', {
  id: t.uuid(),
  number: t.text(),
  customer: t.text(),
  email: t.text(),
  total: t.decimal(),
  currency: t.text(),
  status: orderStatus,
  placedAt: t.timestamp(),
});

export const listOrdersRequest = t.message('ListOrdersRequest', {
  status: orderStatus.optional(),
  text: t.text(),
  pageToken: t.text(),
  pageSize: t.int(),
});
export const listOrdersResponse = t.message('ListOrdersResponse', {
  orders: t.array(order),
  nextPageToken: t.text(),
  total: t.bigint(),
});
export const getOrderRequest = t.message('GetOrderRequest', { id: t.uuid() });
export const cancelOrderRequest = t.message('CancelOrderRequest', { id: t.uuid(), reason: t.text() });
export const countOpenRequest = t.message('CountOpenRequest', {});
export const countResponse = t.message('CountResponse', { count: t.bigint() });

export const daySales = t.message('DaySales', {
  day: t.date(),
  channel: t.text(),
  total: t.decimal(),
  online: t.decimal(),
  retail: t.decimal(),
});
export const salesRequest = t.message('SalesRequest', { channel: t.text(), range: t.text() });
export const salesResponse = t.message('SalesResponse', { days: t.array(daySales), revenue: t.decimal() });
export const product = t.message('Product', { name: t.text(), revenue: t.decimal(), units: t.int() });
export const topProductsRequest = t.message('TopProductsRequest', {});
export const topProductsResponse = t.message('TopProductsResponse', { products: t.array(product) });

// the activity of an order (catalog 3)
export const person = t.message('Person', {
  id: t.text(),
  name: t.text(),
  avatar: t.text(),
  agent: t.bool(),
  icon: t.text(),
  engine: t.text(),
  role: t.text(),
  online: t.bool(),
});
export const reaction = t.message('Reaction', { emoji: t.text(), count: t.int(), mine: t.bool() });
export const jobStep = t.message('JobStep', { kind: t.text(), title: t.text(), detail: t.text() });
export const job = t.message('Job', {
  state: t.text(),
  label: t.text(),
  meta: t.text(),
  started: t.bigint(),
  rows: t.array(jobStep),
  steps: t.int(),
  retryLabel: t.text(),
});
export const replies = t.message('Replies', { count: t.int(), last: t.text() });
export const note = t.message('Note', {
  id: t.text(),
  by: t.text(),
  time: t.text(),
  text: t.text(),
  kind: t.text(),
  edited: t.bool(),
  mine: t.bool(),
  pinned: t.bool(),
  receipt: t.text(),
  receiptTone: t.text(),
  reactions: t.array(reaction),
  replies: replies.optional(),
  job: job.optional(),
  jobId: t.text(),
});
export const packing = t.message('Packing', {
  job: t.text(),
  agent: t.text(),
  icon: t.text(),
  engine: t.text(),
  state: t.text(),
  since: t.bigint(),
  until: t.bigint(),
  sub: t.text(),
  note: t.text(),
});
export const getActivityRequest = t.message('GetActivityRequest', { order: t.text(), pageToken: t.text(), tab: t.text() });
export const activityView = t.message('ActivityView', {
  title: t.text(),
  status: t.text(),
  ends: t.bigint(),
  notes: t.array(note),
  people: t.array(person),
  olderPageToken: t.text(),
  pinned: t.int(),
  closed: t.bool(),
  placeholder: t.text(),
  watching: t.bool(),
});
export const getLiveRequest = t.message('GetLiveRequest', { order: t.text() });
export const liveView = t.message('LiveView', {
  notes: t.array(note),
  people: t.array(person),
  jobs: t.array(packing),
  label: t.text(),
  hint: t.text(),
  canStop: t.bool(),
});
export const getWatchersRequest = t.message('GetWatchersRequest', { order: t.text() });
export const watchersView = t.message('WatchersView', { people: t.array(person) });
export const previewNoteRequest = t.message('PreviewNoteRequest', { order: t.text(), text: t.text() });
export const previewView = t.message('PreviewView', { hint: t.text() });
export const getJobRequest = t.message('GetJobRequest', { id: t.text(), call: t.text() });
export const jobView = t.message('JobView', { title: t.text(), call: jobStep.optional() });
export const addNoteRequest = t.message('AddNoteRequest', { order: t.text(), text: t.text(), reply: t.text(), urgent: t.bool() });
export const addNoteResponse = t.message('AddNoteResponse', { note: note.optional() });
export const noteActionRequest = t.message('NoteActionRequest', { note: t.text(), action: t.text(), text: t.text(), emoji: t.text() });
export const watchRequest = t.message('WatchRequest', { order: t.text(), watch: t.bool() });
export const assignRequest = t.message('AssignRequest', { order: t.text(), people: t.text() });
export const done = t.message('Done', { ok: t.bool() });

export const orderKey = t.message('OrderKey', { id: t.uuid() });
export const allOrders = t.message('AllOrders', {});

// screen and overlay params
export const ordersParams = t.message('OrdersParams', { status: orderStatus.optional() });
export const orderParams = t.message('OrderParams', { id: t.uuid() });
export const cancelParams = t.message('CancelParams', { id: t.uuid() });
export const reportParams = t.message('ReportParams', { product: t.text().optional(), range: t.text().optional() });
export const activityParams = t.message('ActivityParams', { order: t.text().optional(), job: t.text().optional() });
export const assignParams = t.message('AssignParams', { order: t.text().optional() });

// --- topics, screens --------------------------------------------------------------------------------

export const ORDERS = topic('orders', allOrders);
export const ORDER = topic('order', orderKey);
export const ORDERS_SCREEN = ui.screenKey('orders', ordersParams);
export const ORDER_SCREEN = ui.screenKey('order', orderParams);
export const REPORT_SCREEN = ui.screenKey('report', reportParams);
export const CANCEL = ui.overlayKey('cancel_form', cancelParams);
export const ACTIVITY_SCREEN = ui.screenKey('activity', activityParams);
export const ASSIGN = ui.overlayKey('assign', assignParams);

// --- endpoints ------------------------------------------------------------------------------------

const none = () => ({
  id: '00000000-0000-0000-0000-000000000000',
  number: '',
  customer: '',
  email: '',
  total: Decimal.from('0'),
  currency: '',
  status: 'placed',
  placedAt: new Date(0),
});

export const listOrders = endpoint({
  input: listOrdersRequest,
  output: listOrdersResponse,
  readOnly: true,
  handler: () => ({ orders: [], nextPageToken: '', total: 0n }),
});
export const getOrder = endpoint({ input: getOrderRequest, output: order, readOnly: true, handler: none });
export const cancelOrder = endpoint({
  input: cancelOrderRequest,
  output: order,
  roles: [clerk],
  idempotency: 'optional',
  handler: none,
});
export const countOpen = endpoint({
  input: countOpenRequest,
  output: countResponse,
  readOnly: true,
  handler: () => ({ count: 0n }),
});

export const sales = endpoint({
  input: salesRequest,
  output: salesResponse,
  readOnly: true,
  handler: () => ({ days: [], revenue: Decimal.from('0') }),
});
export const topProducts = endpoint({
  input: topProductsRequest,
  output: topProductsResponse,
  readOnly: true,
  handler: () => ({ products: [] }),
});

export const getActivity = endpoint({
  input: getActivityRequest,
  output: activityView,
  readOnly: true,
  handler: () => ({
    title: '', status: '', ends: 0n, notes: [], people: [], olderPageToken: '', pinned: 0, closed: false, placeholder: '', watching: false,
  }),
});
export const getLive = endpoint({
  input: getLiveRequest,
  output: liveView,
  readOnly: true,
  handler: () => ({ notes: [], people: [], jobs: [], label: '', hint: '', canStop: false }),
});
export const getWatchers = endpoint({ input: getWatchersRequest, output: watchersView, readOnly: true, handler: () => ({ people: [] }) });
export const previewNote = endpoint({ input: previewNoteRequest, output: previewView, readOnly: true, handler: () => ({ hint: '' }) });
export const getJob = endpoint({ input: getJobRequest, output: jobView, readOnly: true, handler: () => ({ title: '' }) });
export const addNote = endpoint({ input: addNoteRequest, output: addNoteResponse, idempotency: 'optional', handler: () => ({}) });
export const noteAction = endpoint({ input: noteActionRequest, output: done, idempotency: 'optional', handler: () => ({ ok: true }) });
export const watch = endpoint({ input: watchRequest, output: done, idempotency: 'optional', handler: () => ({ ok: true }) });
export const assign = endpoint({ input: assignRequest, output: done, idempotency: 'optional', handler: () => ({ ok: true }) });

// --- UI -------------------------------------------------------------------------------------------

export const ordersScreen = ui.screen(ORDERS_SCREEN, (s) => {
  const status = s.param(ordersParams.f.status);
  const orders = s.query(
    'orders',
    ui.call(listOrders, { status }).rows(listOrdersResponse.f.orders).paged().follow(ORDERS),
  );
  const open = s.rowAction('open', orders, (a, r) => a.navigate(ORDER_SCREEN, { id: r.get(order.f.id) }));
  const cancel = s.rowAction('cancel', orders, (a, r) =>
    a
      .confirm(
        ui.confirm
          .danger('Cancel this order?')
          .text('The customer is told.')
          .cancelLabel('Keep it')
          .summary('receipt', r.get(order.f.number)),
      )
      .call(cancelOrder, { id: r.get(order.f.id) })
      .toast(ui.text('orders.cancelled', 'Order cancelled'))
      .described(r.get(order.f.customer))
      .withButton('Open', open)
      .onError((e) => e.toastError()),
  );
  s.title(ui.text('orders.title', 'Orders'));
  return ui.stack([
    ui.filterBar([status.select('Status')]),
    ui
      .table(orders)
      .mono(order.f.number, 'Order')
      .col(order.f.customer, 'Customer')
      .withAvatar(order.f.customer)
      .secondLine(order.f.email)
      .money(order.f.total, 'Total', order.f.currency)
      .status(order.f.status, 'Status', { placed: 'attention', paid: 'ok' })
      .relative(order.f.placedAt, 'Placed')
      .open(open)
      .count(orders.field(listOrdersResponse.f.total))
      .countLabel(ui.text('orders.count', '{total} orders · newest first').arg('total', orders.field(listOrdersResponse.f.total)))
      .compact((r) =>
        r
          .view(r.get(order.f.customer))
          .subtitle(r.get(order.f.number))
          .meta(r.get(order.f.total))
          .money(order.f.currency)
          .status(order.f.status, { paid: 'ok' }),
      )
      .rowButton((r) => ui.button('Cancel', cancel).danger().visible(r.get(order.f.status).eq('placed'))),
  ]);
});

// The cancel dialog, document-level: the order screen and the palette's search action open it.
export const cancelDialog = ui.dialog(CANCEL, (o) => {
  const oid = o.param(cancelParams.f.id);
  o.title('Cancel order');
  return o.form(
    cancelOrder,
    (f) => {
      f.textarea(cancelOrderRequest.f.reason, 'Why').required();
      f.cancel('Keep it');
      f.note('The customer gets an email.');
      f.alternate('Cancel quietly', (c) => c.callForm(cancelOrder, { id: oid }).close());
    },
    'Cancel order',
    (a) => a.callForm(cancelOrder, { id: oid }).toast('Order cancelled').close(),
  );
});

export const orderScreen = ui.screen(ORDER_SCREEN, (s) => {
  const id = s.param(orderParams.f.id);
  const record = s.query('order', ui.call(getOrder, { id }).follow(ORDER.key({ id })));
  const edit = s.action('cancel', (a) => a.open(CANCEL, { id }));
  const back = s.action('back', (a) => a.back());
  const remind = s.action('remind', (a) => a.toast('Reminder sent'));
  s.title(record.field(order.f.number));
  return ui.stack([
    ui
      .recordHeader(record.field(order.f.number))
      .subtitle(record.field(order.f.customer))
      .tags([ui.status(record.field(order.f.status), { paid: 'ok' })])
      .actions([ui.button('Cancel order', edit).danger().visible(record.field(order.f.status).eq('placed'))])
      .menu([ui.menuItem('Send a reminder', remind)])
      .close(back),
    ui
      .columns([
        ui.fields([
          ui.item('Customer', record.field(order.f.customer)),
          ui.item('Placed', record.field(order.f.placedAt)).datetime(),
        ]),
        ui
          .titledCard('Payment', [ui.status(record.field(order.f.status), { paid: 'ok' })])
          .trailing(record.field(order.f.total))
          .moneyIn('USD'),
      ])
      .ratios([2, 1]),
  ]);
});

// Charts and figures on a board: a line over dates, a ranked bar that selects, a metric, a pivot and several series;
// over it the range switch, bound to a param with a default.
export const reportScreen = ui.screen(REPORT_SCREEN, (s) => {
  const picked = s.param(reportParams.f.product);
  const range = s.paramOr(reportParams.f.range, '30');
  const channel = s.state('channel', t.text(), '');
  const byDay = s.query('sales', ui.call(sales, { channel, range }).rows(salesResponse.f.days));
  const top = s.query('top', ui.call(topProducts, {}).rows(topProductsResponse.f.products));
  s.title('Report');
  const charts = ui
    .board()
    .span(
      8,
      ui.viz.line
        .line()
        .title(ui.text('report.revenue', 'Revenue'))
        .range('Last 30 days')
        .options({ fmt: 'money', curve: 'smooth' })
        .dates(byDay, daySales.f.day)
        .values(byDay, daySales.f.total)
        .height(280)
        .open(ORDERS_SCREEN, { status: 'paid' })
        .fallback(ui.table(byDay).date(daySales.f.day, 'Day').col(daySales.f.total, 'Revenue')),
    )
    .third(
      ui.viz.bar
        .bar()
        .title('Top products')
        .rows(top, { name: product.f.name, value: product.f.revenue, units: product.f.units })
        .textKeys(['name'])
        .option('fmt', 'money')
        .option('limit', 5)
        .option('field', 'product')
        .selectParam(picked),
    )
    .quarter(
      ui.viz.figure
        .metric()
        .title('Revenue')
        .value('value', byDay.field(salesResponse.f.revenue))
        .value('target', 120000)
        .value('caption', 'this month')
        .value('filter.channel', channel),
    )
    .span(
      9,
      ui.viz.bar
        .column()
        .title('By channel')
        .pivot(byDay, daySales.f.day, daySales.f.channel, daySales.f.total)
        .asDate()
        .option('stacked', true)
        .selectState(channel),
    )
    .full(ui.heading('Channels'))
    .full(
      ui
        .viz('line', 'multiples')
        .labels(byDay, daySales.f.channel)
        .series(byDay, [
          [daySales.f.online, 'Online'],
          [daySales.f.retail, ui.text('report.retail', 'Retail')],
        ])
        .serie(byDay, daySales.f.total, 'Total')
        .column('notes', byDay, daySales.f.channel)
        .asText()
        .rows(top, { name: product.f.name })
        .to('parts')
        .visible(picked.val().empty()),
    )
    .fallback(ui.table(top).col(product.f.name, 'Product'));
  return ui.stack([
    ui.segmented(range, [
      ['7', ui.text('report.week', '7 days')],
      ['30', ui.text('report.month', '30 days')],
    ]),
    charts,
  ]);
});

// The activity of an order (catalog 3): notes and events read from the bottom, a packing job's work on a note, the live
// jobs, a box to write a note, and a side panel of watchers.
export const activityScreen = ui.screen(ACTIVITY_SCREEN, (s) => {
  const order = s.param(activityParams.f.order);
  const jobParam = s.param(activityParams.f.job);
  const tab = s.stateReset('tab', t.text(), 'notes', [order]);
  const wide = s.stateReset('wide', t.bool(), false, [order]);
  const draft = s.state('draft', t.text(), '');
  const who = ui.via('who', note, person);
  const notes = s.query(
    'notes',
    ui
      .call(getActivity, { order })
      .rows(activityView.f.notes)
      .older(activityView.f.olderPageToken, getActivityRequest.f.pageToken)
      .join(who, activityView.f.people, person.f.id, note.f.by)
      .follow(ORDERS),
  );
  const live = s.query(
    'live',
    ui.call(getLive, { order }).rows(liveView.f.notes).join(who, liveView.f.people, person.f.id, note.f.by),
  );
  const people = s.query('people', ui.call(getWatchers, { order }).rows(watchersView.f.people));
  const hint = s.query(
    'hint',
    ui.call(previewNote, { order, text: draft }).debounceMs(300).when(ui.not(draft.val().empty())),
  );
  const jobQ = s.query('job', ui.call(getJob, { id: jobParam }).when(ui.not(jobParam.val().empty())));
  const add = s.action('add', (a) =>
    a
      .call(addNote, { order, text: ui.form.text(), reply: ui.form.reply(), urgent: ui.form.option('urgent') })
      .merge(notes, ui.result(addNoteResponse.f.note)),
  );
  const onNote = (name, extra = {}) =>
    s.rowAction(name, notes, (a, r) => a.call(noteAction, { note: r.get(note.f.id), action: name, ...extra }));
  const seen = onNote('seen');
  const react = onNote('react', { emoji: ui.form.emoji() });
  const edit = onNote('edit', { text: ui.form.text() });
  const retry = onNote('retry');
  const openJob = s.rowAction('open_job', notes, (a, r) => a.setParam(jobParam, r.get(note.f.jobId)));
  const closeJob = s.action('close_job', (a) => a.setParam(jobParam, ui.none()));
  const quote = s.rowAction('quote', notes, (a, r) =>
    a.draft(ui.text('activity.quote', "About {who}'s note: ").arg('who', r.at(who.f(person.f.name)))),
  );
  const answer = s.rowAction('answer', notes, (a, r) => a.replyTo(r.at(who.f(person.f.name)), r.get(note.f.id)));
  const copyLink = s.action('copy_link', (a) => a.copyLink(order.val()));
  const copyTitle = s.action('copy_title', (a) => a.copy(notes.field(activityView.f.title)));
  const toggleWatch = s.action('watch', (a) => a.call(watch, { order, watch: ui.form.pressed() }));
  const mention = s.action('mention', (a) => a.toast(ui.text('activity.mention', 'Write to {to}').arg('to', ui.form.to())));
  const stop = s.action('stop', (a) => a.toast('Stopping'));
  const stopJob = s.action('stop_job', (a) => a.toast('Stopping the job'));
  const jump = s.action('jump', (a) => a.scrollTo(live.field(liveView.f.label)));
  const assignAction = s.action('assign', (a) => a.open(ASSIGN, { order }));
  s.title(notes.field(activityView.f.title));
  const notePost = ui
    .post(note)
    .author(who.f(person.f.name))
    .time(note.f.time)
    .avatar(who.f(person.f.avatar))
    .tile(who.f(person.f.icon), who.f(person.f.engine))
    .badge('AGENT', ui.rowAt(who.f(person.f.agent)).eq(true))
    .markdown(note.f.text)
    .edited(note.f.edited)
    .pinned(note.f.pinned, true)
    .mine(note.f.mine)
    .receipt(note.f.receipt, note.f.receiptTone, { done: 'ok' })
    .reactions(note.f.reactions, reaction.f.emoji, reaction.f.count, react)
    .reactionsMine(reaction.f.mine)
    .quick(['👍', '✅'])
    .summary(ui.path(note.f.replies, replies.f.count), openJob)
    .summaryLabel(
      ui
        .text('activity.replies', '{n, plural, one {# reply} other {# replies}}')
        .arg('n', ui.rowAt(ui.path(note.f.replies, replies.f.count))),
    )
    .process(note.f.job, (w) =>
      w
        .state(job.f.state)
        .label(job.f.label)
        .meta(job.f.meta)
        .started(job.f.started)
        .rows(job.f.rows)
        .steps(job.f.steps)
        .retry(retry, job.f.retryLabel, ui.rowValue(note.f.mine).eq(true))
        .log(openJob)
        .fetchCall(jobQ, getJobRequest.f.call, jobView.f.call, { id: ui.rowValue(note.f.jobId) }),
    )
    .hover([ui.builtin.react(), ui.builtin.reply(), ui.postEntry(quote, 'Quote').textStyle(), ui.builtin.more()])
    .menu([
      ui.builtin.copyText(),
      ui.builtin.copyLink(),
      ui.builtin.edit(),
      ui.builtin.separator(),
      ui.postEntry(answer, 'Answer').icon('reply').danger(),
    ])
    .sheet([ui.builtin.reply(), ui.builtin.copyText()])
    .edit(edit, ui.rowValue(note.f.mine).eq(true))
    .actionsWhen(notes.field(activityView.f.closed).ne(true))
    .card(
      ui
        .cardOf(people, person.f.name)
        .avatar(person.f.avatar)
        .agent(person.f.agent)
        .tile(person.f.icon, person.f.engine)
        .detail(person.f.role)
        .action(mention)
        .unknown('Not a watcher'),
    );
  const column = ui.stack([
    ui
      .header(notes.field(activityView.f.title))
      .titleIcon('package')
      .leadingTile('package', '')
      .leadingSize(40)
      .subtitle([
        ui.seg.text(notes.field(activityView.f.status)),
        ui.seg.dotLive().visible(live.field(liveView.f.canStop).eq(true)),
        ui.seg.countdown(notes.field(activityView.f.ends)).prefix('closes in '),
      ])
      .subtitleAction(jump)
      .faces(notes.field(activityView.f.people), 'Watchers', assignAction)
      .button('Assign', assignAction)
      .buttonWhen(notes.field(activityView.f.closed).ne(true))
      .menu([ui.menuItem('Copy link', copyLink), ui.menuItem('Copy title', copyTitle)])
      .tabs(tab, [ui.headerTab('notes', 'Notes'), ui.headerTab('pinned', 'Pinned').badge(notes.field(activityView.f.pinned))])
      .phoneSub(notes.field(activityView.f.status))
      .phoneIcons([assignAction])
      .menuTabs(),
    ui
      .stream(notes)
      .key(note.f.id)
      .postWith(notePost)
      .kindsLine(note.f.kind, 'event', who.f(person.f.name), note.f.text, note.f.time)
      .tail(live)
      .days(note.f.time)
      .jumpWorking(live.field(liveView.f.label))
      .empty('No notes yet.')
      .phoneIntro(notes.field(activityView.f.status))
      .onSeen(seen)
      .onSeenWhen(ui.rowValue(note.f.mine).ne(true))
      .id('notes')
      .label('Notes')
      .visible(tab.val().eq('notes')),
    ui
      .stream(notes)
      .key(note.f.id)
      .anchorTop()
      .olderButton('Show more pinned notes')
      .postWith(notePost.clone().noSummary())
      .empty('Nothing pinned.')
      .visible(tab.val().eq('pinned')),
    ui
      .composer(add)
      .forStream('notes')
      .primary()
      .placeholder(notes.field(activityView.f.placeholder))
      .tools(['emoji'])
      .mentions(
        ui
          .mentions(people, person.f.name)
          .avatar(person.f.avatar)
          .agent(person.f.agent)
          .tile(person.f.icon, person.f.engine)
          .detail(person.f.role)
          .note('A mention tells them'),
      )
      .tags()
      .dictation()
      .vocabulary('DPD')
      .reply()
      .option('urgent', 'Mark urgent')
      .optionWhen(notes.field(activityView.f.closed).ne(true))
      .captionRoute(hint.field(previewView.f.hint))
      .captionDot(live.field(liveView.f.hint))
      .stop(stop, live.field(liveView.f.canStop).eq(true))
      .stopLabel('Stop')
      .readOnly(notes.field(activityView.f.closed).eq(true), 'This order is closed.')
      .draftState(draft),
  ]);
  const panel = ui
    .stack([
      ui
        .header('Watchers')
        .context(notes.field(activityView.f.title))
        .icons([ui.iconButton('eye', 'Watching', toggleWatch).toggle(notes.field(activityView.f.watching), 'eye-off', 'Watch')])
        .expand()
        .close(closeJob)
        .back('Activity'),
      ui
        .strip(live, liveView.f.jobs, packing.f.job, packing.f.agent)
        .label(live.field(liveView.f.label))
        .tile(packing.f.icon, packing.f.engine)
        .state(packing.f.state, ['working', 'starting'], ['waiting'], ['starting'])
        .since(packing.f.since)
        .until(packing.f.until)
        .subLine(packing.f.sub)
        .countdownText('ends in {left}')
        .open(openJob)
        .jump(packing.f.note, 'Jump to the note', 'Jump to its last note')
        .button(ui.stripButton('Stop', stopJob).icon('square').stopStyle().visible(ui.rowValue(packing.f.state).eq('working')))
        .ariaLabel('Live jobs'),
      ui.process(jobQ, (w) => w.state(jobView.f.title).label(jobView.f.title)),
      ui
        .list(people, person.f.name)
        .subtitle(person.f.role)
        .search('Find a watcher')
        .online(person.f.online)
        .limit(5)
        .groupLabel('Watchers'),
      ui.text(live.field(liveView.f.label)).elapsed(),
    ])
    .visible(ui.not(jobParam.val().empty()));
  return ui.split(column, panel).fill().resize(340, 820, 420, 400).expand(wide);
});

// Assign people to an order: a pick over the watchers, a note while an agent is chosen.
export const assignDialog = ui.dialog(ASSIGN, (o) => {
  const oid = o.param(assignParams.f.order);
  o.title('Assign');
  const directory = o.query('directory', ui.call(getWatchers, { order: oid }).rows(watchersView.f.people));
  return o.form(
    assign,
    (f) => {
      f.pick(assignRequest.f.people, 'People', directory, person.f.name, person.f.name)
        .avatar(person.f.avatar)
        .agent(person.f.agent)
        .tile(person.f.icon, person.f.engine)
        .detail(person.f.role)
        .exclude(ui.lit([]))
        .empty('Nobody matches', 'Everyone is assigned')
        .placeholder('Find people')
        .required();
      f.noteWhen('An agent works on the order at once.', ui.pickedAgents(assignRequest.f.people).gt(0n));
    },
    ui.text('assign.submit', 'Assign {first}').arg('first', ui.pickedFirst(assignRequest.f.people)),
    (a) => a.callForm(assign, { order: oid }).close(),
  );
});

export default plugin({
  id: 'shop',
  version: '1.0.0',
  roles: { clerk },
  agents: {
    helper: {
      roles: [clerk],
      title: 'Order helper',
      description: 'Answers questions about orders.',
      icon: 'package',
      engine: 'codex',
      model: 'gpt-5.5',
      session: true,
      timeout: '15m',
      skills: ['order-notes'],
      topic: ORDERS,
      delegates: ['packer'],
      connection: 'models',
      sshKey: true,
    },
    packer: { description: 'Says how an order is packed.', admin: true },
  },
  connections: {
    models: { kind: 'llm', title: 'Order models', description: 'The model provider the order helper runs on.' },
    speech: { kind: 'stt' },
  },
  endpoints: {
    listOrders: listOrders.ui(),
    getOrder: getOrder.ui(),
    cancelOrder: cancelOrder.ui().invalidates(ORDERS, ORDER.by(orderKey.f.id, order.f.id)),
    countOpen: countOpen.ui(),
    sales: sales.ui(),
    topProducts: topProducts.ui(),
    getActivity: getActivity.ui(),
    getLive: getLive.ui(),
    getWatchers: getWatchers.ui(),
    previewNote: previewNote.ui(),
    getJob: getJob.ui(),
    addNote: addNote.ui().invalidates(ORDERS),
    noteAction: noteAction.ui(),
    watch: watch.ui(),
    assign: assign.ui(),
  },
  topics: [ORDERS.roles([clerk]), ORDER.authorize(getOrder)],
  ui: ui.define({
    nav: [
      ui.nav('Orders', 'shopping-bag', [
        ui.entry(ORDERS_SCREEN, ui.text('nav.all', 'All orders')).badge(
          ui.call(countOpen, {}).follow(ORDERS),
          countResponse.f.count,
        ).alert(),
      ]),
    ],
    landing: ORDERS_SCREEN,
    commands: [
      ui.command.navigate('find', 'Find an order', ORDERS_SCREEN).icon('search'),
      ui.command.navigate('week', 'Sales: last 7 days', REPORT_SCREEN, { range: '7' }).icon('bar-chart'),
    ],
    screens: [ordersScreen, orderScreen, reportScreen, activityScreen],
    home: [
      ui
        .home('sales', 'Sales', (w) => {
          const days = w.query('days', ui.call(sales, {}).rows(salesResponse.f.days));
          return ui.viz.line
            .line()
            .dates(days, daySales.f.day)
            .values(days, daySales.f.total)
            .option('compact', true)
            .open(REPORT_SCREEN)
            .fallback(ui.metric('Revenue', days.field(salesResponse.f.revenue)));
        }),
    ],
    overlays: [cancelDialog, assignDialog],
    search: [
      ui
        .search('Orders', listOrders, listOrdersRequest.f.text, listOrdersResponse.f.orders)
        .item((r) => r.view(r.get(order.f.number)).subtitle(r.get(order.f.customer)))
        .open(ORDER_SCREEN, (r) => ({ id: r.get(order.f.id) }))
        .action('Cancel order', 'x-circle', CANCEL, (r) => ({ id: r.get(order.f.id) })),
    ],
  }),
});
