//! Notifications: the kinds a plugin declares, and what it sends.
//!
//! A plugin only sends: the platform stores each notification in the member's inbox, tracks read and done, keeps
//! follows and preferences, hides what the member can no longer read, keeps their screens live and mails a daily
//! digest of what stays unread. Sending needs the capability `push:send`, declared with its reason.
//!
//! ```
//! use celobus_sdk::notify::Kind;
//!
//! pub const MENTION: Kind = Kind::new("mention").title("Mentions")
//!     .describe("Someone wrote @you in a channel or a thread").digest();
//! pub const THREAD_REPLY: Kind = Kind::new("thread_reply").title("Replies in threads you follow").watching();
//! ```
//!
//! The API is `ctx.notifications()` ([`Notifications`]): `send` a [`Notice`] of a kind, `ask` an [`Ask`] (an
//! approval or a decision with [`Button`]s) and `resolve` it, `follow` a [`Key`] ([`Follow`]), `unfollow` it, ask
//! whether the caller is `following` it.

use std::future::Future;

use celobus_proto as pb;
use pb::{syscall, syscall_result};
use serde_json::Value;

use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::message::{Entity, Message};
use crate::platform::User;
use crate::types::Id;
use crate::ui::{OverlayKey, ScreenKey};

/// The inbox tab a kind arrives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    /// Addressed to the member (a mention, a direct message, an assignment).
    #[default]
    ForYou,
    /// About something the member follows (a reply in a thread).
    Watching,
}

impl Tab {
    pub fn as_str(self) -> &'static str {
        match self {
            Tab::ForYou => "for_you",
            Tab::Watching => "watching",
        }
    }
}

/// A kind of notification the plugin declares (`.notification(MENTION)`): what a member turns on or off in their
/// settings. `name` is snake_case, at most 32 characters; the title is 1 to 40 characters, the description at most
/// 120. The digest leaves it out unless `.digest()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    tab: Tab,
    digest: bool,
}

impl Kind {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            title: "",
            description: "",
            tab: Tab::ForYou,
            digest: false,
        }
    }

    /// The settings row's title ("Mentions").
    pub const fn title(mut self, title: &'static str) -> Self {
        self.title = title;
        self
    }

    /// The settings row's second line.
    pub const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Arrives in Watching (things the member follows) instead of For you.
    pub const fn watching(mut self) -> Self {
        self.tab = Tab::Watching;
        self
    }

    /// The daily digest includes it unless the member turns that off.
    pub const fn digest(mut self) -> Self {
        self.digest = true;
        self
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn get_title(&self) -> &'static str {
        self.title
    }

    pub fn description(&self) -> &'static str {
        self.description
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn in_digest(&self) -> bool {
        self.digest
    }
}

// ---------------------------------------------------------------------------------------------- what is sent

/// What a notification is about: the plugin's name for it (`post:0193…`, at most 200 bytes), optionally one of its
/// entity rows (the subject). Unread notifications of one kind and key group into one row ("3 replies"); members
/// follow keys; an ask's key is its identity. A member who can no longer read the subject no longer sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    key: String,
    subject: Option<String>,
}

impl Key {
    /// A key of the plugin's own (no subject).
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            subject: None,
        }
    }

    /// The key of one of the plugin's entity rows, `<entity>:<id>`, with the row as its subject.
    pub fn row<E: Entity>(id: &Id<E>) -> Self {
        Self {
            key: format!("{}:{id}", E::ENTITY),
            subject: Some(format!("{}/{id}", E::ENTITY)),
        }
    }

    /// The same key tied to a row (`<entity>/<id>`).
    pub fn subject<E: Entity>(mut self, id: &Id<E>) -> Self {
        self.subject = Some(format!("{}/{id}", E::ENTITY));
        self
    }

    pub fn as_str(&self) -> &str {
        &self.key
    }
}

/// Where a notification opens: one of the plugin's screens or document-level overlays (a side panel, shown beside
/// the inbox's list), with its params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    screen: String,
    overlay: String,
    args: Value,
}

impl Link {
    pub fn screen<P: Message>(screen: &ScreenKey<P>, params: &P) -> Self {
        Self {
            screen: screen.id().into(),
            overlay: String::new(),
            args: params.to_json(),
        }
    }

    pub fn overlay<P: Message>(overlay: &OverlayKey<P>, params: &P) -> Self {
        Self {
            screen: String::new(),
            overlay: overlay.id().into(),
            args: params.to_json(),
        }
    }

