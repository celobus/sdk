// Notifications: the kinds a plugin declares, and what it sends.
//
// A plugin only sends: the platform stores each notification in the member's inbox, tracks read and done, keeps
// follows and preferences, hides what the member can no longer read, keeps their screens live and mails a daily
// digest of what stays unread. Sending needs the capability `push:send`, declared with its reason.
//
//   export const mention = notification('mention', { title: 'Mentions', description: 'Someone wrote @you', digest: true });
//   plugin({ ..., capabilities: { 'push:send': 'Tells people when they are mentioned.' }, notifications: [mention] });
//
//   await tx.notifications.send(mention, { to: mentioned, key: notify.row(post, id), title: 'mentioned you', link });
//
// The API is `ctx.notifications` (`Notifications`): `send` a notice of a kind, `ask` an approval or a decision with
// buttons and `resolve` it, `follow` a key, `unfollow` it, ask whether the caller is `following` it.
import { encodeMessage, stringify } from "./codec.js";
import { describedEndpoints, type EndpointDef } from "./declare.js";
import { internal } from "./errors.js";
import { entityNames, type EntityDef, type IoType, type IoValue, type MessageDef } from "./schema.js";
import type { OverlayKey, ScreenKey } from "./ui.js";

/** The inbox tab a kind arrives in: addressed to the member, or about something they follow. */
export type NotificationTab = "for_you" | "watching";

export interface NotificationOptions {
  /** The settings row's title ("Mentions"), 1 to 40 characters. */
  title: string;
  /** The settings row's second line, at most 120 characters. */
  description?: string;
  /** Default `for_you`; `watching`: about something the member follows (a reply in a thread). */
  tab?: NotificationTab;
  /** The daily digest includes it unless the member turns that off (default: not). */
  digest?: boolean;
}

/**
 * A kind of notification the plugin declares (`plugin({ notifications: [mention] })`): what a member turns on or off
 * in their settings. Its name is snake_case, at most 32 characters.
 */
export class NotificationKind {
  constructor(
    readonly name: string,
    readonly options: NotificationOptions,
  ) {}
}

/** `export const mention = notification('mention', { title: 'Mentions', digest: true })`. */
export function notification(name: string, options: NotificationOptions): NotificationKind {
  return new NotificationKind(name, options);
}

// ---------------------------------------------------------------------------------------------- what is sent

function entityName(entity: EntityDef<any>): string {
  const name = entityNames.get(entity) ?? entity.options.name;
  if (!name) throw internal(new Error("notify: the entity is not in plugin({ entities })"));
  return name;
}

/**
 * What a notification is about: the plugin's name for it (`post:0193…`, at most 200 bytes), optionally one of its
 * entity rows (the subject, `<entity>/<id>`). Unread notifications of one kind and key group into one row
 * ("3 replies"); members follow keys; an ask's key is its identity. A member who can no longer read the subject no
 * longer sees it.
 */
export class NotifyKey {
  constructor(
    readonly key: string,
    readonly subject: string = "",
  ) {}

  /** The same key tied to one of the plugin's entity rows. */
  withSubject<E extends EntityDef<any>>(entity: E, id: string | bigint): NotifyKey {
    return new NotifyKey(this.key, `${entityName(entity)}/${id}`);
  }

  toString(): string {
    return this.key;
  }
}

/** Where a notification opens: one of the plugin's screens or document-level overlays, with its params. */
export class NotifyLink {
  /** @internal */
  constructor(
    readonly screen: string,
    readonly overlay: string,
    readonly args: Record<string, unknown>,
  ) {}
}

function paramsJson(message: MessageDef<any> | undefined, params: unknown): Record<string, unknown> {
  if (!message) return (params ?? {}) as Record<string, unknown>;
  try {
    return encodeMessage(message, params);
  } catch (e) {
    throw internal(new Error(`notify: the params are not ${message.name}: ${(e as Error).message}`));
  }
}

/** Where a notification happened: a label ("#fulfilment") and an icon of the kit ("hash"). */
export interface NotifySource {
  label: string;
  icon: string;
}

/**
 * A button of an ask: it calls one of the plugin's `ui` endpoints with this request, as the member who presses it.
 * The endpoint checks the member and resolves the ask. Made with `notify.button(..)`.
 */
export class NotifyButton {
  /** @internal */
  constructor(
    readonly label: string,
    readonly endpoint: EndpointDef<any, any>,
    readonly request: unknown,
    readonly primary: boolean,
    readonly icon: string,
  ) {}
}

