// Static types of UI declarations: field handles come from the message definitions, so a wrong field name, a field
// of another message or a value of the wrong type does not compile. Compiled (not run) by `npm test`; each
// `@ts-expect-error` must be an error.
import { endpoint, t, topic, ui } from '@celobus/sdk';

const status = t.enum('OrderStatus', ['placed', 'paid']);
const order = t.message('Order', { id: t.uuid(), number: t.text(), total: t.decimal(), currency: t.text(), status });
const customer = t.message('Customer', { id: t.uuid(), name: t.text() });
const listRequest = t.message('ListOrdersRequest', { status: status.optional(), pageToken: t.text() });
const listResponse = t.message('ListOrdersResponse', { orders: t.array(order), nextPageToken: t.text() });
const cancelRequest = t.message('CancelOrderRequest', { id: t.uuid(), reason: t.text(), notify: t.bool() });
const orderKey = t.message('OrderKey', { id: t.uuid() });
const params = t.message('OrdersParams', { status: status.optional() });

const listOrders = endpoint({ input: listRequest, output: listResponse, readOnly: true, handler: () => ({ orders: [], nextPageToken: '' }) });
const cancelOrder = endpoint({ input: cancelRequest, output: order, handler: () => ({ id: '', number: '', total: undefined!, currency: '', status: 'paid' as const }) });
const ORDER = topic('order', orderKey);
const ORDERS_SCREEN = ui.screenKey('orders', params);

// @ts-expect-error no such field
void order.f.nope;

// @ts-expect-error the key's field must be a field of the key message
ORDER.by(order.f.id, order.f.id);
// @ts-expect-error a key field and a response field of different types
ORDER.by(orderKey.f.id, order.f.total);
cancelOrder.invalidates(ORDER.by(orderKey.f.id, order.f.id));