    pub(crate) fn wire(&self) -> pb::NotifyLink {
        pb::NotifyLink {
            screen: self.screen.clone(),
            overlay: self.overlay.clone(),
            args: self.args.to_string(),
            ..Default::default()
        }
    }
}

/// Who a notification shows as its actor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Actor {
    /// the caller of the request being handled
    #[default]
    Caller,
    /// one of the plugin's own agents, by name
    Agent(String),
    /// nobody: the plugin's mark
    Nobody,
}

impl Actor {
    fn wire(&self) -> pb::NotifyActor {
        pb::NotifyActor {
            who: Some(match self {
                Actor::Caller => pb::notify_actor::Who::Caller(true),
                Actor::Agent(a) => pb::notify_actor::Who::Agent(a.clone()),
                Actor::Nobody => pb::notify_actor::Who::None(true),
            }),
        }
    }
}

fn ids<'a>(users: impl IntoIterator<Item = &'a Id<User>>) -> Vec<String> {
    users.into_iter().map(|u| u.to_string()).collect()
}

/// A notification of one of the plugin's kinds: to members, to the followers of a key, or both.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Notice {
    to: Vec<String>,
    followers: bool,
    except: Vec<String>,
    key: Option<Key>,
    actor: Actor,
    title: String,
    body: String,
    source: Option<(String, String)>,
    link: Option<Link>,
    once: String,
}

impl Notice {
    /// To these members (at most 100).
    pub fn to<'a>(users: impl IntoIterator<Item = &'a Id<User>>) -> Self {
        Self {
            to: ids(users),
            ..Default::default()
        }
    }

    /// To the active followers of `key` (its key too).
    pub fn followers(key: &Key) -> Self {
        Self {
            followers: true,
            key: Some(key.clone()),
            ..Default::default()
        }
    }

    /// Also to these members.
    pub fn and_to<'a>(mut self, users: impl IntoIterator<Item = &'a Id<User>>) -> Self {
        self.to.extend(ids(users));
        self
    }

    /// Not to these members (those already told in this request).
    pub fn except<'a>(mut self, users: impl IntoIterator<Item = &'a Id<User>>) -> Self {
        self.except.extend(ids(users));
        self
    }

    /// What it is about: unread notifications of its kind and key group into one row.
    pub fn key(mut self, key: &Key) -> Self {
        self.key = Some(key.clone());
        self
    }

    /// The phrase after the actor ("mentioned you in #fulfilment"), at most 140 characters.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// A snippet, at most 500 characters of plain text.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Where it happened: a label ("#fulfilment") and an icon of the kit ("hash").
    pub fn source(mut self, label: impl Into<String>, icon: impl Into<String>) -> Self {
        self.source = Some((label.into(), icon.into()));
        self
    }

    pub fn link(mut self, link: Link) -> Self {
        self.link = Some(link);
        self
    }

    /// Shown as one of the plugin's agents instead of the caller.
    pub fn agent(mut self, name: impl Into<String>) -> Self {
        self.actor = Actor::Agent(name.into());
        self
    }

    /// Shown as the plugin itself, without an actor.
    pub fn no_actor(mut self) -> Self {
        self.actor = Actor::Nobody;
        self
    }

    /// A member gets at most one notification of the plugin with this value (at most 120 bytes); never grouped.
    pub fn once(mut self, value: impl Into<String>) -> Self {
        self.once = value.into();
        self
    }
}

/// A button of an ask: it calls one of the plugin's `ui` endpoints, by its handler, with this request, as the
/// member who presses it. The endpoint checks the member and resolves the ask.
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    label: String,
    endpoint: String,
    body: Value,
    primary: bool,
    icon: String,
}

impl Button {
    pub fn new<Req: Message, Resp, F, Fut>(label: impl Into<String>, _handler: F, req: &Req) -> Self
    where
        F: Fn(Ctx, Req) -> Fut,
        Fut: Future<Output = Result<Resp>>,
    {
        Self {
            label: label.into(),
            endpoint: Req::NAME
                .strip_suffix("Request")
                .filter(|n| !n.is_empty())
                .unwrap_or(Req::NAME)
                .to_string(),
            body: req.to_json(),
            primary: false,
            icon: String::new(),
        }
    }

    /// The row's main button.
    pub fn primary<Req: Message, Resp, F, Fut>(
        label: impl Into<String>,
        handler: F,
        req: &Req,
    ) -> Self
    where
        F: Fn(Ctx, Req) -> Fut,
        Fut: Future<Output = Result<Resp>>,
    {
        Self {
            primary: true,
            ..Self::new(label, handler, req)
        }
    }

    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self
    }
}

