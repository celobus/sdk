use serde_json::{Value, json};

use super::*;

fn field(name: &str, n: u32, ty: &str) -> Value {
    json!({ "name": name, "number": n, "type": ty, "repeated": false, "optional": false, "description": "" })
}

fn col(source: &str, field: &str) -> Value {
    json!({ "column": { "source": source, "field": field } })
}

/// `shop`: orders and their lines, a customer read from `crm`, and the view `sales` over them.
fn shop() -> Value {
    json!({
      "schema": 1, "id": "shop", "version": "1.0.0",
      "dependencies": [{ "plugin": "crm", "calls": [], "reads": [
          { "entity": "customer", "fields": [{ "name": "id", "type": "ref:crm.customer" }, { "name": "segment", "type": "string" },
                                             { "name": "tags", "type": "repeated string" }] }] }],
      "enums": [{ "name": "OrderStatus", "values": ["placed", "paid"] }],
      "messages": [
        { "name": "Sales", "kind": "message", "description": "",
          "fields": [field("day", 1, "date"), field("clerk", 2, "ref:platform.user"), field("segment", 3, "string"),
                     field("amount", 4, "decimal")] },
        { "name": "ByMonth", "kind": "message", "description": "",
          "fields": [field("month", 1, "date"), field("total", 2, "decimal"), field("lines", 3, "int64")] }
      ],
      "entities": [
        { "name": "order", "message": "Order", "key": "uuid", "label": "id", "soft_delete": false, "version": false,
          "append_only": false, "fields": [
            field("id", 1, "ref:shop.order"), field("status", 2, "enum:OrderStatus"),
            field("created_by", 3, "ref:platform.user"), field("customer_id", 4, "ref:crm.customer"),
            field("paid_at", 5, "timestamp"),
            { "name": "note", "number": 6, "type": "string", "repeated": false, "optional": true, "description": "",
              "encrypted": true }] },
        { "name": "line", "message": "Line", "key": "uuid", "label": "id", "soft_delete": false, "version": false,
          "append_only": false, "fields": [
            field("id", 1, "ref:shop.line"), field("order_id", 2, "ref:shop.order"), field("amount", 3, "decimal")] }
      ],
      "endpoints": [],
      "views": [
        { "name": "sales", "message": "Sales", "description": "One row per paid line.",
          "sources": [
            { "entity": "line", "alias": "l" },
            { "entity": "order", "alias": "o", "join": "inner",
              "left": { "source": "l", "field": "order_id" }, "right": { "source": "o", "field": "id" } },
            { "entity": "crm.customer", "alias": "c", "join": "left",
              "left": { "source": "o", "field": "customer_id" }, "right": { "source": "c", "field": "id" } }
          ],
          "filter": { "and": [
            { "compare": { "left": col("o", "status"), "op": "eq", "values": ["paid"] } },
            { "compare": { "left": col("o", "paid_at"), "op": "ge", "right": { "days_ago": 365 } } }
          ] },
          "columns": [
            { "name": "day", "value": { "column": { "source": "o", "field": "paid_at", "bucket": "day" } } },
            { "name": "clerk", "value": col("o", "created_by") },
            { "name": "segment", "value": col("c", "segment") },
            { "name": "amount", "value": col("l", "amount") }
          ] },
        { "name": "by_month", "message": "ByMonth", "description": "",
          "sources": [{ "entity": "sales", "alias": "s" }],
          "group_by": [{ "source": "s", "field": "day", "bucket": "month" }],
          "columns": [
            { "name": "month", "value": { "column": { "source": "s", "field": "day", "bucket": "month" } } },
            { "name": "total", "value": { "aggregate": { "fn": "sum", "column": { "source": "s", "field": "amount" } } } },
            { "name": "lines", "value": { "aggregate": { "fn": "count" } } }
          ] }
      ]
    })
}

fn codes(v: &Value) -> Vec<(&'static str, String)> {
    match check(v.to_string().as_bytes()) {
        Ok(_) => vec![],
        Err(e) => e.into_iter().map(|e| (e.code, e.path)).collect(),
    }
}

fn after(f: impl FnOnce(&mut Value)) -> Vec<&'static str> {
    let mut v = shop();
    f(&mut v);
    codes(&v).into_iter().map(|(c, _)| c).collect()
}

#[test]
fn a_view_over_entities_a_dependency_and_another_view_checks() {
    assert_eq!(codes(&shop()), vec![]);
    // the views come back as they were declared, and the order puts a view after those it reads
    let d = parse(shop().to_string().as_bytes()).unwrap();
    assert_eq!(serde_json::to_value(&d).unwrap()["views"], shop()["views"]);
    let order: Vec<&str> = view::ordered(&d.views)
        .unwrap()
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(order, ["sales", "by_month"]);
    // no views: none written (a descriptor without them hashes as before)
    let mut plain = shop();
    plain.as_object_mut().unwrap().remove("views");
    let d = parse(plain.to_string().as_bytes()).unwrap();
    assert!(serde_json::to_value(&d).unwrap().get("views").is_none());
}

