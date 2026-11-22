//! The SDK conformance plugin `notify`: the notification samples as one small plugin, the
//! same in Rust, TypeScript and C#. A reply notifies who it mentions, makes its author follow the thread and tells
//! the thread's other followers; a member follows a thread or stops; an agent's approval is asked with two buttons
//! and resolved by the one pressed. Its `--describe` must equal `../descriptor.json`, as the TypeScript and C#
//! versions' must (`scripts/sdk-conformance.sh`).

use celobus_sdk::notify::{Ask, Button, Follow, Key, Kind, Link, Notice};
use celobus_sdk::prelude::*;
use celobus_sdk::ui::{Overlay, OverlayKey, Screen, ScreenKey, Ui, text};

pub const MEMBER: Role = Role::new("member").title("Member");

pub const MENTION: Kind = Kind::new("mention")
    .title("Mentions")
    .describe("Someone wrote @you in a channel or a thread")
    .digest();
pub const THREAD_REPLY: Kind = Kind::new("thread_reply")
    .title("Replies in threads you follow")
    .describe("Arrives in Watching")
    .watching();

#[derive(Entity, Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: Id<Channel>,
    pub name: String,
}

#[derive(Entity, Clone, Debug, PartialEq)]
pub struct Post {
    pub id: Id<Post>,
    pub channel: Id<Channel>,
    // absent: the post starts a thread
    pub thread: Option<Id<Post>>,
    pub text: String,
}

