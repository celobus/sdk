//! Notifications over the session: what the SDK sends for each call, and how it reads the platform's refusals, against
//! a fake platform (`db_tests::ctx_with`).

use celobus_proto as pb;
use pb::{syscall, syscall_result};
use serde_json::json;

use crate::db_tests::ctx_with;
use crate::notify::{Ask, Button, Follow, Key, Kind, Link, Notice};
use crate::platform::User;
use crate::prelude::*;
use crate::ui::{OverlayKey, ScreenKey};

#[derive(Entity, Clone, Debug, PartialEq)]
pub struct Post {
    pub id: Id<Post>,
    pub text: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ChannelParams {
    pub c: String,
    pub thread: Option<String>,
}

#[derive(Message, Clone, Debug)]
pub struct ApproveRunRequest {
    pub id: String,
}

async fn approve_run(_: Ctx, req: ApproveRunRequest) -> Result<ApproveRunRequest> {
    Ok(req)
}

const MENTION: Kind = Kind::new("mention").title("Mentions").digest();
const CHANNEL: ScreenKey<ChannelParams> = ScreenKey::new("channel");
const THREAD: OverlayKey<ChannelParams> = OverlayKey::new("thread");

fn ok() -> std::result::Result<syscall_result::Result, (String, Option<pb::DbErrorDetails>)> {
    Ok(syscall_result::Result::Ok(pb::Empty {}))
}

#[tokio::test]
async fn a_send_carries_the_notice_and_the_request() {
    let (ctx, seen) = ctx_with(|c| match c {
        syscall::Call::TxBegin(_) => Ok(syscall_result::Result::TxBegun(pb::TxBegun { tx: 5 })),
        _ => ok(),
    });
    let post: Id<Post> =
        Id::new(crate::types::Uuid::parse_str("0193a4c2-0000-7000-8000-000000000001").unwrap());
    let sarah: Id<User> = Id::new("0193a4c2-0000-7000-8000-0000000000aa".to_string());
    let thread = Key::row(&post);
    assert_eq!(thread.as_str(), "post:0193a4c2-0000-7000-8000-000000000001");
    let link = Link::screen(
        &CHANNEL,
        &ChannelParams {
            c: "1".into(),
            thread: Some("2".into()),
        },
    );
    ctx.notifications()
        .send(
            &MENTION,
            Notice::to([&sarah])
                .key(&thread)
                .title("mentioned you in #fulfilment")
                .body("@Sarah can we hold #1042")
                .source("#fulfilment", "hash")
                .link(link),
        )
        .await
        .unwrap();
    ctx.transaction(async |tx| {
        tx.notifications()
            .send(
                &MENTION,
                Notice::followers(&thread)
                    .except([&sarah])
                    .title("replied")
                    .once("x")
                    .no_actor(),
            )
            .await
    })
    .await
    .unwrap();
    let calls = seen.lock().unwrap().clone();
    let syscall::Call::NotifySend(s) = &calls[0] else {
        panic!("{calls:?}")
    };
    assert_eq!(s.request_id, 77);
    assert_eq!((s.tx, s.kind.as_str(), s.followers), (0, "mention", false));
    assert_eq!(s.to, ["0193a4c2-0000-7000-8000-0000000000aa"]);
    assert_eq!(s.key, "post:0193a4c2-0000-7000-8000-000000000001");
    assert_eq!(s.subject, "post/0193a4c2-0000-7000-8000-000000000001");
    assert_eq!(
        s.actor,
        Some(pb::NotifyActor {
            who: Some(pb::notify_actor::Who::Caller(true))
        })
    );
    assert_eq!(s.source.as_ref().unwrap().icon, "hash");
    let l = s.link.as_ref().unwrap();
    assert_eq!((l.screen.as_str(), l.overlay.as_str()), ("channel", ""));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&l.args).unwrap(),
        json!({ "c": "1", "thread": "2" })
    );
    assert!(s.ask.is_none());
    // in a transaction: on it, to the followers of the key but Sarah, as nobody
    let syscall::Call::NotifySend(s) = &calls[2] else {
        panic!("{calls:?}")
    };
    assert_eq!((s.tx, s.followers, s.once.as_str()), (5, true, "x"));
    assert_eq!(s.except, ["0193a4c2-0000-7000-8000-0000000000aa"]);
    assert!(s.to.is_empty());
    assert_eq!(s.key, "post:0193a4c2-0000-7000-8000-000000000001");
    assert_eq!(
        s.actor,
        Some(pb::NotifyActor {
            who: Some(pb::notify_actor::Who::None(true))
        })
    );
    assert!(matches!(calls[3], syscall::Call::TxCommit(_)));
}