ui.screen(ORDERS_SCREEN, (s) => {
  const st = s.param(params.f.status);
  // @ts-expect-error a param must be a field of the screen's params message
  s.param(order.f.status);

  // @ts-expect-error not a field of the request
  ui.call(listOrders, { stauts: st });
  // @ts-expect-error a string param where the request takes the enum
  ui.call(listOrders, { status: ui.lit('x' as string) });
  // @ts-expect-error rows must be a repeated field of the response
  ui.call(listOrders, {}).rows(listResponse.f.nextPageToken);
  // @ts-expect-error rows must be a field of the response
  ui.call(listOrders, {}).rows(order.f.id);

  const orders = s.query('orders', ui.call(listOrders, { status: st }).rows(listResponse.f.orders).paged());
  const one = s.query('one', ui.call(cancelOrder, {}));
  one.field(order.f.number);
  // @ts-expect-error a field of another message
  one.field(customer.f.name);

  const cancel = s.rowAction('cancel', orders, (a, r) =>
    a
      .call(cancelOrder, { id: r.get(order.f.id), notify: true })
      // @ts-expect-error a field of a row of another message
      .call(cancelOrder, { id: r.get(customer.f.id) })
      // @ts-expect-error a string where the request takes a bool
      .call(cancelOrder, { notify: 'yes' })
      .navigate(ORDERS_SCREEN, { status: 'paid' })
      // @ts-expect-error not a value of the enum
      .navigate(ORDERS_SCREEN, { status: 'gone' }),
  );

  s.form(
    cancelOrder,
    (f) => {
      f.textarea(cancelRequest.f.reason, 'Why');
      f.checkbox(cancelRequest.f.notify, 'Tell the customer');
      // @ts-expect-error a form input must be a field of the endpoint's request
      f.text(order.f.number, 'Number');
      // @ts-expect-error a checkbox needs a bool field
      f.checkbox(cancelRequest.f.reason, 'Reason');
    },
    'Cancel',
    (a, v) => a.callForm(cancelOrder, { reason: v.get(cancelRequest.f.reason) }),
  );

  const view = s.state('view', t.enum(t.enum('View', ['list', 'board'])), 'list');
  ui.segmented(view, [['list', 'List'], ['board', 'Board']]);
  // @ts-expect-error not a value of the state's type
  s.action('x', (a) => a.set(view, 'grid'));

  // a segmented over a param takes values of the param's type, optional or not
  ui.segmented(st, [['placed', 'Placed'], ['paid', 'Paid']]);
  // @ts-expect-error not a value of the param's enum
  ui.segmented(st, [['refunded', 'Refunded']]);
  // @ts-expect-error an option is a value, never an absent one
  ui.segmented(st, [[undefined, 'Any']]);
  // @ts-expect-error not a value of the state's type
  ui.segmented(view, [['grid', 'Grid']]);
  // a param with a default always has a value: its handle is of the plain type
  const paid = s.paramOr(params.f.status, 'paid');
  s.action('y', (a) => a.setParam(paid, 'placed').navigate(ORDERS_SCREEN, { status: paid }));
  // @ts-expect-error the default is a value of the param's type
  s.paramOr(params.f.status, 'refunded');
  // @ts-expect-error a param with a default is a field of the screen's params message
  s.paramOr(order.f.status, 'paid');
  // a command's args are the params of its screen
  ui.command.navigate('paid', 'Paid orders', ORDERS_SCREEN, { status: 'paid' });
  // @ts-expect-error not a param of the screen the command opens
  ui.command.navigate('paid', 'Paid orders', ORDERS_SCREEN, { state: 'paid' });
  // @ts-expect-error not a value of the param's enum
  ui.command.navigate('paid', 'Paid orders', ORDERS_SCREEN, { status: 'refunded' });

  return ui.stack([
    ui.filterBar([st.select()]),
    ui
      .table(orders)
      .col(order.f.number, 'Order')
      .money(order.f.total, 'Total', order.f.currency)
      .status(order.f.status, 'Status', { placed: 'attention', paid: 'ok' })
      // @ts-expect-error a column of another message
      .col(customer.f.name, 'Customer')
      // @ts-expect-error not a value of the enum
      .status(order.f.status, 'Status', { shipped: 'ok' })
      // @ts-expect-error not a tone
      .status(order.f.status, 'Status', { paid: 'green' })
      .rowButton((r) => ui.button('Cancel', cancel).danger().visible(r.get(order.f.status).eq('placed')))
      // @ts-expect-error not a value of the enum
      .rowButton((r) => ui.button('Cancel', cancel).visible(r.get(order.f.status).eq('shipped'))),
    ui
      .table(orders)
      .col(order.f.number, 'Order')
      .secondLine(order.f.currency)
      // @ts-expect-error a second line of another message
      .secondLine(customer.f.name)
      .compact((r) => r.view(r.get(order.f.number)).money(order.f.currency))
      // @ts-expect-error a compact row's currency of another message
      .compact((r) => r.view(r.get(order.f.number)).money(customer.f.name)),
    ui.list(orders, order.f.number),
    // @ts-expect-error a list title of another message
    ui.list(orders, customer.f.name),
    ui.viz.bar
      .column()
      .labels(orders, order.f.number)
      .values(orders, order.f.total)
      .series(orders, [[order.f.total, 'Total']])
      .rows(orders, { name: order.f.number, value: order.f.total })
      .value('value', one.field(order.f.total))
      .selectState(view)
      .selectParam(st)
      .open(ORDERS_SCREEN, { status: 'paid' })
      // @ts-expect-error a column of another message
      .values(orders, customer.f.name)
      // @ts-expect-error a series of another message
      .series(orders, [[customer.f.name, 'Name']])
      // @ts-expect-error a row key of another message
      .rows(orders, { name: customer.f.name })
      // @ts-expect-error not a param of the screen the link leads to
      .open(ORDERS_SCREEN, { state: 'paid' }),
    ui.viz.meter.meterList(),
    ui.board().quarter(ui.viz.figure.metric()).span(8, ui.table(orders)).full(ui.heading('All')),
    // @ts-expect-error an item is a node
    ui.board().half('Revenue'),
    ui.viz('figure', 'metric'),
    // a pair the catalogue does not have is still a node (`plugin check` names it); a constructor is not
    ui.viz('figure', 'column'),
    // @ts-expect-error no such kind in the family
    ui.viz.figure.column(),
    // @ts-expect-error no such family
    ui.viz.pie.pie(),
    ui.fields([ui.item('Total', one.field(order.f.total)).moneyIn('USD')]),
    ui.text(ui.row(order.f.number)),
  ]);
});

