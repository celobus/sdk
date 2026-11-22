// The SDK conformance plugin `notify`, TypeScript: the notification samples as one small
// plugin, the same as the Rust and C# versions. A reply notifies who it mentions, makes its author follow the thread
// and tells the thread's other followers; a member follows a thread or stops; an agent's approval is asked with two
// buttons and resolved by the one pressed. `--describe` must print sdk/conformance/notify/descriptor.json.
import { endpoint, entity, notification, notify, plugin, role, t, ui, type Ctx, type EntityDef } from '@celobus/sdk';
import * as platform from '@celobus/sdk/platform';

export const member = role({ title: 'Member' });

export const mention = notification('mention', {
  title: 'Mentions', description: 'Someone wrote @you in a channel or a thread', digest: true,
});
export const threadReply = notification('thread_reply', {
  title: 'Replies in threads you follow', description: 'Arrives in Watching', tab: 'watching',
});

// --- data -----------------------------------------------------------------------------------------

export const channel = entity({
  columns: { name: t.text() },
  access: (a) => {
    a.read(member);
  },
});

export const post = entity({
  columns: {
    channel: t.ref(() => channel),
    thread: t.ref((): EntityDef<any> => post).optional(), // absent: the post starts a thread
    text: t.text(),
  },
  access: (a) => {
    a.read(member);
    a.create(member);
  },
});

// An agent's request for a person's approval; the ask in their inbox is keyed by it.
export const approval = entity({
  columns: {
    owner: t.ref(platform.user),
    order: t.text(),
    summary: t.text(),
    status: t.text(), // pending, approved or declined
  },
  access: (a) => {
    const owner = a.relation('owner', a.column('owner'));
    a.read(owner);
    a.create(member);
    a.update(owner);
  },
});

export const channelParams = t.message('ChannelParams', { c: t.text(), thread: t.text().optional() });
export const orderParams = t.message('OrderParams', { id: t.text() });

export const postReplyRequest = t.message('PostReplyRequest', {
  channel: t.ref(() => channel),
  thread: t.ref(() => post),
  text: t.text(),
  mentioned: t.array(t.ref(platform.user)), // the members the text mentions
});
export const postView = t.message('PostView', { id: t.ref(() => post), text: t.text() });
export const followThreadRequest = t.message('FollowThreadRequest', {
  thread: t.ref(() => post),
  follow: t.bool(), // false: stop following
});
export const followState = t.message('FollowState', { following: t.bool() });
export const requestApprovalRequest = t.message('RequestApprovalRequest', {
  owner: t.ref(platform.user),
  order: t.text(),
  summary: t.text(),
});
export const approveRequest = t.message('ApproveRequest', { id: t.ref(() => approval) });
export const declineRequest = t.message('DeclineRequest', { id: t.ref(() => approval) });

// --- UI: what the notifications open --------------------------------------------------------------

export const CHANNEL = ui.screenKey('channel', channelParams);
export const ORDER_PANEL = ui.overlayKey('order_panel', orderParams);

export const channelScreen = ui.screen(CHANNEL, (s) => {
  s.title('Channel');
  return ui.text('The channel and its threads.');
});

export const orderPanel = ui.sidePanel(ORDER_PANEL, (o) => {
  o.title('Order');
  return ui.text('The order and its changes.');
});

// --- endpoints ------------------------------------------------------------------------------------

const snippet = (text: string) => [...text].slice(0, 140).join('');

/**
 * A reply in a thread: the members it mentions are told, its author follows the thread (if their setting says so),
 * and the thread's other followers are told, all in the transaction that writes the post.
 */
export const postReply = endpoint({
  description: 'Posts a reply in a thread.',
  input: postReplyRequest,
  output: postView,
  roles: [member],
  handler: (ctx, req) => ctx.transaction(async (tx) => {
    const ch = await tx.db(channel).get(req.channel);
    const created = await tx.db(post).create({ channel: req.channel, thread: req.thread, text: req.text });
    const thread = notify.row(post, req.thread); // key "post:<id>", subject post/<id>
    const link = notify.screen(CHANNEL, { c: req.channel, thread: req.thread });
    const source = { label: `#${ch.name}`, icon: 'hash' };
    await tx.notifications.send(mention, { to: req.mentioned, key: thread, title: `mentioned you in #${ch.name}`,
      body: snippet(req.text), source, link });
    await tx.notifications.follow(thread, { reason: 'replied', noun: 'thread' });
    await tx.notifications.send(threadReply, { followers: thread, except: req.mentioned,
      title: `replied in #${ch.name}`, body: snippet(req.text), source, link });
    return { id: created.id, text: created.text };
  }),
});

/** The caller follows a thread (or stops), and answers whether they follow it now. */
export const followThread = endpoint({
  description: 'Follows a thread, or stops.',
  input: followThreadRequest,
  output: followState,
  roles: [member],
  ui: true,
  async handler(ctx, req) {
    const thread = notify.row(post, req.thread);
    if (req.follow) await ctx.notifications.follow(thread, { noun: 'thread' });
    else await ctx.notifications.unfollow(thread);
    return { following: await ctx.notifications.following(thread) };
  },
});

/** Sets the approval's status and resolves its ask, for every member it asked. */
function decide(status: string, outcome: string) {
  return (ctx: Ctx, req: { id: string }) =>
    ctx.transaction(async (tx) => {
      const a = await tx.db(approval).update(req.id, { status });
      await tx.notifications.resolve(notify.row(approval, req.id), `${outcome} by ${tx.caller.displayName}`);
      return a;
    });
}

/** The ask's primary button. */
export const approve = endpoint({
  description: 'Approves.',
  input: approveRequest,
  output: approval,
  ui: true,
  handler: decide('approved', 'Approved'),
});

export const decline = endpoint({
  description: 'Declines.',
  input: declineRequest,
  output: approval,
  ui: true,
  handler: decide('declined', 'Declined'),
});

/**
 * An agent's approval: the Approval row and the ask in one transaction; the key is the ask's identity and the caller
 * (the agent's account) its actor.
 */
export const requestApproval = endpoint({
  description: "Asks a member to approve an order's changes.",
  input: requestApprovalRequest,
  output: approval,
  roles: [member],
  handler: (ctx, req) => ctx.transaction(async (tx) => {
    const a = await tx.db(approval).create({ owner: req.owner, order: req.order, summary: req.summary, status: 'pending' });
    await tx.notifications.ask({
      kind: 'approval', key: notify.row(approval, a.id), to: [req.owner],
      title: 'needs your approval', body: req.summary, source: { label: `#${req.order}`, icon: 'receipt' },
      link: notify.overlay(ORDER_PANEL, { id: req.order }), review: 'Review the changes',
      buttons: [
        notify.button('Approve', approve, { id: a.id }, { primary: true, icon: 'check' }),
        notify.button('Decline', decline, { id: a.id }, { icon: 'x' }),
      ],
    });
    return a;
  }),
});

export default plugin({
  id: 'notify', // the version is package.json's
  title: 'Notify',
  description: 'The SDK conformance plugin of notifications.',
  roles: { member },
  capabilities: { 'push:send': 'Tells people when they are mentioned or written to.' },
  notifications: [mention, threadReply],
  entities: { channel, post, approval },
  endpoints: { postReply, followThread, requestApproval, approve, decline },
  ui: ui.define({ screens: [channelScreen], overlays: [orderPanel] }),
});