#[test]
fn what_cannot_work_is_refused() {
    // an unknown entity, an unknown column, an undeclared read
    assert_eq!(
        after(|v| v["views"][0]["sources"][0]["entity"] = json!("nope")),
        ["E_VIEW_SOURCE"]
    );
    assert!(
        after(|v| v["views"][0]["columns"][3]["value"] = col("l", "price"))
            .contains(&"E_VIEW_COLUMN")
    );
    assert!(
        after(|v| v["views"][0]["sources"][2]["entity"] = json!("crm.lead"))
            .contains(&"E_VIEW_SOURCE")
    );
    // a join of two types
    assert_eq!(
        after(|v| v["views"][0]["sources"][1]["left"]["field"] = json!("amount")),
        ["E_VIEW_JOIN"]
    );
    // a plain join of one type is fine; a list is not joined
    assert_eq!(
        after(|v| {
            v["views"][0]["sources"][2]["left"]["field"] = json!("status");
            v["views"][0]["sources"][2]["right"]["field"] = json!("segment");
        }),
        ["E_VIEW_JOIN"]
    );
    assert_eq!(
        after(|v| v["views"][0]["sources"][2]["right"]["field"] = json!("tags")),
        ["E_VIEW_JOIN"]
    );
    // a cycle between views
    assert_eq!(
        after(|v| {
            v["views"][0]["sources"][0] = json!({ "entity": "by_month", "alias": "l" });
        }),
        ["E_VIEW_CYCLE"]
    );
    // a name an entity has
    assert!(after(|v| v["views"][1]["name"] = json!("order")).contains(&"E_VIEW_CLASH"));
    assert!(after(|v| v["views"][1]["name"] = json!("x_with_deleted")).contains(&"E_VIEW_NAME"));
    // a bucket of a string, a column of the wrong type, a missing column, an encrypted field
    assert_eq!(
        after(|v| v["views"][0]["columns"][0]["value"]["column"]["field"] = json!("status")),
        ["E_VIEW_BUCKET"]
    );
    assert_eq!(
        after(|v| v["views"][0]["columns"][1]["value"] = col("o", "status")),
        ["E_VIEW_TYPE"]
    );
    assert_eq!(
        after(|v| {
            v["views"][0]["columns"].as_array_mut().unwrap().pop();
        }),
        ["E_VIEW_COLUMN"]
    );
    assert!(
        after(|v| v["views"][0]["filter"] =
            json!({ "compare": { "left": col("o", "note"), "op": "is_null" } }))
        .contains(&"E_VIEW_ENCRYPTED")
    );
    // a grouped view: every column grouped or aggregated, every group a column
    assert_eq!(
        after(|v| v["views"][1]["group_by"][0]["bucket"] = json!("year")),
        ["E_VIEW_GROUP", "E_VIEW_GROUP"]
    );
    // a search, an unknown operator, days_ago against a string
    assert!(
        after(|v| v["views"][0]["filter"]["and"][0]["compare"]["op"] = json!("search"))
            .contains(&"E_VIEW_FILTER")
    );
    assert_eq!(
        after(|v| v["views"][0]["filter"]["and"][1]["compare"]["left"] = col("o", "status")),
        ["E_VIEW_TYPE"]
    );
    // a message that is not there
    assert_eq!(
        after(|v| v["views"][1]["message"] = json!("Nope")),
        ["E_VIEW_MESSAGE"]
    );
}

#[test]
fn views_read_through_at_most_three_views() {
    let mut v = shop();
    let views = v["views"].as_array_mut().unwrap();
    for (i, from) in ["by_month", "v1", "v2"].iter().enumerate() {
        views.push(json!({ "name": format!("v{}", i + 1), "message": "ByMonth",
            "sources": [{ "entity": from, "alias": "s" }],
            "columns": [{ "name": "month", "value": col("s", "month") }, { "name": "total", "value": col("s", "total") },
                        { "name": "lines", "value": col("s", "lines") }] }));
    }
    assert_eq!(codes(&v), vec![("E_VIEW_DEPTH", "views[v3]".to_string())]);
}

#[test]
fn a_decimal_value_is_a_string_or_a_whole_number() {
    let amount = |values: Value| {
        after(|v| {
            v["views"][0]["filter"]["and"].as_array_mut().unwrap().push(
                json!({ "compare": { "left": col("l", "amount"), "op": "in", "values": values } }),
            )
        })
    };
    assert!(amount(json!(["12345678901234567.89", 12, "-0.5"])).is_empty());
    // a fraction (or a number past 64 bits) is read as a binary floating point: another number than written
    for bad in [
        json!([12.5]),
        json!(["1", 12345678901234567.89]),
        json!([1e3]),
    ] {
        assert_eq!(amount(bad.clone()), ["E_VIEW_FILTER"], "{bad}");
    }
    let past = serde_json::from_str::<Value>("[123456789012345678901234]").unwrap();
    assert_eq!(amount(past), ["E_VIEW_FILTER"]);
    // a having over a decimal sum alike
    assert_eq!(
        after(|v| v["views"][1]["having"] = json!({ "compare": {
            "left": { "aggregate": { "fn": "sum", "column": { "source": "s", "field": "amount" } } },
            "op": "gt", "values": [0.1] } })),
        ["E_VIEW_FILTER"]
    );
}