// ctx.invalidate takes the topic's key message
const ALL_ORDERS = topic('orders', t.message('AllOrders', {}));
endpoint({
  input: cancelRequest,
  output: order,
  handler: (ctx, { id }) => {
    ctx.invalidate(ORDER, { id });
    ctx.invalidateAll(ALL_ORDERS);
    // @ts-expect-error not a field of the key message
    ctx.invalidate(ORDER, { number: '1' });
    // @ts-expect-error the key's field has the wrong type
    ctx.invalidate(ORDER, { id: 1 });
    return { id, number: '', total: undefined!, currency: '', status: 'paid' as const };
  },
});

// catalog 3: paths, joins, post templates and steps are typed by the row message
const face = t.message('Face', { id: t.text(), name: t.text(), agent: t.bool() });
const summary = t.message('Summary', { count: t.int() });
const msg = t.message('Msg', { id: t.text(), by: t.text(), text: t.text(), thread: summary.optional(), faces: t.array(face), mine: t.bool() });
const channelView = t.message('ChannelView', { messages: t.array(msg), people: t.array(face), older: t.text() });
const channelRequest = t.message('ChannelRequest', { id: t.text(), pageToken: t.text() });
const getChannel = endpoint({ input: channelRequest, output: channelView, readOnly: true, handler: () => ({ messages: [], people: [], older: '' }) });
const posted = t.message('Posted', { message: msg.optional(), order: order.optional() });
const who = ui.via('who', msg, face);
ui.path(msg.f.thread, summary.f.count);
ui.path(msg.f.faces, face.f.name);
// @ts-expect-error a field of another message than the one the path leads to
ui.path(msg.f.thread, face.f.name);
// @ts-expect-error a path continues only into a message
ui.path(msg.f.text, face.f.name);
// @ts-expect-error a field of another message than the joined one
who.f(summary.f.count);

ui.screen('channel', (s) => {
  const msgs = s.query(
    'channel',
    ui.call(getChannel, { id: 'c' })
      .rows(channelView.f.messages)
      .older(channelView.f.older, channelRequest.f.pageToken)
      .join(who, channelView.f.people, face.f.id, msg.f.by),
  );
  // @ts-expect-error the key and the row's field have different types
  ui.call(getChannel, {}).rows(channelView.f.messages).join(who, channelView.f.people, face.f.agent, msg.f.by);
  // @ts-expect-error the older token is a field of the response
  ui.call(getChannel, {}).older(channelRequest.f.pageToken, channelRequest.f.pageToken);
  const thread = s.rowAction('thread', msgs, (a, r) => a.replyTo(r.at(who.f(face.f.name)), r.get(msg.f.id)));
  s.action('merge', (a) => a.merge(msgs, ui.result(posted.f.message)));
  // @ts-expect-error a row of another message
  s.action('merge2', (a) => a.merge(msgs, ui.result(posted.f.order)));
  return ui.stream(msgs).key(msg.f.id).post((p) =>
    p
      .author(who.f(face.f.name))
      .markdown(msg.f.text)
      .mine(msg.f.mine)
      .summary(ui.path(msg.f.thread, summary.f.count), thread)
      .process(msg.f.thread, (w) => w.state(summary.f.count))
      // @ts-expect-error a template's field of another message
      .time(order.f.number)
      // @ts-expect-error `mine` is a bool field
      .mine(msg.f.text),
  );
});