/// An approval or a decision a member answers in their inbox (NEEDS YOU): buttons that call the plugin's own
/// endpoints, and a review link. It stays until it is resolved (`resolve`) or the member marks it done; sending it
/// again refreshes the open one of its key.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    kind: pb::notify_ask::Kind,
    key: Key,
    notice: Notice,
    buttons: Vec<Button>,
    review: String,
}

impl Ask {
    pub fn approval(key: Key) -> Self {
        Self::of(pb::notify_ask::Kind::Approval, key)
    }

    pub fn decision(key: Key) -> Self {
        Self::of(pb::notify_ask::Kind::Decision, key)
    }

    fn of(kind: pb::notify_ask::Kind, key: Key) -> Self {
        Self {
            kind,
            key,
            notice: Notice::default(),
            buttons: vec![],
            review: String::new(),
        }
    }

    pub fn to<'a>(mut self, users: impl IntoIterator<Item = &'a Id<User>>) -> Self {
        self.notice.to.extend(ids(users));
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.notice.title = title.into();
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.notice.body = body.into();
        self
    }

    pub fn source(mut self, label: impl Into<String>, icon: impl Into<String>) -> Self {
        self.notice.source = Some((label.into(), icon.into()));
        self
    }

    /// What the review link opens.
    pub fn link(mut self, link: Link) -> Self {
        self.notice.link = Some(link);
        self
    }

    /// The review link's label ("Review 5 changes").
    pub fn review(mut self, label: impl Into<String>) -> Self {
        self.review = label.into();
        self
    }

    /// One of 1 to 3 buttons.
    pub fn button(mut self, button: Button) -> Self {
        self.buttons.push(button);
        self
    }

    pub fn agent(mut self, name: impl Into<String>) -> Self {
        self.notice.actor = Actor::Agent(name.into());
        self
    }

    pub fn no_actor(mut self) -> Self {
        self.notice.actor = Actor::Nobody;
        self
    }
}

/// How members come to follow a key.
#[derive(Debug, Clone, PartialEq)]
pub struct Follow {
    reason: pb::notify_follow::Reason,
    noun: String,
    users: Vec<String>,
}

impl Follow {
    /// The member asked to (a Follow button); resumes a follow they stopped.
    pub fn manual() -> Self {
        Self::of(pb::notify_follow::Reason::Manual)
    }

    /// They replied: only members whose setting "follow threads I reply in" is on and who never stopped following
    /// the key.
    pub fn replied() -> Self {
        Self::of(pb::notify_follow::Reason::Replied)
    }

    /// They edited: as `replied`, with the setting "follow records I edit".
    pub fn edited() -> Self {
        Self::of(pb::notify_follow::Reason::Edited)
    }

    fn of(reason: pb::notify_follow::Reason) -> Self {
        Self {
            reason,
            noun: String::new(),
            users: vec![],
        }
    }

    /// What the inbox calls the key ("thread": "Stop following this thread"), at most 24 characters.
    pub fn noun(mut self, noun: impl Into<String>) -> Self {
        self.noun = noun.into();
        self
    }

    /// These members (at most 100) instead of the caller: with `replied()` or `edited()` only (a manual follow is
    /// the caller's own; naming anyone else is `invalid`).
    pub fn users<'a>(mut self, users: impl IntoIterator<Item = &'a Id<User>>) -> Self {
        self.users = ids(users);
        self
    }
}

/// The plugin's notifications (`ctx.notifications()`; `tx.notifications()` writes in the transaction, and its
/// members hear of it when it commits). A notification never undoes the transaction: a call the platform or a limit
/// refuses in it is let go (logged) and answers ok, so `?` is safe. Send late in a transaction: a send holds the
/// unread groups it grew until the commit. Needs the capability `push:send` (403 `capability_missing`); a send the
/// platform refuses is 400 `invalid`; sends over 50 a second are let go; a manual follow of a key with 2,000
/// followers, outside a transaction, is 409 `follow_limit`. A send answers nothing about who received it.
pub struct Notifications<'a> {
    ctx: &'a Ctx,
}

impl Ctx {
    pub fn notifications(&self) -> Notifications<'_> {
        Notifications { ctx: self }
    }
}

