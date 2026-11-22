//! The repository and transactions over the session: what the SDK sends for each call and how it reads the platform's
//! answers and refusals, against a fake platform.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use celobus_proto as pb;
use pb::{
    db_condition::Op, db_filter::Node, db_value::Kind, plugin_message, syscall, syscall_result,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::ctx::{Link, PluginInfo, RequestInfo};
use crate::prelude::*;

#[derive(Entity, Clone, Debug, PartialEq)]
#[entity(key = i64, version, soft_delete)]
pub struct Note {
    pub id: Id<Note>,
    #[col(searchable)]
    pub title: String,
    pub body: Option<String>,
}

/// A tag of a note: a forward nav `Tag::note`, an inverse one `Note::tags`.
#[derive(Entity, Clone, Debug, PartialEq)]
#[entity(key = i64)]
pub struct Tag {
    pub id: Id<Tag>,
    #[col(unique)]
    pub label: String,
    pub weight: i32,
    #[col(inverse = tags)]
    pub note_id: Id<Note>,
}

pub(crate) type Answer =
    std::result::Result<syscall_result::Result, (String, Option<pb::DbErrorDetails>)>;

/// A context of request 77 whose syscalls `answer` answers; the calls it saw.
pub(crate) fn ctx_with(
    answer: impl Fn(&syscall::Call) -> Answer + Send + 'static,
) -> (Ctx, Arc<Mutex<Vec<syscall::Call>>>) {
    let (tx, mut rx) = mpsc::channel(16);
    let link = Arc::new(Link::new(tx));
    let seen: Arc<Mutex<Vec<syscall::Call>>> = Arc::default();
    {
        let (link, seen) = (Arc::downgrade(&link), seen.clone());
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let Some(plugin_message::Kind::Syscall(sc)) = msg.kind else {
                    continue;
                };
                let call = sc.call.unwrap();
                let (result, error, details) = match answer(&call) {
                    Ok(r) => (Some(r), String::new(), None),
                    Err((e, d)) => (None, e, d),
                };
                seen.lock().unwrap().push(call);
                let Some(link) = link.upgrade() else { break };
                link.complete(pb::SyscallResult {
                    id: sc.id,
                    error,
                    result,
                    error_details: details,
                });
            }
        });
    }
    let ctx = Ctx::new(
        link,
        Arc::new(PluginInfo::new("hello", "1.0.0")),
        RequestInfo::new(77, Duration::from_secs(5), None),
        Caller::from_principal(None, "hello"),
    );
    (ctx, seen)
}

pub(crate) fn rows(rows: &[Value]) -> syscall_result::Result {
    syscall_result::Result::DbRows(pb::DbRows {
        rows: rows.iter().map(|r| Bytes::from(r.to_string())).collect(),
        ..Default::default()
    })
}

fn stored(id: i64, title: &str, version: i64) -> Value {
    json!({ "id": id.to_string(), "title": title, "version": version.to_string(),
            "createdAt": "2026-11-12T10:00:00.000000Z", "updatedAt": "2026-11-12T10:00:01.000000Z" })
}

fn json_of(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap()
}

#[tokio::test]
async fn create_get_and_the_row() {
    let (ctx, seen) = ctx_with(|call| match call {
        syscall::Call::DbCreate(c) => {
            let made: Vec<Value> = c
                .rows
                .iter()
                .enumerate()
                .map(|(i, r)| stored(i as i64 + 1, json_of(r)["title"].as_str().unwrap(), 1))
                .collect();
            Ok(rows(&made))
        }
        syscall::Call::DbGet(g) if g.id == "1" => Ok(rows(&[stored(1, "a", 1)])),
        _ => Err(("not_found: no note 9".into(), None)),
    });
    let note = Note {
        id: Id::none(),
        title: "a".into(),
        body: None,
    };
    let row = ctx.db::<Note>().create(note.clone()).await.unwrap();
    assert_eq!((row.id.key(), row.title.as_str()), (&1, "a"));
    assert_eq!(row.version(), Some(1));
    assert_eq!(row.created_at().to_rfc3339(), "2026-11-12T10:00:00+00:00");
    assert!(row.deleted_at().is_none());
    let many = ctx
        .db::<Note>()
        .create_many(vec![note.clone(), note.clone()])
        .await
        .unwrap();
    assert_eq!(many.len(), 2);
    let got = ctx.db::<Note>().get(&Id::new(1)).await.unwrap();
    assert_eq!(got.into_inner().title, "a");
    let missing = ctx.db::<Note>().get(&Id::new(9)).await.unwrap_err();
    assert_eq!((missing.status(), missing.code()), (404, "not_found"));

    let calls = seen.lock().unwrap().clone();
    let syscall::Call::DbCreate(c) = &calls[0] else {
        panic!("{:?}", calls[0]);
    };
    assert_eq!((c.request_id, c.tx, c.entity.as_str()), (77, 0, "note"));
    assert_eq!(json_of(&c.rows[0]), json!({ "id": "0", "title": "a" }));
    let syscall::Call::DbGet(g) = &calls[2] else {
        panic!();
    };
    assert_eq!(
        (g.entity.as_str(), g.id.as_str(), g.with_deleted),
        ("note", "1", false)
    );
}