/** The actor a notification shows: the caller (default), one of the plugin's agents, or nobody (the plugin's mark). */
interface Actor {
  /** Shown as one of the plugin's agents, by name, instead of the caller. */
  agent?: string;
  /** Shown as the plugin itself, without an actor. */
  noActor?: boolean;
}

/** A notification of one of the plugin's kinds: to members (`to`), to the followers of a key (`followers`), or both. */
export interface Notice extends Actor {
  /** User ids, at most 100. */
  to?: readonly string[];
  /** Also to the active followers of this key (its key too, unless `key` names another). */
  followers?: NotifyKey;
  /** Not to these members (those already told in this request). */
  except?: readonly string[];
  /** What it is about: unread notifications of its kind and key group into one row. */
  key?: NotifyKey;
  /** The phrase after the actor ("mentioned you in #fulfilment"), at most 140 characters. */
  title?: string;
  /** A snippet, at most 500 characters of plain text. */
  body?: string;
  source?: NotifySource;
  link?: NotifyLink;
  /** A member gets at most one notification of the plugin with this value (at most 120 bytes); never grouped. */
  once?: string;
}

/**
 * An approval or a decision a member answers in their inbox (NEEDS YOU): 1 to 3 buttons that call the plugin's own
 * endpoints, and a review link. It stays until it is resolved (`resolve`) or the member marks it done; asking again
 * refreshes the open one of its key.
 */
export interface Ask extends Actor {
  kind: "approval" | "decision";
  /** Its identity. */
  key: NotifyKey;
  /** User ids, at most 100. */
  to: readonly string[];
  title?: string;
  body?: string;
  source?: NotifySource;
  /** What the review link opens. */
  link?: NotifyLink;
  /** The review link's label ("Review 5 changes"). */
  review?: string;
  buttons: readonly NotifyButton[];
}

/**
 * How members come to follow a key. `manual` (default): the member asked to (a Follow button), and it resumes a follow
 * they stopped; `replied` / `edited`: only members whose setting ("follow threads I reply in", "follow records I
 * edit") is on and who never stopped following the key.
 */
export type FollowReason = "manual" | "replied" | "edited";

export interface Follow {
  reason?: FollowReason;
  /** What the inbox calls the key ("thread": "Stop following this thread"), at most 24 characters. */
  noun?: string;
  /** These members (user ids, at most 100) instead of the caller: with `replied` or `edited` only (a manual follow is
   * the caller's own; naming anyone else is `invalid`). */
  users?: readonly string[];
}

/** Keys, links and buttons of notifications. */
export const notify = {
  /** A key of the plugin's own (no subject). */
  key(key: string): NotifyKey {
    return new NotifyKey(key);
  },
  /** The key of one of the plugin's entity rows, `<entity>:<id>`, with the row as its subject. */
  row<E extends EntityDef<any>>(entity: E, id: string | bigint): NotifyKey {
    const name = entityName(entity);
    return new NotifyKey(`${name}:${id}`, `${name}/${id}`);
  },
  /** Opens one of the plugin's screens with these params. */
  screen<P>(screen: ScreenKey<P>, params: P): NotifyLink {
    return new NotifyLink(screen.id, "", paramsJson(screen.paramsMessage, params));
  },
  /** Opens one of the plugin's document-level overlays (a side panel, shown beside the inbox's list). */
  overlay<P>(overlay: OverlayKey<P>, params: P): NotifyLink {
    return new NotifyLink("", overlay.id, paramsJson(overlay.paramsMessage, params));
  },
  /** A button of an ask that calls `endpoint` (a `ui` endpoint of the plugin) with `request`. */
  button<I extends IoType>(
    label: string,
    endpoint: EndpointDef<I, any>,
    request: IoValue<I>,
    options: { primary?: boolean; icon?: string } = {},
  ): NotifyButton {
    return new NotifyButton(label, endpoint, request, !!options.primary, options.icon ?? "");
  },
};

/** @internal What a `Notifications` sends its syscalls through. */
export interface NotifyLinkage {
  syscall(call: Record<string, unknown>): Promise<{ notifyFollowState?: { following: boolean } | null }>;
  requestId: string;
  tx: string;
}

function actorWire(a: Actor): Record<string, unknown> {
  if (a.agent) return { agent: a.agent };
  if (a.noActor) return { none: true };
  return { caller: true };
}

/** @internal A link as the wire carries it (a notification's, an agent run's `notifyLink`). */
export function linkWire(l: NotifyLink | undefined): Record<string, unknown> | undefined {
  return l && { screen: l.screen, overlay: l.overlay, args: stringify(l.args) };
}

