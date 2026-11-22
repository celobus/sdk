//! The query builder: the tree it sends (the conformance query, as the three SDKs build it), how rows come back typed,
//! and the reads' modes, against a fake platform.

use celobus_proto as pb;
use pb::{query_expr::Node, query_read::Mode, query_term, syscall, syscall_result};
use serde_json::{Map, Value, json};

use crate::db_tests::{ctx_with, rows};
use crate::platform::{self, RoleAssignment};
use crate::prelude::*;
use crate::query::{Label, alias, count, exists, label, sum};

#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum OrderStatus {
    Placed,
    Paid,
}

/// `crm.customer`, as `celobus plugin add crm` generates it.
#[derive(Entity, Clone, Debug, PartialEq)]
#[entity(plugin = "crm", key = uuid)]
pub struct Customer {
    pub id: Id<Customer>,
    pub name: String,
    pub owner_id: Id<platform::User>,
}
impl crate::sql::Exported for Customer {}

#[derive(Entity, Clone, Debug, PartialEq)]
pub struct Order {
    pub id: Id<Order>,
    pub number: String,
    pub status: OrderStatus,
    pub total: Decimal,
    pub created_by: Id<platform::User>,
    pub customer_id: Option<Id<Customer>>,
}

#[derive(Entity, Clone, Debug, PartialEq)]
pub struct OrderLine {
    pub id: Id<OrderLine>,
    pub order_id: Id<Order>,
    pub sku: String,
    pub amount: Decimal,
}

/// The conformance query (`sdk/conformance/query/revenue.json`): the paid lines of orders over 10.50 placed by a
/// clerk, per clerk and customer (its id and name: C#'s LINQ selects a group's key), with the clerk's label, the
/// customer's name, the revenue and the lines.
fn revenue(ctx: &Ctx) -> crate::query::Selected<(Option<Label>, Option<String>, Decimal, i64)> {
    ctx.query::<OrderLine>()
        .join::<Order>(OrderLine::order_id)
        .left_join::<Customer>(Order::customer_id)
        .filter(Order::status.eq(OrderStatus::Paid) & Order::total.ge("10.50".parse().unwrap()))
        .filter(exists::<RoleAssignment>(
            RoleAssignment::user_id.eq_col(Order::created_by)
                & RoleAssignment::role.eq("shop.clerk".into()),
        ))
        .group_by((Order::created_by, Customer::id, Customer::name))
        .having(count().gt(1))
        .select((
            label(Order::created_by),
            Customer::name,
            sum(OrderLine::amount),
            count(),
        ))
        .order_by(sum(OrderLine::amount).desc())
        .limit(20)
}

// --- the tree as protobuf JSON (what the TypeScript and C# SDKs compare with the same file) -------------------

fn obj(pairs: Vec<(&str, Option<Value>)>) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        if let Some(v) = v {
            m.insert(k.into(), v);
        }
    }
    Value::Object(m)
}

fn column(c: &pb::QueryColumn) -> Value {
    json!({ "source": c.source, "field": c.field })
}