#[tokio::test]
async fn find_filters_orders_and_pages() {
    let (ctx, seen) = ctx_with(|call| match call {
        syscall::Call::DbFind(q) if q.all => Ok(rows(&[stored(1, "a", 1), stored(2, "b", 1)])),
        syscall::Call::DbFind(q) if q.page_token.is_empty() => {
            Ok(syscall_result::Result::DbRows(pb::DbRows {
                rows: vec![Bytes::from(stored(1, "a", 1).to_string())],
                next_page_token: "t1".into(),
                ..Default::default()
            }))
        }
        syscall::Call::DbFind(_) => Ok(rows(&[])),
        syscall::Call::DbCount(_) => Ok(syscall_result::Result::DbCount(pb::DbCountResult {
            count: 3,
        })),
        _ => Err(("invalid: no".into(), None)),
    });
    let db = ctx.db::<Note>();
    let page = db
        .find(Note::title.eq("a".to_string()) & !Note::body.is_null())
        .order_by(Note::title.desc())
        .order_by(Note::created_at.asc())
        .limit(10)
        .with_deleted()
        .page()
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.next.as_deref(), Some("t1"));
    let next = db
        .find(Filter::all())
        .page_token(page.next.as_deref().unwrap())
        .page()
        .await
        .unwrap();
    assert!(next.rows.is_empty() && next.next.is_none());
    let all = db
        .find(Note::title.is_in(["a".to_string(), "b".to_string()]))
        .all()
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    let first = db.find(Filter::all()).first().await.unwrap();
    assert_eq!(first.unwrap().title, "a");
    assert_eq!(db.count().await.unwrap(), 3);
    assert!(db.exists(Note::title.ne("z".to_string())).await.unwrap());

    let calls = seen.lock().unwrap().clone();
    let syscall::Call::DbFind(q) = &calls[0] else {
        panic!();
    };
    assert_eq!(
        (q.limit, q.with_deleted, q.all, q.page_token.as_str()),
        (10, true, false, "")
    );
    let order: Vec<(&str, bool)> = q.order.iter().map(|o| (o.field.as_str(), o.desc)).collect();
    assert_eq!(order, [("title", true), ("created_at", false)]);
    let Some(Node::And(and)) = &q.filter.as_ref().unwrap().node else {
        panic!("{:?}", q.filter);
    };
    let Some(Node::Condition(c)) = &and.items[0].node else {
        panic!();
    };
    assert_eq!((c.field.as_str(), c.op), ("title", Op::Eq as i32));
    assert_eq!(c.values[0].kind, Some(Kind::StringValue("a".into())));
    let Some(Node::Not(not)) = &and.items[1].node else {
        panic!();
    };
    let Some(Node::Condition(c)) = &not.node else {
        panic!();
    };
    assert_eq!((c.field.as_str(), c.op), ("body", Op::IsNull as i32));
    let syscall::Call::DbFind(q) = &calls[1] else {
        panic!();
    };
    assert_eq!(q.page_token, "t1");
    assert!(q.filter.is_none());
    let syscall::Call::DbFind(q) = &calls[2] else {
        panic!();
    };
    assert!(q.all);
    let syscall::Call::DbFind(q) = &calls[3] else {
        panic!();
    };
    assert_eq!(q.limit, 1);
    let syscall::Call::DbCount(q) = &calls[4] else {
        panic!();
    };
    assert!(q.filter.is_none());
    // exists: the first matching row
    let syscall::Call::DbFind(q) = &calls[5] else {
        panic!();
    };
    assert!(q.filter.is_some() && q.limit == 1);
}