impl Notifications<'_> {
    async fn call(&self, call: syscall::Call) -> Result<Option<syscall_result::Result>> {
        self.ctx.syscall(call).await
    }

    fn wire(
        &self,
        kind: &str,
        n: &Notice,
        ask: Option<pb::NotifyAsk>,
        key: Option<&Key>,
    ) -> pb::NotifySend {
        let key = key.or(n.key.as_ref());
        pb::NotifySend {
            request_id: self.ctx.request().id(),
            tx: self.ctx.tx(),
            kind: kind.into(),
            to: n.to.clone(),
            followers: n.followers,
            except: n.except.clone(),
            key: key.map(|k| k.key.clone()).unwrap_or_default(),
            subject: key.and_then(|k| k.subject.clone()).unwrap_or_default(),
            actor: Some(n.actor.wire()),
            title: n.title.clone(),
            body: n.body.clone(),
            source: n.source.as_ref().map(|(label, icon)| pb::NotifySource {
                label: label.clone(),
                icon: icon.clone(),
            }),
            link: n.link.as_ref().map(Link::wire),
            ask,
            once: n.once.clone(),
        }
    }

    /// Sends a notification of one of the plugin's kinds.
    pub async fn send(&self, kind: &Kind, notice: Notice) -> Result<()> {
        let call = self.wire(kind.name(), &notice, None, None);
        self.call(syscall::Call::NotifySend(call)).await.map(|_| ())
    }

    /// Asks members for an approval or a decision.
    pub async fn ask(&self, ask: Ask) -> Result<()> {
        let wire = pb::NotifyAsk {
            kind: ask.kind as i32,
            buttons: ask
                .buttons
                .iter()
                .map(|b| pb::NotifyButton {
                    label: b.label.clone(),
                    endpoint: b.endpoint.clone(),
                    body: b.body.to_string(),
                    primary: b.primary,
                    icon: b.icon.clone(),
                })
                .collect(),
            review: ask.review.clone(),
        };
        let call = self.wire("", &ask.notice, Some(wire), Some(&ask.key));
        self.call(syscall::Call::NotifySend(call)).await.map(|_| ())
    }

    /// Resolves every open ask of `key`, for every member it asked: `outcome` ("Approved by Sarah Bennett", at most
    /// 80 characters) is what the inbox shows.
    pub async fn resolve(&self, key: &Key, outcome: impl Into<String>) -> Result<()> {
        let call = pb::NotifyResolve {
            request_id: self.ctx.request().id(),
            tx: self.ctx.tx(),
            key: key.key.clone(),
            outcome: outcome.into(),
        };
        self.call(syscall::Call::NotifyResolve(call))
            .await
            .map(|_| ())
    }

    fn follow_call(
        &self,
        key: &Key,
        op: pb::notify_follow::Op,
        f: Option<&Follow>,
    ) -> pb::NotifyFollow {
        pb::NotifyFollow {
            request_id: self.ctx.request().id(),
            tx: self.ctx.tx(),
            key: key.key.clone(),
            subject: key.subject.clone().unwrap_or_default(),
            noun: f.map(|f| f.noun.clone()).unwrap_or_default(),
            users: f.map(|f| f.users.clone()).unwrap_or_default(),
            reason: f.map_or(pb::notify_follow::Reason::Unspecified, |f| f.reason) as i32,
            op: op as i32,
        }
    }

    /// Makes the caller (or, for `replied()` and `edited()`, `Follow::users`) follow `key`.
    pub async fn follow(&self, key: &Key, follow: Follow) -> Result<()> {
        let call = self.follow_call(key, pb::notify_follow::Op::Follow, Some(&follow));
        self.call(syscall::Call::NotifyFollow(call))
            .await
            .map(|_| ())
    }

    /// The caller stops following `key` (an automatic follow does not resume it).
    pub async fn unfollow(&self, key: &Key) -> Result<()> {
        let call = self.follow_call(key, pb::notify_follow::Op::Unfollow, None);
        self.call(syscall::Call::NotifyFollow(call))
            .await
            .map(|_| ())
    }

    /// Whether the caller follows `key` (for a "Following" button): the one thing a plugin may read of
    /// notifications.
    pub async fn following(&self, key: &Key) -> Result<bool> {
        let call = self.follow_call(key, pb::notify_follow::Op::Get, None);
        match self.call(syscall::Call::NotifyFollow(call)).await? {
            Some(syscall_result::Result::NotifyFollowState(s)) => Ok(s.following),
            _ => Err(Error::internal(
                "the platform answered a follow with something else",
            )),
        }
    }
}