function buttonWire(b: NotifyButton): Record<string, unknown> {
  const described = describedEndpoints.get(b.endpoint.root);
  if (!described) throw internal(new Error(`notify: the button "${b.label}" names an endpoint that is not in plugin({ endpoints })`));
  let body: Record<string, unknown>;
  try {
    body = encodeMessage(described.input, b.request);
  } catch (e) {
    throw internal(new Error(`notify: the request of the button "${b.label}" is not ${described.name}'s: ${(e as Error).message}`));
  }
  return { label: b.label, endpoint: described.name, body: stringify(body), primary: b.primary, icon: b.icon };
}

const REASONS: Record<FollowReason, string> = { manual: "MANUAL", replied: "REPLIED", edited: "EDITED" };

/**
 * The plugin's notifications (`ctx.notifications`; `tx.notifications` writes in the transaction, and its members hear
 * of it when it commits). A notification never undoes the transaction: a call the database or a limit refuses in it
 * is let go (logged) and resolves normally. Send late in a transaction: a send holds the unread groups it grew until
 * the commit. Needs the capability `push:send` (403 `capability_missing`); a send the platform refuses is 400
 * `invalid`; sends over 50 a second are let go; a manual follow of a key with 2,000 followers, outside a
 * transaction, is 409 `follow_limit`. A send answers nothing about who received it.
 */
export class Notifications {
  /** @internal */
  constructor(private readonly link: NotifyLinkage) {}

  private sendWire(kind: string, n: Notice, ask?: Record<string, unknown>): Record<string, unknown> {
    const key = n.key ?? n.followers;
    return {
      requestId: this.link.requestId,
      tx: this.link.tx,
      kind,
      to: [...(n.to ?? [])],
      followers: n.followers !== undefined,
      except: [...(n.except ?? [])],
      key: key?.key ?? "",
      subject: key?.subject ?? "",
      actor: actorWire(n),
      title: n.title ?? "",
      body: n.body ?? "",
      ...(n.source ? { source: { label: n.source.label, icon: n.source.icon } } : {}),
      ...(n.link ? { link: linkWire(n.link) } : {}),
      ...(ask ? { ask } : {}),
      once: n.once ?? "",
    };
  }

  /** Sends a notification of one of the plugin's kinds. */
  async send(kind: NotificationKind, notice: Notice): Promise<void> {
    await this.link.syscall({ notifySend: this.sendWire(kind.name, notice) });
  }

  /** Asks members for an approval or a decision. */
  async ask(ask: Ask): Promise<void> {
    const wire = {
      kind: ask.kind === "decision" ? "DECISION" : "APPROVAL",
      buttons: ask.buttons.map(buttonWire),
      review: ask.review ?? "",
    };
    const { kind: _kind, buttons: _buttons, review: _review, ...notice } = ask;
    await this.link.syscall({ notifySend: this.sendWire("", notice, wire) });
  }

  /**
   * Resolves every open ask of `key`, for every member it asked: `outcome` ("Approved by Sarah Bennett", at most 80
   * characters) is what the inbox shows.
   */
  async resolve(key: NotifyKey, outcome: string): Promise<void> {
    await this.link.syscall({ notifyResolve: { requestId: this.link.requestId, tx: this.link.tx, key: key.key, outcome } });
  }

  private followWire(key: NotifyKey, op: string, follow: Follow = {}, reason = ""): Record<string, unknown> {
    return {
      requestId: this.link.requestId,
      tx: this.link.tx,
      key: key.key,
      subject: key.subject,
      noun: follow.noun ?? "",
      users: [...(follow.users ?? [])],
      reason: reason || "REASON_UNSPECIFIED",
      op,
    };
  }

  /** Makes the caller (or, for `replied` and `edited`, `follow.users`) follow `key`. */
  async follow(key: NotifyKey, follow: Follow = {}): Promise<void> {
    await this.link.syscall({ notifyFollow: this.followWire(key, "FOLLOW", follow, REASONS[follow.reason ?? "manual"]) });
  }

  /** The caller stops following `key`; an automatic follow does not resume it. */
  async unfollow(key: NotifyKey): Promise<void> {
    await this.link.syscall({ notifyFollow: this.followWire(key, "UNFOLLOW") });
  }

  /** Whether the caller follows `key` (for a "Following" button): the one thing a plugin may read of notifications. */
  async following(key: NotifyKey): Promise<boolean> {
    const r = await this.link.syscall({ notifyFollow: this.followWire(key, "GET") });
    if (!r.notifyFollowState) throw internal(new Error("the platform answered a follow with something else"));
    return !!r.notifyFollowState.following;
  }
}