#[tokio::test]
async fn updates_deletes_and_their_refusals() {
    let (ctx, seen) = ctx_with(|call| match call {
        syscall::Call::DbUpdate(u) if u.expected_version == Some(1) => Err((
            "version_conflict: note 1 is at version 2".into(),
            Some(pb::DbErrorDetails {
                current_version: 2,
                ..Default::default()
            }),
        )),
        syscall::Call::DbUpdate(_) => Ok(rows(&[stored(1, "b", 3)])),
        syscall::Call::DbDelete(d) if d.force => Ok(syscall_result::Result::Ok(pb::Empty {})),
        syscall::Call::DbDelete(_) => Err((
            "unique_conflict: another row has this title".into(),
            Some(pb::DbErrorDetails {
                column: "title".into(),
                ..Default::default()
            }),
        )),
        syscall::Call::DbRestore(_) => Ok(rows(&[stored(1, "b", 4)])),
        syscall::Call::DbCreate(_) => {
            Err(("bulk_limit: at most 1000 rows in one create".into(), None))
        }
        _ => Err(("query_rows_limit: more than 1000 rows".into(), None)),
    });
    let db = ctx.db::<Note>();
    let patch = Patch::new()
        .set(Note::title, "b".to_string())
        .clear(Note::body);
    let row = db
        .update(&Id::new(1), patch.clone())
        .expect_version(2)
        .await
        .unwrap();
    assert_eq!(row.version(), Some(3));
    let stale = db
        .update(&Id::new(1), patch)
        .expect_version(1)
        .await
        .unwrap_err();
    assert_eq!((stale.status(), stale.code()), (409, "version_conflict"));
    assert_eq!(stale.details()["current"], json!(2));
    let dup = db.delete(&Id::new(1)).await.unwrap_err();
    assert_eq!((dup.status(), dup.code()), (409, "unique_conflict"));
    assert_eq!(dup.details()["column"], json!("title"));
    db.delete_force(&Id::new(1)).await.unwrap();
    assert_eq!(db.restore(&Id::new(1)).await.unwrap().version(), Some(4));
    let e = db.find(Filter::all()).all().await.unwrap_err();
    assert_eq!((e.status(), e.code()), (422, "query_rows_limit"));
    let e = db.create_many(vec![]).await.unwrap_err();
    assert_eq!((e.status(), e.code()), (422, "bulk_limit"));

    let calls = seen.lock().unwrap().clone();
    let syscall::Call::DbUpdate(u) = &calls[0] else {
        panic!();
    };
    assert_eq!((u.id.as_str(), u.expected_version), ("1", Some(2)));
    assert_eq!(json_of(&u.patch), json!({ "title": "b", "body": null }));
    let syscall::Call::DbDelete(d) = &calls[3] else {
        panic!();
    };
    assert!(d.force && d.expected_version.is_none());
}

#[tokio::test]
async fn transactions_carry_their_id_and_end_as_their_block_does() {
    let (ctx, seen) = ctx_with(|call| match call {
        syscall::Call::TxBegin(b) => Ok(syscall_result::Result::TxBegun(pb::TxBegun {
            tx: if b.tx == 0 { 5 } else { 6 },
        })),
        syscall::Call::DbCreate(_) => Ok(rows(&[stored(1, "a", 1)])),
        _ => Ok(syscall_result::Result::Ok(pb::Empty {})),
    });
    let note = Note {
        id: Id::none(),
        title: "a".into(),
        body: None,
    };
    let n = note.clone();
    let made = ctx
        .transaction(async move |tx| {
            tx.db::<Note>().create(n.clone()).await?;
            // a block inside a block: rolled back alone by its error, the outer one goes on
            let inner: Result<()> = tx
                .transaction(async |sp| {
                    sp.db::<Note>().create(n.clone()).await?;
                    Err(Error::conflict("no"))
                })
                .await;
            assert!(inner.is_err());
            Ok(7)
        })
        .await
        .unwrap();
    assert_eq!(made, 7);
    let failed: Result<()> = ctx
        .transaction_with(TxOptions::new().serializable().read_only(), async |_| {
            Err(Error::invalid("nope"))
        })
        .await;
    assert_eq!(failed.unwrap_err().code(), "invalid");

    let calls = seen.lock().unwrap().clone();
    let kinds: Vec<String> = calls
        .iter()
        .map(|c| match c {
            syscall::Call::TxBegin(b) => format!("begin {} {} {}", b.tx, b.isolation, b.read_only),
            syscall::Call::TxCommit(c) => format!("commit {}", c.tx),
            syscall::Call::TxRollback(c) => format!("rollback {}", c.tx),
            syscall::Call::DbCreate(c) => format!("create {}", c.tx),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "begin 0 0 false",
            "create 5",
            "begin 5 0 false",
            "create 6",
            "rollback 6",
            "commit 5",
            "begin 0 2 true",
            "rollback 5"
        ]
    );
    let syscall::Call::TxBegin(b) = &calls[0] else {
        panic!();
    };
    assert_eq!(b.request_id, 77);
}