#[tokio::test]
async fn an_ask_names_its_buttons_by_their_endpoints() {
    let (ctx, seen) = ctx_with(|_| ok());
    let sarah: Id<User> = Id::new("0193a4c2-0000-7000-8000-0000000000aa".to_string());
    let key = Key::new("approval:1");
    ctx.notifications()
        .ask(
            Ask::approval(key.clone())
                .to([&sarah])
                .agent("fulfilment")
                .title("needs your approval")
                .link(Link::overlay(
                    &THREAD,
                    &ChannelParams {
                        c: "9".into(),
                        thread: None,
                    },
                ))
                .review("Review 5 changes")
                .button(
                    Button::primary(
                        "Approve",
                        approve_run,
                        &ApproveRunRequest { id: "1".into() },
                    )
                    .icon("check"),
                ),
        )
        .await
        .unwrap();
    ctx.notifications()
        .resolve(&key, "Approved by Sarah Bennett")
        .await
        .unwrap();
    let calls = seen.lock().unwrap().clone();
    let syscall::Call::NotifySend(s) = &calls[0] else {
        panic!()
    };
    assert_eq!(
        (s.kind.as_str(), s.key.as_str(), s.subject.as_str()),
        ("", "approval:1", "")
    );
    assert_eq!(
        s.actor,
        Some(pb::NotifyActor {
            who: Some(pb::notify_actor::Who::Agent("fulfilment".into()))
        })
    );
    let a = s.ask.as_ref().unwrap();
    assert_eq!(a.kind, pb::notify_ask::Kind::Approval as i32);
    assert_eq!(a.review, "Review 5 changes");
    assert_eq!(
        a.buttons,
        [pb::NotifyButton {
            label: "Approve".into(),
            endpoint: "ApproveRun".into(),
            body: r#"{"id":"1"}"#.into(),
            primary: true,
            icon: "check".into(),
        }]
    );
    assert_eq!(s.link.as_ref().unwrap().overlay, "thread");
    let syscall::Call::NotifyResolve(r) = &calls[1] else {
        panic!()
    };
    assert_eq!(
        (r.key.as_str(), r.outcome.as_str(), r.request_id),
        ("approval:1", "Approved by Sarah Bennett", 77)
    );
}

#[tokio::test]
async fn follows_and_their_refusals() {
    let (ctx, seen) = ctx_with(|c| match c {
        syscall::Call::NotifyFollow(f) if f.op == pb::notify_follow::Op::Get as i32 => Ok(
            syscall_result::Result::NotifyFollowState(pb::NotifyFollowState { following: true }),
        ),
        syscall::Call::NotifyFollow(f) if f.key == "full" => {
            Err(("follow_limit: the key has 2000 followers".into(), None))
        }
        syscall::Call::NotifySend(_) => Err(("rate_limited: too many".into(), None)),
        _ => ok(),
    });
    let key = Key::new("post:1");
    let n = ctx.notifications();
    n.follow(&key, Follow::replied().noun("thread"))
        .await
        .unwrap();
    assert!(n.following(&key).await.unwrap());
    n.unfollow(&key).await.unwrap();
    let e = n
        .follow(&Key::new("full"), Follow::manual())
        .await
        .unwrap_err();
    assert_eq!((e.status(), e.code()), (409, "follow_limit"));
    let e = n
        .send(&MENTION, Notice::to([]).title("x"))
        .await
        .unwrap_err();
    assert_eq!((e.status(), e.code()), (429, "rate_limited"));
    let calls = seen.lock().unwrap().clone();
    let syscall::Call::NotifyFollow(f) = &calls[0] else {
        panic!()
    };
    assert_eq!(
        (f.op, f.reason, f.noun.as_str(), f.users.len()),
        (
            pb::notify_follow::Op::Follow as i32,
            pb::notify_follow::Reason::Replied as i32,
            "thread",
            0
        )
    );
    let syscall::Call::NotifyFollow(f) = &calls[2] else {
        panic!()
    };
    assert_eq!(
        (f.op, f.reason),
        (pb::notify_follow::Op::Unfollow as i32, 0)
    );
}
