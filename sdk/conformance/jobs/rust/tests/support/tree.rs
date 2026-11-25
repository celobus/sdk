//! A query tree (`celobus.v1.Query`) and a filter (`DbFilter`) as protobuf JSON: what the TypeScript and C# SDKs
//! compare with the same files.

#![allow(dead_code)]

use celobus_proto as pb;
use pb::{query_expr::Node, query_term};
use serde_json::{Map, Value, json};

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
    let mut v = json!({ "source": c.source, "field": c.field });
    if c.bucket != 0 {
        let b = pb::query_column::Bucket::try_from(c.bucket).unwrap();
        v["bucket"] = json!(b.as_str_name());
    }
    if c.parse != 0 {
        let p = pb::query_column::Parse::try_from(c.parse).unwrap();
        v["parse"] = json!(p.as_str_name());
    }
    v
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
        query_term::Kind::DaysAgo(n) => json!({ "daysAgo": n }),
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
        ("history", s.history.then_some(json!(true))),
        ("eachDay", s.each_day.then_some(json!(true))),
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

pub fn tree_json(q: &pb::Query) -> Value {
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

/// A filter of the plugin's repository calls (`DbFilter`).
pub fn filter_json(f: &pb::DbFilter) -> Value {
    use pb::db_filter::Node;
    match f.node.as_ref().unwrap() {
        Node::And(l) => {
            json!({ "and": { "items": l.items.iter().map(filter_json).collect::<Vec<_>>() } })
        }
        Node::Or(l) => {
            json!({ "or": { "items": l.items.iter().map(filter_json).collect::<Vec<_>>() } })
        }
        Node::Not(i) => json!({ "not": filter_json(i) }),
        Node::Condition(c) => {
            let op = pb::db_condition::Op::try_from(c.op).unwrap();
            json!({ "condition": obj(vec![
                ("field", Some(json!(c.field))),
                ("op", Some(json!(op.as_str_name()))),
                ("values", (!c.values.is_empty()).then(|| json!(c.values.iter().map(value).collect::<Vec<_>>()))),
            ]) })
        }
    }
}