/// `as_system`: every call of the context, and the transactions it begins, carry the reason; the caller's own
/// context carries none; a refusal of the rules is a 403 with its code.
#[tokio::test]
async fn a_system_context_names_its_reason_on_every_call() {
    let (ctx, seen) = ctx_with(|call| match call {
        syscall::Call::DbGet(_) => Ok(rows(&[stored(1, "a", 1)])),
        syscall::Call::TxBegin(_) => Ok(syscall_result::Result::TxBegun(pb::TxBegun { tx: 9 })),
        syscall::Call::DbCount(_) => Ok(syscall_result::Result::DbCount(pb::DbCountResult {
            count: 3,
        })),
        syscall::Call::TxCommit(_) => Ok(syscall_result::Result::Ok(pb::Empty {})),
        syscall::Call::DbDelete(_) => Err(("permission_denied: not yours".into(), None)),
        _ => panic!("{call:?}"),
    });
    let system = ctx.as_system("archive");
    system.db::<Note>().get(&Id::new(1)).await.unwrap();
    let n = system
        .transaction(async |tx| tx.db::<Note>().count().await)
        .await
        .unwrap();
    assert_eq!(n, 3);
    let e = ctx.db::<Note>().delete(&Id::new(1)).await.unwrap_err();
    assert_eq!((e.status(), e.code()), (403, "permission_denied"));
    let seen = seen.lock().unwrap();
    let systems: Vec<String> = seen
        .iter()
        .map(|c| match c {
            syscall::Call::DbGet(x) => x.as_system.clone(),
            syscall::Call::TxBegin(x) => x.as_system.clone(),
            syscall::Call::DbCount(x) => x.as_system.clone(),
            syscall::Call::DbDelete(x) => x.as_system.clone(),
            _ => "-".into(),
        })
        .collect();
    assert_eq!(systems, ["archive", "archive", "archive", "-", ""]);
}

/// A dependency's entity as `celobus plugin add` generates it.
#[derive(Entity, Clone, Debug, PartialEq)]
#[entity(plugin = "crm", key = uuid, soft_delete)]
pub struct Customer {
    pub id: Id<Customer>,
    pub name: String,
    pub email: Option<String>,
}
impl crate::sql::Exported for Customer {}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct Tally {
    pub owner_id: Id<platform::User>,
    pub notes: i64,
}

const TALLY: &str =
    "select owner_id, count(*) as notes from notes_note group by owner_id having count(*) >= $1";

const ADA: &str = "0190f3c2-0000-7000-8000-0000000000aa";