#[test]
fn days_ago_stays_within_a_hundred_years() {
    let days = |n: i64| {
        after(|v| v["views"][0]["filter"]["and"][1]["compare"]["right"]["days_ago"] = json!(n))
    };
    assert!(days(36_500).is_empty());
    assert!(days(-36_500).is_empty());
    assert_eq!(days(36_501), ["E_VIEW_TERM"]);
    assert_eq!(days(-2_000_000_000), ["E_VIEW_TERM"]);
}

/// Four views, each an `and` of `n` exists over the next one: a small descriptor whose views would compile
/// (references per level)^depth times.
pub(crate) fn fan_out(mut d: Value, n: usize) -> Value {
    d["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "One", "kind": "message", "description": "", "fields": [field("amount", 1, "decimal")] }));
    let views = d["views"].as_array_mut().unwrap();
    for i in 1..=4 {
        let mut v = json!({ "name": format!("f{i}"), "message": "One", "description": "",
            "sources": [{ "entity": "line", "alias": "l" }],
            "columns": [{ "name": "amount", "value": col("l", "amount") }] });
        if i < 4 {
            let all: Vec<Value> = (0..n)
                .map(|k| json!({ "exists": { "entity": format!("f{}", i + 1), "alias": format!("x{k}") } }))
                .collect();
            v["filter"] = json!({ "and": all });
        }
        views.push(v);
    }
    d
}

#[test]
fn a_view_s_size_is_bounded() {
    // the fan-out of the review: refused at once, not compiled
    let d = fan_out(shop(), 200);
    let bytes = d.to_string().len();
    assert!(bytes < 64 << 10, "{bytes}");
    let started = std::time::Instant::now();
    let got = codes(&d);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert!(
        got.iter()
            .any(|(c, p)| *c == "E_VIEW_SOURCE" && p == "views[f1].sources"),
        "{got:?}"
    );
    // exists count toward the sources: seven and the root pass, eight and the root do not
    assert_eq!(codes(&fan_out(shop(), 7)), vec![]);
    assert_eq!(
        codes(&fan_out(shop(), 8))
            .into_iter()
            .map(|(c, _)| c)
            .collect::<Vec<_>>(),
        ["E_VIEW_SOURCE"; 3]
    );
    // nodes: an or of 300 comparisons
    let wide = after(|v| {
        let all: Vec<Value> = (0..300)
            .map(
                |k| json!({ "compare": { "left": col("l", "amount"), "op": "eq", "values": [k] } }),
            )
            .collect();
        v["views"][0]["filter"] = json!({ "or": all });
    });
    assert_eq!(wide, ["E_VIEW_SIZE"]);
    // views per plugin
    let many = after(|v| {
        let views = v["views"].as_array_mut().unwrap();
        for k in 0..view::MAX_VIEWS {
            let mut x = views[1].clone();
            x["name"] = json!(format!("m{k}"));
            views.push(x);
        }
    });
    assert_eq!(many, ["E_VIEW_SIZE"]);
}

#[test]
fn a_text_value_holds_no_nul() {
    assert_eq!(
        after(|v| v["views"][0]["filter"]["and"][0]["compare"]["values"] = json!(["pa\u{0}id"])),
        ["E_VIEW_FILTER"]
    );
}

#[test]
fn the_checks_hold_a_view_to_the_query_s_edges() {
    // a whole number past a signed 64-bit integer is not exact for a decimal
    let amount = |values: Value| {
        after(|v| {
            v["views"][0]["filter"]["and"].as_array_mut().unwrap().push(
                json!({ "compare": { "left": col("l", "amount"), "op": "in", "values": values } }),
            )
        })
    };
    assert!(amount(json!([9_223_372_036_854_775_807i64])).is_empty());
    assert_eq!(
        amount(json!([9_223_372_036_854_775_808u64])),
        ["E_VIEW_FILTER"]
    );
    // sixteen levels of conditions, as a query has; seventeen are refused
    let nested = |levels: usize| {
        let mut e =
            json!({ "compare": { "left": col("o", "status"), "op": "eq", "values": ["paid"] } });
        for _ in 1..levels {
            e = json!({ "not": e });
        }
        after(|v| v["views"][0]["filter"] = e)
    };
    assert!(nested(16).is_empty(), "{:?}", nested(16));
    assert_eq!(nested(17), ["E_VIEW_FILTER"]);
}