// An agent's request for a person's approval; the ask in their inbox is keyed by it.
#[derive(Entity, Clone, Debug, PartialEq)]
pub struct Approval {
    pub id: Id<Approval>,
    pub owner: Id<platform::User>,
    pub order: String,
    pub summary: String,
    // pending, approved or declined
    pub status: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ChannelParams {
    pub c: String,
    pub thread: Option<String>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct OrderParams {
    pub id: String,
}

#[derive(Message, Clone, Debug)]
pub struct PostReplyRequest {
    pub channel: Id<Channel>,
    pub thread: Id<Post>,
    pub text: String,
    // the members the text mentions
    pub mentioned: Vec<Id<platform::User>>,
}

#[derive(Message, Clone, Debug)]
pub struct PostView {
    pub id: Id<Post>,
    pub text: String,
}

#[derive(Message, Clone, Debug)]
pub struct FollowThreadRequest {
    pub thread: Id<Post>,
    // false: stop following
    pub follow: bool,
}

#[derive(Message, Clone, Debug)]
pub struct FollowState {
    pub following: bool,
}

#[derive(Message, Clone, Debug)]
pub struct RequestApprovalRequest {
    pub owner: Id<platform::User>,
    pub order: String,
    pub summary: String,
}

#[derive(Message, Clone, Debug)]
pub struct ApproveRequest {
    pub id: Id<Approval>,
}

#[derive(Message, Clone, Debug)]
pub struct DeclineRequest {
    pub id: Id<Approval>,
}

pub const CHANNEL: ScreenKey<ChannelParams> = ScreenKey::new("channel");
pub const ORDER_PANEL: OverlayKey<OrderParams> = OverlayKey::new("order_panel");

fn snippet(text: &str) -> String {
    text.chars().take(140).collect()
}

/// A reply in a thread: the members it mentions are told, its author follows the thread (if their setting says so),
/// and the thread's other followers are told, all in the transaction that writes the post.
pub async fn post_reply(ctx: Ctx, req: PostReplyRequest) -> Result<PostView> {
    ctx.transaction(async |tx| {
        let ch = tx.db::<Channel>().get(&req.channel).await?;
        let post = tx
            .db::<Post>()
            .create(Post {
                id: Id::none(),
                channel: req.channel,
                thread: Some(req.thread),
                text: req.text.clone(),
            })
            .await?;
        let thread = Key::row(&req.thread); // key "post:<id>", subject post/<id>
        let link = Link::screen(
            &CHANNEL,
            &ChannelParams {
                c: req.channel.to_string(),
                thread: Some(req.thread.to_string()),
            },
        );
        let source = format!("#{}", ch.name);
        let n = tx.notifications(); // last in the transaction
        n.send(
            &MENTION,
            Notice::to(&req.mentioned)
                .key(&thread)
                .title(format!("mentioned you in #{}", ch.name))
                .body(snippet(&req.text))
                .source(&source, "hash")
                .link(link.clone()),
        )
        .await?;
        n.follow(&thread, Follow::replied().noun("thread")).await?;
        n.send(
            &THREAD_REPLY,
            Notice::followers(&thread)
                .except(&req.mentioned)
                .title(format!("replied in #{}", ch.name))
                .body(snippet(&req.text))
                .source(&source, "hash")
                .link(link),
        )
        .await?;
        Ok(PostView {
            id: post.id,
            text: post.text.clone(),
        })
    })
    .await
}

/// The caller follows a thread (or stops), and answers whether they follow it now.
pub async fn follow_thread(ctx: Ctx, req: FollowThreadRequest) -> Result<FollowState> {
    let thread = Key::row(&req.thread);
    let n = ctx.notifications();
    if req.follow {
        n.follow(&thread, Follow::manual().noun("thread")).await?;
    } else {
        n.unfollow(&thread).await?;
    }
    Ok(FollowState {
        following: n.following(&thread).await?,
    })
}

/// An agent's approval: the Approval row and the ask in one transaction; the key is the ask's identity and the
/// caller (the agent's account) its actor.
pub async fn request_approval(ctx: Ctx, req: RequestApprovalRequest) -> Result<Row<Approval>> {
    ctx.transaction(async |tx| {
        let a = tx
            .db::<Approval>()
            .create(Approval {
                id: Id::none(),
                owner: req.owner.clone(),
                order: req.order.clone(),
                summary: req.summary.clone(),
                status: "pending".into(),
            })
            .await?;
        tx.notifications()
            .ask(
                Ask::approval(Key::row(&a.id))
                    .to([&req.owner])
                    .title("needs your approval")
                    .body(&req.summary)
                    .source(format!("#{}", req.order), "receipt")
                    .link(Link::overlay(
                        &ORDER_PANEL,
                        &OrderParams {
                            id: req.order.clone(),
                        },
                    ))
                    .review("Review the changes")
                    .button(
                        Button::primary("Approve", approve, &ApproveRequest { id: a.id })
                            .icon("check"),
                    )
                    .button(
                        Button::new("Decline", decline, &DeclineRequest { id: a.id }).icon("x"),
                    ),
            )
            .await?;
        Ok(a)
    })
    .await
}

/// Sets the approval's status and resolves its ask, for every member it asked.
async fn decide(
    ctx: &Ctx,
    id: &Id<Approval>,
    status: &str,
    outcome: &str,
) -> Result<Row<Approval>> {
    ctx.transaction(async |tx| {
        let a = tx
            .db::<Approval>()
            .update(id, Patch::new().set(Approval::status, status.to_string()))
            .await?;
        tx.notifications()
            .resolve(
                &Key::row(id),
                format!("{outcome} by {}", tx.caller().display_name()),
            )
            .await?;
        Ok(a)
    })
    .await
}

/// The ask's primary button.
pub async fn approve(ctx: Ctx, req: ApproveRequest) -> Result<Row<Approval>> {
    decide(&ctx, &req.id, "approved", "Approved").await
}

pub async fn decline(ctx: Ctx, req: DeclineRequest) -> Result<Row<Approval>> {
    decide(&ctx, &req.id, "declined", "Declined").await
}

fn channel_screen() -> Screen {
    Screen::build(CHANNEL, |s| {
        s.title("Channel");
        text("The channel and its threads.")
    })
}

fn order_panel() -> Overlay {
    Overlay::side_panel(ORDER_PANEL, |o| {
        o.title("Order");
        text("The order and its changes.")
    })
}

pub fn plugin() -> Plugin {
    Plugin::new("notify", env!("CARGO_PKG_VERSION"))
        .title("Notify")
        .describe("The SDK conformance plugin of notifications.")
        .role(MEMBER)
        .capability(
            Capability::PUSH_SEND,
            "Tells people when they are mentioned or written to.",
        )
        .notification(MENTION)
        .notification(THREAD_REPLY)
        .entity::<Channel>()
        .access::<Channel>(Access::build(|a| {
            a.read(MEMBER);
        }))
        .entity::<Post>()
        .access::<Post>(Access::build(|a| {
            a.read(MEMBER);
            a.create(MEMBER);
        }))
        .entity::<Approval>()
        .access::<Approval>(Access::build(|a| {
            let owner = a.relation("owner", Rel::column(Approval::owner));
            a.read(owner.clone());
            a.create(MEMBER);
            a.update(owner);
        }))
        .endpoint(
            Endpoint::new(post_reply)
                .describe("Posts a reply in a thread.")
                .roles([MEMBER]),
        )
        .endpoint(
            Endpoint::new(follow_thread)
                .describe("Follows a thread, or stops.")
                .roles([MEMBER])
                .ui(),
        )
        .endpoint(
            Endpoint::new(request_approval)
                .describe("Asks a member to approve an order's changes.")
                .roles([MEMBER]),
        )
        .endpoint(Endpoint::new(approve).describe("Approves.").ui())
        .endpoint(Endpoint::new(decline).describe("Declines.").ui())
        .ui(Ui::new().screen(channel_screen()).overlay(order_panel()))
}

fn main() {
    plugin().serve()
}