#[tokio::test]
async fn query_reads_a_dependencys_entity_by_its_provider() {
    let customer = json!({ "id": ADA, "name": "Ada", "createdAt": "2026-11-12T10:00:00.000000Z",
                           "updatedAt": "2026-11-12T10:00:00.000000Z" });
    let (ctx, seen) = ctx_with(move |call| match call {
        syscall::Call::DbGet(_) | syscall::Call::DbFind(_) | syscall::Call::Query(_) => {
            Ok(rows(std::slice::from_ref(&customer)))
        }
        syscall::Call::DbCount(_) => Ok(syscall_result::Result::DbCount(pb::DbCountResult {
            count: 1,
        })),
        _ => Err(("invalid: no".into(), None)),
    });
    let q = ctx.query::<Customer>();
    let ada = q.get(&Id::new(ADA.parse().unwrap())).await.unwrap();
    assert_eq!((ada.name.as_str(), ada.email.as_deref()), ("Ada", None));
    let found = q
        .find(Customer::name.eq("Ada".into()))
        .order_by(Customer::created_at.desc())
        .limit(5)
        .page_token("t")
        .page()
        .await
        .unwrap();
    assert_eq!(found.rows.len(), 1);
    assert_eq!(
        q.count_where(Customer::name.is_not_null()).await.unwrap(),
        1
    );
    assert_eq!(q.all().await.unwrap().len(), 1);

    let calls = seen.lock().unwrap().clone();
    let syscall::Call::DbGet(g) = &calls[0] else {
        panic!("{:?}", calls[0])
    };
    assert_eq!(
        (
            g.dependency.as_str(),
            g.entity.as_str(),
            g.id.as_str(),
            g.as_system.as_str()
        ),
        ("crm", "customer", ADA, "")
    );
    let syscall::Call::DbFind(f) = &calls[1] else {
        panic!()
    };
    assert_eq!(
        (f.dependency.as_str(), f.entity.as_str()),
        ("crm", "customer")
    );
    assert_eq!((f.limit, f.page_token.as_str(), f.all), (5, "t", false));
    assert_eq!(f.order[0].field, "created_at");
    let Some(Node::Condition(c)) = &f.filter.as_ref().unwrap().node else {
        panic!()
    };
    assert_eq!((c.field.as_str(), c.op), ("name", Op::Eq as i32));
    let syscall::Call::DbCount(n) = &calls[2] else {
        panic!()
    };
    assert_eq!(n.dependency, "crm");
    // every row: a query of the root alone (QueryRead), the provider's entity by its name
    let syscall::Call::Query(r) = &calls[3] else {
        panic!("{:?}", calls[3])
    };
    let root = &r.query.as_ref().unwrap().sources[0];
    assert_eq!(
        (root.entity.as_str(), root.alias.as_str()),
        ("crm.customer", "crm_customer")
    );
    // the plugin's own entities say no dependency
    let (ctx, seen) = ctx_with(|_| Ok(rows(&[stored(1, "a", 1)])));
    ctx.db::<Note>().get(&Id::new(1)).await.unwrap();
    let calls = seen.lock().unwrap().clone();
    let syscall::Call::DbGet(g) = &calls[0] else {
        panic!()
    };
    assert_eq!(g.dependency, "");
}

#[tokio::test]
async fn query_in_a_transaction_carries_its_id_and_refusals_come_back() {
    let (ctx, seen) = ctx_with(|call| match call {
        syscall::Call::TxBegin(_) => Ok(syscall_result::Result::TxBegun(pb::TxBegun { tx: 5 })),
        syscall::Call::TxCommit(_) | syscall::Call::TxRollback(_) => {
            Ok(syscall_result::Result::Ok(pb::Empty {}))
        }
        _ => Err((
            "permission_denied: crm.customer does not export read".into(),
            None,
        )),
    });
    let err = ctx
        .transaction(async |tx| tx.query::<Customer>().count().await)
        .await
        .unwrap_err();
    assert_eq!((err.status(), err.code()), (403, "permission_denied"));
    let calls = seen.lock().unwrap().clone();
    let syscall::Call::Query(c) = &calls[1] else {
        panic!("{:?}", calls[1])
    };
    assert_eq!(
        (c.tx, c.query.as_ref().unwrap().sources[0].entity.as_str()),
        (5, "crm.customer")
    );
}