fn aggregate(a: &pb::QueryAggregate) -> Value {
    let f = pb::query_aggregate::Fn::try_from(a.r#fn).unwrap();
    obj(vec![
        ("fn", (a.r#fn != 0).then(|| json!(f.as_str_name()))),
        ("column", a.column.as_ref().map(column)),
    ])
}

fn term(t: &pb::QueryTerm) -> Value {
    match t.kind.as_ref().unwrap() {
        query_term::Kind::Column(c) => json!({ "column": column(c) }),
        query_term::Kind::Aggregate(a) => json!({ "aggregate": aggregate(a) }),
    }
}

fn value(v: &pb::DbValue) -> Value {
    use pb::db_value::Kind;
    match v.kind.as_ref().unwrap() {
        Kind::StringValue(s) => json!({ "stringValue": s }),
        Kind::IntValue(i) => json!({ "intValue": i.to_string() }),
        Kind::DoubleValue(d) => json!({ "doubleValue": d }),
        Kind::BoolValue(b) => json!({ "boolValue": b }),
        Kind::BytesValue(_) => json!({ "bytesValue": "" }),
    }
}

fn source(s: &pb::QuerySource) -> Value {
    let j = pb::query_source::Join::try_from(s.join).unwrap();
    obj(vec![
        ("entity", Some(json!(s.entity))),
        ("alias", Some(json!(s.alias))),
        ("join", (s.join != 0).then(|| json!(j.as_str_name()))),
        ("left", s.left.as_ref().map(column)),
        ("right", s.right.as_ref().map(column)),
    ])
}

fn expr(e: &pb::QueryExpr) -> Value {
    match e.node.as_ref().unwrap() {
        Node::And(l) => json!({ "and": { "items": l.items.iter().map(expr).collect::<Vec<_>>() } }),
        Node::Or(l) => json!({ "or": { "items": l.items.iter().map(expr).collect::<Vec<_>>() } }),
        Node::Not(i) => json!({ "not": expr(i) }),
        Node::Exists(x) => json!({ "exists": obj(vec![
            ("source", x.source.as_ref().map(source)),
            ("filter", x.filter.as_deref().map(expr)),
        ]) }),
        Node::Compare(c) => {
            let op = pb::db_condition::Op::try_from(c.op).unwrap();
            json!({ "compare": obj(vec![
                ("left", c.left.as_ref().map(term)),
                ("op", Some(json!(op.as_str_name()))),
                ("values", (!c.values.is_empty()).then(|| json!(c.values.iter().map(value).collect::<Vec<_>>()))),
                ("right", c.right.as_ref().map(term)),
            ]) })
        }
    }
}

pub(crate) fn tree_json(q: &pb::Query) -> Value {
    use pb::query_select::Kind;
    let list = |v: Vec<Value>| (!v.is_empty()).then_some(Value::Array(v));
    obj(vec![
        ("sources", list(q.sources.iter().map(source).collect())),
        ("filter", q.filter.as_ref().map(expr)),
        ("groupBy", list(q.group_by.iter().map(column).collect())),
        ("having", q.having.as_ref().map(expr)),
        (
            "select",
            list(
                q.select
                    .iter()
                    .map(|s| match s.kind.as_ref().unwrap() {
                        Kind::Column(c) => json!({ "column": column(c) }),
                        Kind::Aggregate(a) => json!({ "aggregate": aggregate(a) }),
                        Kind::Label(c) => json!({ "label": column(c) }),
                    })
                    .collect(),
            ),
        ),
        (
            "order",
            list(
                q.order
                    .iter()
                    .map(|o| {
                        obj(vec![
                            ("term", o.term.as_ref().map(term)),
                            ("desc", o.desc.then_some(json!(true))),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("limit", (q.limit != 0).then(|| json!(q.limit))),
    ])
}

#[test]
fn the_conformance_query_is_the_shared_tree() {
    let ctx = Ctx::test("shop", "1.0.0");
    let q = revenue(&ctx);
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../sdk/conformance/query/revenue.json"
    ))
    .unwrap();
    assert_eq!(tree_json(q.tree()), expected);
}

#[tokio::test]
async fn rows_come_back_typed_and_reads_say_their_mode() {
    let clerk = "0190f3c2-0000-7000-8000-000000000011";
    let (ctx, seen) = ctx_with(move |call| {
        match call {
        syscall::Call::Query(r) if r.mode == Mode::Count as i32 => {
            Ok(syscall_result::Result::DbCount(pb::DbCountResult { count: 2 }))
        }
        syscall::Call::Query(_) => Ok(syscall_result::Result::DbRows(pb::DbRows {
            rows: [
                json!([{ "id": clerk, "text": "Sarah", "avatarUrl": "https://a" }, "Ada", "30.00", "3"]),
                json!([null, null, "0", "2"]),
            ]
            .iter()
            .map(|v| bytes::Bytes::from(v.to_string()))
            .collect(),
            next_page_token: "next".into(),
            included: vec![],
        })),
        _ => Err(("invalid: no".into(), None)),
    }
    });
    let q = revenue(&ctx);
    let got = q.all().await.unwrap();
    assert_eq!(got.len(), 2);
    let (who, customer, revenue, lines) = &got[0];
    assert_eq!(
        who,
        &Some(Label {
            id: clerk.into(),
            text: "Sarah".into(),
            avatar_url: Some("https://a".into())
        })
    );
    assert_eq!(customer.as_deref(), Some("Ada"));
    assert_eq!(revenue.to_string(), "30.00");
    assert_eq!(*lines, 3);
    assert_eq!(got[1].0, None);
    assert_eq!(got[1].1, None);
    let page = q.page(Some("t1")).await.unwrap();
    assert_eq!(page.next.as_deref(), Some("next"));
    assert!(q.first().await.unwrap().is_some());
    assert_eq!(q.count().await.unwrap(), 2);
    let calls = seen.lock().unwrap().clone();
    let reads: Vec<(i32, String, u64)> = calls
        .iter()
        .map(|c| match c {
            syscall::Call::Query(r) => (r.mode, r.page_token.clone(), r.request_id),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        reads,
        [
            (Mode::All as i32, String::new(), 77),
            (Mode::Page as i32, "t1".into(), 77),
            (Mode::First as i32, String::new(), 77),
            (Mode::Count as i32, String::new(), 77),
        ]
    );
    // the source location of the query: this file
    let syscall::Call::Query(r) = &calls[0] else {
        panic!()
    };
    assert!(r.source.contains("query/tests.rs:"), "{}", r.source);
}

#[test]
fn joins_go_either_way_and_aliases_name_an_entity_twice() {
    let ctx = Ctx::test("shop", "1.0.0");
    // reverse: the query has the order, the line's ref joins the line
    let q = ctx
        .query::<Order>()
        .join::<OrderLine>(OrderLine::order_id)
        .select((Order::number, OrderLine::sku));
    let t = tree_json(q.tree());
    assert_eq!(
        t["sources"][1],
        json!({ "entity": "order_line", "alias": "order_line", "join": "INNER",
                "left": { "source": "order", "field": "id" }, "right": { "source": "order_line", "field": "order_id" } })
    );
    // the directory twice: who placed the order, who owns its customer (left: a column of Option)
    let owner = alias::<platform::User>("owner").left();
    let q = ctx
        .query::<Order>()
        .join::<platform::User>(Order::created_by)
        .left_join::<Customer>(Order::customer_id)
        .join_as(&owner, Customer::owner_id)
        .select((
            platform::User::display_name,
            owner.col(platform::User::display_name),
        ));
    let t = tree_json(q.tree());
    assert_eq!(t["sources"][1]["alias"], "platform_user");
    assert_eq!(
        t["sources"][3],
        json!({ "entity": "platform.user", "alias": "owner", "join": "LEFT",
                "left": { "source": "crm_customer", "field": "owner_id" }, "right": { "source": "owner", "field": "id" } })
    );
    assert_eq!(
        t["select"][1],
        json!({ "column": { "source": "owner", "field": "display_name" } })
    );
    // join_on: two refs to one entity
    let q = ctx
        .query::<Order>()
        .join_on::<Customer>(Customer::owner_id.eq_col(Order::created_by))
        .select(Order::number);
    assert_eq!(
        tree_json(q.tree())["sources"][1],
        json!({ "entity": "crm.customer", "alias": "crm_customer", "join": "INNER",
                "left": { "source": "order", "field": "created_by" }, "right": { "source": "crm_customer", "field": "owner_id" } })
    );
}

#[tokio::test]
async fn a_join_the_query_cannot_place_is_refused_before_it_is_sent() {
    let (ctx, seen) = ctx_with(|_| Err(("invalid: no".into(), None)));
    // the query has no order to join a customer to
    let err = ctx
        .query::<OrderLine>()
        .join::<Customer>(Order::customer_id)
        .count()
        .await
        .unwrap_err();
    assert_eq!((err.status(), err.code()), (500, "query_invalid"));
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_cells_query_refusals_come_back_typed() {
    let (ctx, _) = ctx_with(|_| Err(("query_join_limit: at most 8".into(), None)));
    let err = ctx.query::<Order>().count().await.unwrap_err();
    assert_eq!((err.status(), err.code()), (422, "query_join_limit"));
    let (ctx, _) = ctx_with(|_| Err(("query_invalid: order has no column \"x\"".into(), None)));
    let err = ctx.query::<Order>().all().await.unwrap_err();
    assert_eq!((err.status(), err.code()), (500, "query_invalid"));
    // without select, the root's rows
    let (ctx, _) = ctx_with(|_| {
        Ok(rows(&[
            json!({ "id": "0190f3c2-0000-7000-8000-0000000000aa", "number": "SO-1", "status": "paid",
                          "total": "1", "createdBy": "0190f3c2-0000-7000-8000-000000000011",
                          "createdAt": "2026-11-12T10:00:00.000000Z", "updatedAt": "2026-11-12T10:00:00.000000Z" }),
        ]))
    });
    let orders = ctx
        .query::<Order>()
        .filter(Order::status.eq(OrderStatus::Paid))
        .all()
        .await
        .unwrap();
    assert_eq!(orders[0].number, "SO-1");
    assert_eq!(orders[0].customer_id, None);
}