#[tokio::test]
async fn sql_sends_its_text_and_values_and_reads_rows_by_column() {
    let owner = "0190f3c2-0000-7000-8000-000000000011";
    let (ctx, seen) = ctx_with(move |call| match call {
        // a column of SQL is its snake_case name; a bigint is a JSON number
        syscall::Call::Sql(_) => Ok(rows(&[json!({ "owner_id": owner, "notes": 3 })])),
        syscall::Call::TxBegin(_) => Ok(syscall_result::Result::TxBegun(pb::TxBegun { tx: 9 })),
        syscall::Call::TxCommit(_) => Ok(syscall_result::Result::Ok(pb::Empty {})),
        _ => Err(("invalid: no".into(), None)),
    });
    let got: Vec<Tally> = ctx.sql(TALLY).bind(2).rows().await.unwrap();
    assert_eq!(
        (got[0].owner_id.to_string(), got[0].notes),
        (owner.to_string(), 3)
    );
    let json = ctx
        .sql("select $1 as a, $2 as b, $3 as c")
        .bind("it's")
        .bind(None::<i64>)
        .bind(json!({ "x": [1] }))
        .json()
        .await
        .unwrap();
    assert_eq!(json[0]["notes"], 3);
    ctx.transaction(async |tx| tx.sql(TALLY).bind(1).json().await)
        .await
        .unwrap();
    ctx.unsafe_sql("update notes_note set title = $1 where id = $2")
        .bind("x")
        .bind(ADA)
        .run()
        .await
        .unwrap();
    let calls = seen.lock().unwrap().clone();
    let syscall::Call::Sql(s) = &calls[0] else {
        panic!("{:?}", calls[0])
    };
    assert_eq!(
        (s.request_id, s.tx, s.query.as_str(), s.r#unsafe),
        (77, 0, TALLY, false)
    );
    assert_eq!(json_of(&s.params), json!([2]));
    let syscall::Call::Sql(s) = &calls[1] else {
        panic!()
    };
    assert_eq!(json_of(&s.params), json!(["it's", null, { "x": [1] }]));
    let syscall::Call::Sql(s) = &calls[3] else {
        panic!("{:?}", calls[3])
    };
    assert_eq!(s.tx, 9);
    let syscall::Call::Sql(s) = &calls[5] else {
        panic!("{:?}", calls[5])
    };
    assert!(s.r#unsafe);
    assert_eq!(json_of(&s.params), json!(["x", ADA]));
}

#[tokio::test]
async fn sql_refusals_come_back_as_errors() {
    let (ctx, _) = ctx_with(|_| {
        Err((
            "query_rows_limit: the query answers more than 1000 rows".into(),
            None,
        ))
    });
    let err = ctx.sql(TALLY).bind(0).json().await.unwrap_err();
    assert_eq!((err.status(), err.code()), (422, "query_rows_limit"));
    let (ctx, _) = ctx_with(|_| {
        Err((
            "capability_missing: unsafe_sql needs the capability sql:unsafe".into(),
            None,
        ))
    });
    let err = ctx.unsafe_sql("select 1").json().await.unwrap_err();
    assert_eq!((err.status(), err.code()), (403, "capability_missing"));
}

fn first_call(seen: &Arc<Mutex<Vec<syscall::Call>>>) -> syscall::Call {
    seen.lock().unwrap().remove(0)
}

#[tokio::test]
async fn search_and_contains_are_text_conditions() {
    let (ctx, seen) = ctx_with(|_| Ok(rows(&[])));
    ctx.db::<Note>()
        .find(Note::title.search("tea -green") & Note::body.contains("50%_off"))
        .all()
        .await
        .unwrap();
    let syscall::Call::DbFind(f) = first_call(&seen) else {
        panic!()
    };
    let Some(Node::And(list)) = f.filter.unwrap().node else {
        panic!()
    };
    let cond = |i: usize| match &list.items[i].node {
        Some(Node::Condition(c)) => c.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        (cond(0).field.as_str(), cond(0).op),
        ("title", Op::Search as i32)
    );
    assert_eq!(
        cond(0).values[0].kind,
        Some(Kind::StringValue("tea -green".into()))
    );
    assert_eq!(
        (cond(1).field.as_str(), cond(1).op),
        ("body", Op::Contains as i32)
    );
    assert_eq!(
        cond(1).values[0].kind,
        Some(Kind::StringValue("50%_off".into()))
    );
}

#[tokio::test]
async fn include_sends_the_navigations_and_reads_what_came_with_the_rows() {
    let tag = |id: i64, label: &str| {
        json!({ "id": id.to_string(), "label": label, "weight": 1, "noteId": "1",
                "createdAt": "2026-11-12T10:00:00Z", "updatedAt": "2026-11-12T10:00:00Z" })
    };
    let (t1, t2) = (tag(10, "a"), tag(11, "b"));
    let (ctx, seen) = ctx_with(move |call| {
        let mut r = pb::DbRows::default();
        match call {
            syscall::Call::DbGet(_) => {
                r.rows = vec![Bytes::from(stored(1, "n", 1).to_string())];
                r.included = vec![Bytes::from(json!({ "tags": [t1, t2] }).to_string())];
            }
            _ => {
                r.rows = vec![
                    Bytes::from(tag(10, "a").to_string()),
                    Bytes::from(tag(12, "c").to_string()),
                ];
                // the second tag's note is not readable: left out
                r.included = vec![
                    Bytes::from(json!({ "note": stored(1, "n", 1) }).to_string()),
                    Bytes::from(json!({}).to_string()),
                ];
            }
        }
        Ok(syscall_result::Result::DbRows(r))
    });
    let note = ctx
        .db::<Note>()
        .get(&Id::new(1))
        .include(Note::tags)
        .await
        .unwrap();
    let tags = note.included(Note::tags).unwrap();
    assert_eq!(
        tags.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    let found = ctx
        .db::<Tag>()
        .find(Filter::all())
        .include(Tag::note)
        .all()
        .await
        .unwrap();
    assert_eq!(found[0].included(Tag::note).unwrap().unwrap().title, "n");
    // the ref field's own handle names the same navigation's rows by its name
    assert!(found[1].included(Tag::note).unwrap().is_none());
    assert!(found[1].included(Tag::note_id).unwrap().is_none());
    // not included: none, and an empty list
    assert!(note.included(Note::tags).is_ok());
    assert!(found[0].included(Tag::note_id).unwrap().is_none());

    let syscall::Call::DbGet(g) = first_call(&seen) else {
        panic!()
    };
    assert_eq!(g.include, ["tags"]);
    assert_eq!(g.lock, None);
    let syscall::Call::DbFind(f) = first_call(&seen) else {
        panic!()
    };
    assert_eq!(f.include, ["note"]);
}

#[tokio::test]
async fn aggregates_count_and_sum_by_group() {
    let (ctx, seen) = ctx_with(|call| {
        let syscall::Call::DbAggregate(a) = call else {
            panic!()
        };
        let group = |k: Value, v: Value| pb::DbGroup {
            key: Bytes::from(k.to_string()),
            value: Bytes::from(v.to_string()),
        };
        let groups = match (a.group_by.as_str(), a.measure) {
            ("", 0) => vec![group(Value::Null, json!("7"))],
            ("", _) => vec![group(Value::Null, json!("12"))],
            (_, 0) => vec![group(json!("a"), json!("2")), group(json!("b"), json!("1"))],
            _ => vec![group(json!("1"), json!("5"))],
        };
        Ok(syscall_result::Result::DbGroups(pb::DbGroups { groups }))
    });
    let db = ctx.db::<Tag>();
    assert_eq!(db.aggregate(Filter::all()).count().await.unwrap(), 7);
    let total: i64 = db
        .aggregate(Tag::weight.gt(0))
        .with_deleted()
        .sum(Tag::weight)
        .await
        .unwrap();
    assert_eq!(total, 12);
    let by_label = db
        .aggregate(Filter::all())
        .group_by(Tag::label)
        .count()
        .await
        .unwrap();
    assert_eq!(by_label, [("a".to_string(), 2), ("b".to_string(), 1)]);
    let by_note = db
        .aggregate(Filter::all())
        .group_by(Tag::note_id)
        .sum(Tag::weight)
        .await
        .unwrap();
    assert_eq!(by_note, [(Id::<Note>::new(1), 5i64)]);

    let calls = seen.lock().unwrap().clone();
    let a = |i: usize| match &calls[i] {
        syscall::Call::DbAggregate(a) => a.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        (
            a(0).entity.as_str(),
            a(0).group_by.as_str(),
            a(0).filter.is_none()
        ),
        ("tag", "", true)
    );
    assert_eq!(
        (a(1).measure, a(1).field.as_str(), a(1).with_deleted),
        (pb::db_aggregate::Measure::Sum as i32, "weight", true)
    );
    assert_eq!(a(2).group_by, "label");
    assert_eq!(
        (a(3).group_by.as_str(), a(3).field.as_str()),
        ("note_id", "weight")
    );
}

#[tokio::test]
async fn update_where_and_upsert() {
    let (ctx, seen) = ctx_with(|call| match call {
        syscall::Call::DbUpdateWhere(_) => Ok(syscall_result::Result::DbCount(pb::DbCountResult {
            count: 3,
        })),
        syscall::Call::DbUpsert(_) => Ok(rows(&[json!({ "id": "4", "label": "x", "weight": 2,
            "noteId": "1", "createdAt": "2026-11-12T10:00:00Z", "updatedAt": "2026-11-12T10:00:00Z" })])),
        _ => Err(("bulk_limit: more than 1000 rows match".into(), None)),
    });
    let n = ctx
        .db::<Tag>()
        .update_where(Tag::weight.lt(2), Patch::new().set(Tag::weight, 2))
        .await
        .unwrap();
    assert_eq!(n, 3);
    let row = ctx
        .db::<Tag>()
        .upsert(
            Tag::label,
            Tag {
                id: Id::none(),
                label: "x".into(),
                weight: 2,
                note_id: Id::new(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(row.weight, 2);
    let calls = seen.lock().unwrap().clone();
    let syscall::Call::DbUpdateWhere(u) = &calls[0] else {
        panic!()
    };
    assert_eq!((u.entity.as_str(), u.request_id), ("tag", 77));
    assert_eq!(json_of(&u.patch), json!({ "weight": 2 }));
    assert!(u.filter.is_some());
    let syscall::Call::DbUpsert(p) = &calls[1] else {
        panic!()
    };
    assert_eq!(p.on, "label");
    assert_eq!(json_of(&p.row)["label"], "x");
}

#[tokio::test]
async fn row_locks_are_reads_of_a_transaction() {
    let (ctx, seen) = ctx_with(|call| match call {
        syscall::Call::TxBegin(_) => Ok(syscall_result::Result::TxBegun(pb::TxBegun { tx: 5 })),
        syscall::Call::DbGet(g)
            if g.lock
                .is_some_and(|l| l.wait == pb::db_lock::Wait::Nowait as i32) =>
        {
            Err(("row_locked: another transaction holds the row".into(), None))
        }
        syscall::Call::DbGet(_) | syscall::Call::DbFind(_) => Ok(rows(&[stored(1, "a", 1)])),
        syscall::Call::DbCount(_) => Ok(syscall_result::Result::DbCount(pb::DbCountResult {
            count: 1,
        })),
        _ => Ok(syscall_result::Result::Ok(pb::Empty {})),
    });
    let err = ctx
        .transaction(async |tx| {
            tx.db::<Note>().get(&Id::new(1)).for_update().await?;
            tx.db::<Note>()
                .find(Note::title.eq("a".into()))
                .for_update()
                .skip_locked()
                .limit(10)
                .page()
                .await?;
            tx.db::<Note>()
                .find(Filter::all())
                .for_share()
                .first()
                .await?;
            // the repository's other calls are there too (it derefs to the plain one)
            tx.db::<Note>().count().await?;
            tx.db::<Note>().get(&Id::new(2)).for_update().nowait().await
        })
        .await
        .unwrap_err();
    assert_eq!((err.status(), err.code()), (409, "row_locked"));
    let calls = seen.lock().unwrap().clone();
    let locks: Vec<(u64, Option<pb::DbLock>)> = calls
        .iter()
        .filter_map(|c| match c {
            syscall::Call::DbGet(g) => Some((g.tx, g.lock)),
            syscall::Call::DbFind(f) => Some((f.tx, f.lock)),
            _ => None,
        })
        .collect();
    let l = |s: pb::db_lock::Strength, w: pb::db_lock::Wait| {
        Some(pb::DbLock {
            strength: s as i32,
            wait: w as i32,
        })
    };
    use pb::db_lock::{Strength, Wait};
    assert_eq!(
        locks,
        [
            (5, l(Strength::ForUpdate, Wait::Wait)),
            (5, l(Strength::ForUpdate, Wait::SkipLocked)),
            (5, l(Strength::ForShare, Wait::Wait)),
            (5, l(Strength::ForUpdate, Wait::Nowait)),
        ]
    );
    // outside a transaction a read sends no lock (and has no lock methods: see `Tx`)
    ctx.db::<Note>().get(&Id::new(1)).await.unwrap();
    let last = seen.lock().unwrap().last().cloned().unwrap();
    let syscall::Call::DbGet(g) = last else {
        panic!()
    };
    assert_eq!((g.tx, g.lock), (0, None));
}
