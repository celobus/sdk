use serde_json::{Value, json};

use crate::ui_check::{requirements, ui_warnings};
use crate::*;

fn f(name: &str, n: u32, ty: &str) -> Value {
    json!({ "name": name, "number": n, "type": ty })
}

fn rep(name: &str, n: u32, ty: &str) -> Value {
    json!({ "name": name, "number": n, "type": ty, "repeated": true })
}

/// A shop with a list screen, a detail screen, an overlay form, a badge and a topic.
fn shop(ui: Value) -> Value {
    json!({
      "schema": 1, "id": "shop", "version": "1.0.0",
      "roles": [{ "name": "clerk" }],
      "enums": [{ "name": "OrderStatus", "values": ["placed", "paid"] }],
      "messages": [
        { "name": "Empty" },
        { "name": "OrderKey", "fields": [f("id", 1, "uuid")] },
        { "name": "Order", "fields": [f("id", 1, "uuid"), f("number", 2, "string"), f("total", 3, "decimal"),
                                       f("currency", 4, "string"), f("status", 5, "enum:OrderStatus")] },
        { "name": "ListOrdersRequest", "fields": [f("status", 1, "enum:OrderStatus"), f("page_token", 2, "string"),
                                                   f("page_size", 3, "int32"), f("sort_total", 4, "string")] },
        { "name": "ListOrdersResponse", "fields": [rep("orders", 1, "message:Order"), f("next_page_token", 2, "string")] },
        { "name": "GetOrderRequest", "fields": [f("id", 1, "uuid")] },
        { "name": "CancelOrderRequest", "fields": [f("id", 1, "uuid"), f("reason", 2, "string")] },
        { "name": "CountResponse", "fields": [f("count", 1, "int64")] }
      ],
      "endpoints": [
        { "name": "ListOrders", "input": "ListOrdersRequest", "output": "ListOrdersResponse", "scopes": ["read"], "read_only": true, "ui": true },
        { "name": "GetOrder", "input": "GetOrderRequest", "output": "Order", "scopes": ["read"], "read_only": true, "ui": true },
        { "name": "CancelOrder", "input": "CancelOrderRequest", "output": "Order", "ui": true, "idempotency": "optional",
          "invalidates": [{ "topic": "orders" }, { "topic": "order", "key": [{ "field": "id", "from": "id" }] }] },
        { "name": "CountOpen", "input": "Empty", "output": "CountResponse", "scopes": ["read"], "read_only": true, "ui": true },
        { "name": "Internal", "input": "Empty", "output": "CountResponse", "scopes": ["read"], "read_only": true }
      ],
      "topics": [{ "name": "orders", "key": "Empty", "roles": ["clerk"] },
                 { "name": "order", "key": "OrderKey", "authorize": "GetOrder" }],
      "ui": ui
    })
}

fn good_ui() -> Value {
    json!({
      "catalog": 1,
      "texts": { "en": { "nav.all": "All orders" } },
      "queries": [{ "name": "open_count", "endpoint": "CountOpen", "follow": [{ "topic": "orders" }] }],
      "nav": [{ "title": "Orders", "icon": "shopping-bag", "entries": [
          { "title": { "key": "nav.all" }, "screen": "orders", "badge": { "data": { "query": "open_count", "field": "count" } } }] }],
      "commands": [{ "id": "find", "title": "Find an order", "target": { "screen": "orders" } }],
      "screens": [
        { "id": "orders", "title": "Orders",
          "params": [{ "name": "status", "type": "enum:OrderStatus" }],
          "queries": [{ "name": "orders", "endpoint": "ListOrders", "paged": true, "rows": "orders",
                        "args": [{ "field": "status", "value": { "param": "status" } }], "follow": [{ "topic": "orders" }] }],
          "actions": [
            { "name": "open", "steps": [{ "navigate": { "screen": "order", "args": [{ "field": "id", "value": { "row": "id" } }] } }] },
            { "name": "cancel", "steps": [
                { "confirm": { "title": "Cancel this order?", "danger": true } },
                { "call": { "endpoint": "CancelOrder", "args": [{ "field": "id", "value": { "row": "id" } }] } },
                { "toast": { "text": "Order cancelled" } }],
              "on_error": [{ "toast": { "error": true } }] }],
          "layout": { "stack": { "children": [
            { "filter_bar": { "inputs": [{ "name": "status", "control": "select", "label": "Status",
                                             "options": [{ "value": "placed", "label": "Placed" }] }] } },
            { "table": { "query": "orders", "open": "open",
                "columns": [{ "field": "number", "title": "Order" },
                            { "field": "total", "title": "Total", "format": { "money": { "currency": "currency" } } },
                            { "field": "status", "title": "Status", "format": { "status": { "tones": { "placed": "attention" } } } }],
                "row_actions": [{ "button": { "label": "Cancel", "action": "cancel", "emphasis": "danger" },
                                  "visible": { "eq": [{ "row": "status" }, { "lit": "placed" }] } }] } }
          ] } } },
        { "id": "order", "title": { "data": { "query": "order", "field": "number" } },
          "params": [{ "name": "id", "type": "uuid", "required": true }],
          "queries": [{ "name": "order", "endpoint": "GetOrder", "args": [{ "field": "id", "value": { "param": "id" } }],
                        "follow": [{ "topic": "order", "key": [{ "field": "id", "value": { "param": "id" } }] }] }],
          "actions": [
            { "name": "edit", "steps": [{ "open": { "overlay": "cancel_form", "args": [{ "field": "id", "value": { "param": "id" } }] } }] }],
          "overlays": [{ "id": "cancel_form", "kind": "dialog", "title": "Cancel order",
              "params": [{ "name": "id", "type": "uuid" }],
              "actions": [{ "name": "submit", "steps": [{ "call": { "endpoint": "CancelOrder", "form": true,
                                                                     "args": [{ "field": "id", "value": { "param": "id" } }] } },
                                                         { "toast": { "text": "Done" } }, { "close": {} }] }],
              "layout": { "form": { "endpoint": "CancelOrder", "submit": "submit", "submit_label": "Cancel order",
                  "sections": [{ "inputs": [{ "name": "reason", "control": "textarea", "label": "Why", "required": true }] }] } } }],
          "header": [{ "button": { "label": "Cancel order", "action": "edit", "emphasis": "danger" } }],
          "layout": { "fields": { "items": [
              { "label": "Total", "value": { "data": { "query": "order", "field": "total" } }, "format": { "money": { "code": "USD" } } },
              { "label": "Status", "value": { "data": { "query": "order", "field": "status" } } }] } } }
      ]
    })
}

fn errors(ui: Value) -> Vec<(String, String)> {
    let d: Descriptor = serde_json::from_value(shop(ui)).expect("parses");
    validate(&d)
        .into_iter()
        .map(|e| (e.code.to_string(), e.path))
        .collect()
}

fn codes(ui: Value) -> Vec<String> {
    errors(ui).into_iter().map(|(c, _)| c).collect()
}

#[test]
fn a_whole_document_validates() {
    assert_eq!(errors(good_ui()), vec![]);
    let d: Descriptor = serde_json::from_value(shop(good_ui())).unwrap();
    // it round-trips through the canonical JSON
    let again: Descriptor =
        serde_json::from_str(&canonical_json(&serde_json::to_value(&d).unwrap())).unwrap();
    assert_eq!(again, d);
    assert_eq!(ui_warnings(&d), vec![]);
    let r = requirements(d.ui.as_ref().unwrap());
    assert_eq!(r["orders"].compat(1), "full");
    assert_eq!(r["orders"].compat(0), "unsupported");
}

#[test]
fn references_are_checked() {
    let mut ui = good_ui();
    ui["screens"][0]["queries"][0]["endpoint"] = json!("Internal");
    ui["screens"][0]["actions"][1]["steps"][1]["call"]["args"][0]["field"] = json!("idd");
    ui["screens"][0]["layout"]["stack"]["children"][1]["table"]["columns"][0]["field"] =
        json!("nr");
    ui["screens"][1]["actions"][0]["steps"][0]["open"]["overlay"] = json!("nope");
    ui["nav"][0]["entries"][0]["title"] = json!({ "key": "nav.none" });
    ui["commands"][0]["target"] = json!({ "screen": "order" });
    let c = codes(ui);
    for code in [
        "E_UI_ENDPOINT_NOT_UI",
        "E_UI_FIELD_UNKNOWN",
        "E_UI_OVERLAY_UNKNOWN",
        "E_UI_TEXT_UNKNOWN",
        "E_UI_PARAM_MISSING",
    ] {
        assert!(c.iter().any(|x| x == code), "{code} in {c:?}");
    }
}

#[test]
fn contexts_types_and_safety() {
    let mut ui = good_ui();
    // a row value outside a row
    ui["screens"][1]["layout"]["fields"]["items"][0]["value"] = json!({ "row": "total" });
    // a uuid field given a number
    ui["screens"][1]["queries"][0]["args"][0]["value"] = json!({ "lit": 7 });
    // a query on an endpoint that changes data
    ui["screens"][1]["queries"][0]["endpoint"] = json!("CancelOrder");
    // an error toast outside on_error
    ui["screens"][0]["actions"][0]["steps"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "toast": { "error": true } }));
    // an http URL
    ui["screens"][0]["actions"][0]["steps"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "open_url": { "value": { "lit": "http://example.com" } } }));
    let c = codes(ui);
    for code in [
        "E_UI_REF_CONTEXT",
        "E_UI_ARG_TYPE",
        "E_UI_QUERY_MUTATES",
        "E_UI_URL",
    ] {
        assert!(c.iter().any(|x| x == code), "{code} in {c:?}");
    }
}

#[test]
fn topics_are_declared_and_authorized() {
    let mut v = shop(good_ui());
    v["topics"][1]["authorize"] = json!("CountOpen");
    v["endpoints"][2]["invalidates"][1]["key"][0]["from"] = json!("nope");
    v["ui"]["screens"][0]["queries"][0]["follow"][0]["topic"] = json!("ghost");
    let d: Descriptor = serde_json::from_value(v).unwrap();
    let c: Vec<_> = validate(&d).into_iter().map(|e| e.code).collect();
    for code in [
        "E_TOPIC_AUTHORIZE",
        "E_UI_FIELD_UNKNOWN",
        "E_UI_TOPIC_UNKNOWN",
    ] {
        assert!(c.contains(&code), "{code} in {c:?}");
    }
}

#[test]
fn a_keyed_topic_needs_an_authorizing_read() {
    let mut v = shop(good_ui());
    v["topics"][1].as_object_mut().unwrap().remove("authorize");
    let d: Descriptor = serde_json::from_value(v).unwrap();
    let e: Vec<_> = validate(&d).into_iter().map(|e| (e.code, e.path)).collect();
    assert!(
        e.iter()
            .any(|(c, p)| *c == "E_TOPIC_AUTHORIZE" && p == "topics[order].authorize"),
        "{e:?}"
    );
}

#[test]
fn warnings_name_unsafe_calls_and_silent_topics() {
    let mut v = shop(good_ui());
    v["endpoints"][2]["idempotency"] = json!("off");
    v["endpoints"][2]["invalidates"] = json!([]);
    let d: Descriptor = serde_json::from_value(v).unwrap();
    let w: Vec<_> = ui_warnings(&d).into_iter().map(|e| e.code).collect();
    assert!(w.contains(&"W_UI_CALL_NOT_IDEMPOTENT"), "{w:?}");
    assert!(w.contains(&"W_UI_TOPIC_NEVER_INVALIDATED"), "{w:?}");
}

#[test]
fn a_topic_the_runs_of_an_agent_invalidate_is_not_silent() {
    let mut v = shop(good_ui());
    v["endpoints"][2]["invalidates"] = json!([]);
    let silent = |v: &Value| {
        let d: Descriptor = serde_json::from_value(v.clone()).unwrap();
        ui_warnings(&d)
            .into_iter()
            .filter(|e| e.code == "W_UI_TOPIC_NEVER_INVALIDATED")
            .map(|e| e.path)
            .collect::<Vec<_>>()
    };
    let before = silent(&v);
    assert!(before.contains(&"topics[orders]".to_string()), "{before:?}");
    v["agents"] = json!([{ "name": "helper", "topic": "orders" }]);
    let after = silent(&v);
    assert!(!after.contains(&"topics[orders]".to_string()), "{after:?}");
}

#[test]
fn removed_screens_and_new_required_params_break() {
    let old: Descriptor = serde_json::from_value(shop(good_ui())).unwrap();
    let mut ui = good_ui();
    ui["screens"].as_array_mut().unwrap().remove(1);
    ui["screens"][0]["params"][0]["required"] = json!(true);
    ui["commands"] = json!([]);
    let new: Descriptor = serde_json::from_value(shop(ui)).unwrap();
    let ch = lock::compare(&old, &new);
    let breaking: Vec<_> = ch.iter().filter(|c| c.breaking).map(|c| c.code).collect();
    for code in [
        "E_UI_SCREEN_REMOVED",
        "E_UI_PARAM_REQUIRED",
        "E_UI_COMMAND_REMOVED",
    ] {
        assert!(breaking.contains(&code), "{code} in {breaking:?}");
    }
}

/// The orders screen with a `range` param that has a default and a switch over it.
fn ranged_ui() -> Value {
    let mut ui = good_ui();
    ui["screens"][0]["params"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "range", "type": "string", "default": "30" }));
    ui["screens"][0]["layout"]["stack"]["children"]
        .as_array_mut()
        .unwrap()
        .push(
            json!({ "segmented": { "bind": { "param": "range" }, "options": [
            { "value": "7", "label": "7 days" }, { "value": "30", "label": "30 days" }] } }),
        );
    ui
}

#[test]
fn a_param_default_is_a_value_of_the_param_and_an_option_of_its_switch() {
    assert_eq!(errors(ranged_ui()), vec![]);
    // it round-trips, and a document without a default reads as before
    let d: Descriptor = serde_json::from_value(shop(ranged_ui())).unwrap();
    let ui = d.ui.as_ref().unwrap();
    assert_eq!(ui.screens[0].params[1].default, Some(json!("30")));
    assert_eq!(ui.screens[0].params[0].default, None);
    assert_eq!(ui.catalog, 1);
    // the default of each type: a string, a number, a boolean, an enum value
    let with = |ty: &str, default: Value| {
        let mut ui = good_ui();
        ui["screens"][0]["params"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "x", "type": ty, "default": default }));
        errors(ui)
    };
    let bad = vec![(
        "E_UI_PARAM_DEFAULT".to_string(),
        "ui.screens[orders].params[x].default".to_string(),
    )];
    for (ty, default) in [
        ("string", json!("30")),
        ("int32", json!(30)),
        ("int64", json!("30")),
        ("double", json!(0.5)),
        ("bool", json!(true)),
        ("date", json!("2026-11-01")),
        ("enum:OrderStatus", json!("paid")),
    ] {
        assert_eq!(with(ty, default.clone()), vec![], "{ty} {default}");
    }
    for (ty, default) in [
        ("string", json!(30)),
        ("int32", json!("thirty")),
        ("bool", json!("yes")),
        ("enum:OrderStatus", json!("refunded")),
        ("string", json!(["30"])),
        ("string", json!({ "days": 30 })),
    ] {
        assert_eq!(with(ty, default.clone()), bad, "{ty} {default}");
    }
    // a switch over the param offers its default (`30` and `"30"` are one value in a link)
    let mut ui = ranged_ui();
    ui["screens"][0]["params"][1]["default"] = json!("90");
    assert_eq!(
        errors(ui),
        vec![(
            "E_UI_PARAM_DEFAULT".to_string(),
            "ui.screens[orders].layout.stack[2].segmented.options".to_string()
        )]
    );
    let mut ui = ranged_ui();
    ui["screens"][0]["params"][1] = json!({ "name": "range", "type": "int32", "default": 30 });
    assert_eq!(errors(ui), vec![]);
    // without a default the switch is as it was: any options
    let mut ui = ranged_ui();
    ui["screens"][0]["params"][1] = json!({ "name": "range", "type": "string" });
    assert_eq!(errors(ui), vec![]);
}

#[test]
fn a_param_with_a_default_is_never_missing() {
    // a required param with a default needs no arg in a link, a nav entry or a command
    let mut ui = ranged_ui();
    ui["screens"][0]["params"][1]["required"] = json!(true);
    assert_eq!(errors(ui.clone()), vec![]);
    // and adding it is not a breaking change, while a required one without a default is
    let old: Descriptor = serde_json::from_value(shop(good_ui())).unwrap();
    let new: Descriptor = serde_json::from_value(shop(ui.clone())).unwrap();
    let breaking = |new: &Descriptor| -> Vec<&str> {
        lock::compare(&old, new)
            .iter()
            .filter(|c| c.breaking)
            .map(|c| c.code)
            .collect()
    };
    assert_eq!(breaking(&new), Vec::<&str>::new());
    ui["screens"][0]["params"][1]
        .as_object_mut()
        .unwrap()
        .remove("default");
    ui["screens"][0]["layout"]["stack"]["children"]
        .as_array_mut()
        .unwrap()
        .pop();
    ui["nav"][0]["entries"][0]["args"] = json!([{ "field": "range", "value": { "lit": "30" } }]);
    ui["commands"][0]["target"]["args"] = json!([{ "field": "range", "value": { "lit": "30" } }]);
    let new: Descriptor = serde_json::from_value(shop(ui)).unwrap();
    assert_eq!(validate(&new), vec![]);
    assert_eq!(breaking(&new), vec!["E_UI_PARAM_REQUIRED"]);
}

#[test]
fn a_command_carries_args_to_its_screen() {
    let mut ui = ranged_ui();
    ui["commands"] = json!([
        { "id": "find", "title": "Find an order", "target": { "screen": "orders" } },
        { "id": "week", "title": "Orders: last 7 days", "icon": "calendar", "target": { "screen": "orders", "args": [
            { "field": "range", "value": { "lit": "7" } }, { "field": "status", "value": { "lit": "paid" } }] } },
        { "id": "order", "title": "Open an order", "target": { "screen": "order", "args": [
            { "field": "id", "value": { "lit": "0b0e7c1e-58a5-4b53-9d0c-3a1e2f6f7a10" } }] } }
    ]);
    assert_eq!(errors(ui.clone()), vec![]);
    let d: Descriptor = serde_json::from_value(shop(ui.clone())).unwrap();
    assert_eq!(d.ui.as_ref().unwrap().commands[1].target.args.len(), 2);
    // the args are params of the screen, and a command has no screen to read a param or a state from
    ui["commands"][1]["target"]["args"] = json!([
        { "field": "days", "value": { "lit": "7" } }, { "field": "range", "value": { "param": "range" } }]);
    // a required param without a default is still asked for
    ui["commands"][2]["target"]["args"] = json!([]);
    assert_eq!(
        errors(ui),
        vec![
            (
                "E_UI_PARAM_UNKNOWN".to_string(),
                "ui.commands[week].target.args[days]".to_string()
            ),
            (
                "E_UI_PARAM_UNKNOWN".to_string(),
                "ui.commands[week].target.args[range]".to_string()
            ),
            (
                "E_UI_PARAM_MISSING".to_string(),
                "ui.commands[order].target.args".to_string()
            ),
        ]
    );
}

#[test]
fn a_search_action_reads_the_found_record() {
    let source = |field: &str| {
        json!([{ "title": "Orders", "endpoint": "ListOrders", "text": "page_token", "rows": "orders",
                 "item": { "title": { "row": "number" } },
                 "open": { "screen": "order", "args": [{ "field": "id", "value": { "row": "id" } }] },
                 "actions": [{ "title": "Cancel this order", "icon": "x",
                               "target": { "screen": "order", "args": [{ "field": "id", "value": { "row": field } }] } }] }])
    };
    let mut ui = good_ui();
    ui["search"] = source("id");
    let d: Descriptor = serde_json::from_value(shop(ui)).unwrap();
    assert_eq!(validate(&d), vec![]);

    let mut ui = good_ui();
    ui["search"] = source("nope");
    let d: Descriptor = serde_json::from_value(shop(ui)).unwrap();
    let e: Vec<_> = validate(&d).into_iter().map(|e| e.path).collect();
    assert!(
        e.iter()
            .any(|p| p.starts_with("ui.search[0].actions[0].target")),
        "{e:?}"
    );
}

#[test]
fn a_sortable_column_and_a_multi_value_filter_are_checked() {
    fn sort(ui: &mut Value) -> &mut Value {
        &mut ui["screens"][0]["layout"]["stack"]["children"][1]["table"]["columns"][1]["sort"]
    }
    // a sort field is a string field of the request of the table's query
    let mut ui = good_ui();
    *sort(&mut ui) = json!("sort_total");
    assert_eq!(errors(ui.clone()), vec![]);
    *sort(&mut ui) = json!("sort_number");
    assert_eq!(codes(ui.clone()), vec!["E_UI_FIELD_UNKNOWN"]);
    *sort(&mut ui) = json!("page_size");
    assert_eq!(codes(ui), vec!["E_UI_ARG_TYPE"]);
    // a multi-select filter keeps its values in a string param
    let mut ui = good_ui();
    ui["screens"][0]["layout"]["stack"]["children"][0]["filter_bar"]["inputs"][0]["control"] =
        json!("multi_select");
    assert_eq!(codes(ui), vec!["E_UI_ARG_TYPE"]);
}

/// The shop with a daily sales report: rows per day and channel, and totals.
fn report(ui: Value) -> Value {
    let mut v = shop(ui);
    v["messages"].as_array_mut().unwrap().extend([
        json!({ "name": "Day", "fields": [f("day", 1, "date"), f("channel", 2, "string"), f("total", 3, "decimal"),
                                          f("online", 4, "decimal"), f("retail", 5, "decimal")] }),
        json!({ "name": "SalesResponse", "fields": [rep("days", 1, "message:Day"), f("revenue", 2, "decimal")] }),
    ]);
    v["endpoints"].as_array_mut().unwrap().push(json!(
        { "name": "Sales", "input": "Empty", "output": "SalesResponse", "scopes": ["read"], "read_only": true, "ui": true }
    ));
    v
}

/// The whole `viz` node of the contract, over the `sales` query.
fn viz() -> Value {
    json!({ "viz": {
      "family": "line", "kind": "line", "title": "Revenue", "range": { "key": "nav.all" },
      "options": { "fmt": "money", "curve": "smooth", "center": { "key": "not a text" } },
      "bind": [
        { "to": "labels", "from": { "column": { "query": "sales", "field": "day", "as": "date" } } },
        { "to": "values", "from": { "column": { "query": "sales", "field": "total" } } },
        { "to": "series", "from": { "series": { "query": "sales", "fields": [{ "field": "online", "label": "Online store" }] } } },
        { "to": "series", "from": { "pivot": { "query": "sales", "x": "day", "series": "channel", "value": "total", "as": "date" } } },
        { "to": "rows", "from": { "rows": { "query": "sales", "map": { "key": "channel", "value": "total" }, "text": ["key"] } } },
        { "to": "center.value", "from": { "value": { "data": { "query": "sales", "field": "revenue" } } } }
      ],
      "height": 280,
      "select": { "state": "channel" },
      "open": { "screen": "orders" }
    } })
}

/// `good_ui` with a report screen whose layout is `node`.
fn report_ui(node: Value) -> Value {
    let mut ui = good_ui();
    ui["catalog"] = json!(2);
    ui["screens"].as_array_mut().unwrap().push(json!({
      "id": "report", "title": "Report",
      "params": [{ "name": "period", "type": "string" }],
      "state": [{ "name": "channel", "type": "string" }],
      "queries": [{ "name": "sales", "endpoint": "Sales", "rows": "days" },
                  { "name": "count", "endpoint": "CountOpen" }],
      "layout": node
    }));
    ui
}

fn report_errors(ui: Value) -> Vec<(String, String)> {
    let d: Descriptor = serde_json::from_value(report(ui)).expect("parses");
    validate(&d)
        .into_iter()
        .map(|e| (e.code.to_string(), e.path))
        .collect()
}

/// The errors of a report screen whose `viz` was changed by `change`.
fn viz_errors(change: impl FnOnce(&mut Value)) -> Vec<(String, String)> {
    let mut node = viz();
    change(&mut node["viz"]);
    report_errors(report_ui(node))
}

#[test]
fn a_viz_validates_and_round_trips() {
    assert_eq!(report_errors(report_ui(viz())), vec![]);
    let d: Descriptor = serde_json::from_value(report(report_ui(viz()))).unwrap();
    // the document keeps the contract's shape
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["ui"]["screens"][2]["layout"], viz());
    let again: Descriptor = serde_json::from_str(&canonical_json(&v)).unwrap();
    assert_eq!(again, d);
    // the smallest one: a kind alone (`options` and `bind` are left out)
    let small = json!({ "viz": { "family": "figure", "kind": "metric" } });
    assert_eq!(report_errors(report_ui(small.clone())), vec![]);
    let d: Descriptor = serde_json::from_value(report(report_ui(small.clone()))).unwrap();
    assert_eq!(
        serde_json::to_value(&d).unwrap()["ui"]["screens"][2]["layout"],
        small
    );
    // unknown fields, sources and casts do not parse
    for bad in [
        json!({ "viz": { "family": "line", "kind": "line", "colour": "red" } }),
        json!({ "viz": { "family": "line", "kind": "line", "options": [] } }),
        json!({ "viz": { "family": "line", "kind": "line",
                         "bind": [{ "to": "values", "from": { "cells": { "query": "sales" } } }] } }),
        json!({ "viz": { "family": "line", "kind": "line", "bind": [{ "to": "values",
                         "from": { "column": { "query": "sales", "field": "total", "as": "money" } } }] } }),
    ] {
        assert!(
            serde_json::from_value::<Descriptor>(report(report_ui(bad.clone()))).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn a_viz_is_a_display_primitive_of_catalog_2() {
    let mut node = viz();
    node["fallback"] = json!({ "text": { "text": "Revenue by day" } });
    let ui = report_ui(json!({ "stack": { "children": [node] } }));
    assert_eq!(report_errors(ui.clone()), vec![]);
    let d: Descriptor = serde_json::from_value(report(ui.clone())).unwrap();
    let r = requirements(d.ui.as_ref().unwrap());
    // an older client draws the fallback; the screens without a viz are whole for it
    assert_eq!(r["report"].compat(1), "degraded");
    assert_eq!(r["report"].compat(2), "full");
    assert_eq!(r["orders"].compat(1), "full");
    assert_eq!((r["report"].interactive, r["report"].display), (0, 2));
    // a document of a newer catalog than the platform's is refused
    let mut newer = ui;
    newer["catalog"] = json!(ui::CATALOG + 1);
    assert_eq!(
        report_errors(newer),
        vec![("E_UI_CATALOG".to_string(), "ui.catalog".to_string())]
    );
}

#[test]
fn a_viz_kind_is_of_the_catalogue() {
    let at = "ui.screens[report].layout.viz";
    for (family, kind) in [("line", "column"), ("pie", "pie"), ("meter", "meter_list")] {
        let e = viz_errors(|v| {
            v["family"] = json!(family);
            v["kind"] = json!(kind);
        });
        assert_eq!(e, vec![("E_UI_VIZ_KIND".into(), format!("{at}.kind"))]);
    }
    assert_eq!(
        viz_errors(|v| {
            v["family"] = json!("meter");
            v["kind"] = json!("meterList");
        }),
        vec![]
    );
    // neither an option nor a bind replaces the kind
    assert_eq!(
        viz_errors(|v| v["options"]["kind"] = json!("slope")),
        vec![("E_UI_VIZ_KIND".into(), format!("{at}.options.kind"))]
    );
    assert_eq!(
        viz_errors(|v| v["bind"][5]["to"] = json!("kind")),
        vec![("E_UI_VIZ_KIND".into(), format!("{at}.bind[5].to"))]
    );
}

#[test]
fn the_binds_of_a_viz_are_checked() {
    let at = "ui.screens[report].layout.viz";
    let one = |change: fn(&mut Value), code: &str, path: &str| {
        assert_eq!(
            viz_errors(change),
            vec![(code.to_string(), format!("{at}{path}"))],
            "{code} at {path}"
        );
    };
    // queries: declared in the frame, with rows for the row sources
    one(
        |v| v["bind"][0]["from"]["column"]["query"] = json!("ghost"),
        "E_UI_QUERY_UNKNOWN",
        ".bind[0].from.column.query",
    );
    one(
        |v| v["bind"][2]["from"]["series"]["query"] = json!("count"),
        "E_UI_ROWS",
        ".bind[2].from.series.query",
    );
    one(
        |v| v["bind"][5]["from"]["value"]["data"]["query"] = json!("ghost"),
        "E_UI_QUERY_UNKNOWN",
        ".bind[5].from.value",
    );
    // fields: of the row message (of the response for a value)
    one(
        |v| v["bind"][1]["from"]["column"]["field"] = json!("sum"),
        "E_UI_FIELD_UNKNOWN",
        ".bind[1].from.column.field",
    );
    one(
        |v| v["bind"][2]["from"]["series"]["fields"][0]["field"] = json!("web"),
        "E_UI_FIELD_UNKNOWN",
        ".bind[2].from.series.fields[web]",
    );
    one(
        |v| v["bind"][3]["from"]["pivot"]["series"] = json!("shop"),
        "E_UI_FIELD_UNKNOWN",
        ".bind[3].from.pivot.series",
    );
    one(
        |v| v["bind"][4]["from"]["rows"]["map"]["value"] = json!("sum"),
        "E_UI_FIELD_UNKNOWN",
        ".bind[4].from.rows.map[value]",
    );
    one(
        |v| v["bind"][5]["from"]["value"]["data"]["field"] = json!("total"),
        "E_UI_FIELD_UNKNOWN",
        ".bind[5].from.value",
    );
    // a value is checked like any value: a row is not in reach, a param and a literal are
    one(
        |v| v["bind"][5]["from"]["value"] = json!({ "row": "total" }),
        "E_UI_REF_CONTEXT",
        ".bind[5].from.value",
    );
    one(
        |v| v["bind"][5]["from"]["value"] = json!({ "state": "ghost" }),
        "E_UI_STATE_UNKNOWN",
        ".bind[5].from.value",
    );
    for ok in [
        json!({ "param": "period" }),
        json!({ "lit": 12.5 }),
        json!({ "caller": "display_name" }),
    ] {
        assert_eq!(viz_errors(|v| v["bind"][5]["from"]["value"] = ok), vec![]);
    }
    // the shape of a bind
    one(
        |v| v["bind"][5]["to"] = json!("center..value"),
        "E_UI_VIZ_BIND",
        ".bind[5].to",
    );
    one(
        |v| v["bind"][5]["to"] = json!("__proto__.polluted"),
        "E_UI_VIZ_BIND",
        ".bind[5].to",
    );
    one(
        |v| v["bind"][2]["from"]["series"]["fields"] = json!([]),
        "E_UI_VIZ_BIND",
        ".bind[2].from.series.fields",
    );
    one(
        |v| v["bind"][4]["from"]["rows"] = json!({ "query": "sales", "map": {} }),
        "E_UI_VIZ_BIND",
        ".bind[4].from.rows.map",
    );
    one(
        |v| v["bind"][4]["from"]["rows"]["text"] = json!(["name"]),
        "E_UI_VIZ_BIND",
        ".bind[4].from.rows.text",
    );
    one(
        |v| v["bind"][2]["from"]["series"]["fields"][0]["label"] = json!(" "),
        "E_UI_TEXT",
        ".bind[2].from.series.fields[online].label",
    );
}

#[test]
fn the_limits_selection_and_link_of_a_viz_are_checked() {
    let at = "ui.screens[report].layout.viz";
    let limit = |change: fn(&mut Value), path: &str| {
        assert_eq!(
            viz_errors(change),
            vec![("E_UI_LIMIT".to_string(), format!("{at}{path}"))]
        );
    };
    limit(
        |v| v["options"]["note"] = json!("x".repeat(ui::VIZ_MAX_OPTIONS)),
        ".options",
    );
    limit(
        |v| {
            let b = v["bind"][1].clone();
            v["bind"] = json!(vec![b; ui::VIZ_MAX_BINDS + 1]);
        },
        ".bind",
    );
    limit(|v| v["height"] = json!(15), ".height");
    limit(|v| v["height"] = json!(1201), ".height");
    for h in [16, 1200] {
        assert_eq!(viz_errors(|v| v["height"] = json!(h)), vec![]);
    }
    assert_eq!(
        viz_errors(|v| {
            let b = v["bind"][1].clone();
            v["bind"] = json!(vec![b; ui::VIZ_MAX_BINDS]);
        }),
        vec![]
    );
    // the selection goes to a state or a param of the screen
    assert_eq!(
        viz_errors(|v| v["select"] = json!({ "param": "period" })),
        vec![]
    );
    assert_eq!(
        viz_errors(|v| v["select"] = json!({ "state": "period" })),
        vec![("E_UI_STATE_UNKNOWN".into(), format!("{at}.select"))]
    );
    assert_eq!(
        viz_errors(|v| v["select"] = json!({ "param": "channel" })),
        vec![("E_UI_PARAM_UNKNOWN".into(), format!("{at}.select"))]
    );
    // the link leads to a screen, with its required params
    assert_eq!(
        viz_errors(|v| v["open"] = json!({ "screen": "ghost" })),
        vec![("E_UI_SCREEN_UNKNOWN".into(), format!("{at}.open.screen"))]
    );
    assert_eq!(
        viz_errors(|v| v["open"] = json!({ "screen": "order" })),
        vec![("E_UI_PARAM_MISSING".into(), format!("{at}.open.args"))]
    );
    assert_eq!(
        viz_errors(|v| v["open"] = json!({})),
        vec![("E_UI_TARGET".into(), format!("{at}.open"))]
    );
    // the title and the range are texts: a key is of the en catalog
    assert_eq!(
        viz_errors(|v| v["range"] = json!({ "key": "report.range" })),
        vec![(
            "E_UI_TEXT_UNKNOWN".into(),
            "ui.screens[2].layout.viz.range".to_string()
        )]
    );
}

#[test]
fn a_viz_in_an_overlay_reads_the_overlay() {
    // the state and the query of an overlay are in reach of its viz; the screen's state is not
    let mut ui = report_ui(json!({ "text": { "text": "Report" } }));
    let mut node = viz();
    node["viz"]["bind"] =
        json!([{ "to": "values", "from": { "column": { "query": "by_day", "field": "total" } } }]);
    node["viz"]["select"] = json!({ "state": "picked" });
    ui["screens"][2]["overlays"] = json!([{
      "id": "chart", "kind": "dialog", "title": "Chart",
      "state": [{ "name": "picked", "type": "string" }],
      "queries": [{ "name": "by_day", "endpoint": "Sales", "rows": "days" }],
      "layout": node
    }]);
    assert_eq!(report_errors(ui.clone()), vec![]);
    ui["screens"][2]["overlays"][0]["layout"]["viz"]["select"] = json!({ "state": "channel" });
    assert_eq!(
        report_errors(ui),
        vec![(
            "E_UI_STATE_UNKNOWN".to_string(),
            "ui.screens[report].overlays[chart].layout.viz.select".to_string()
        )]
    );
}

#[test]
fn a_viz_in_a_home_widget_reads_document_queries() {
    let widget = |query: &str| {
        let mut ui = good_ui();
        ui["catalog"] = json!(2);
        ui["queries"].as_array_mut().unwrap().push(json!(
            { "name": "home_sales_days", "endpoint": "Sales", "rows": "days" }
        ));
        ui["home"] = json!([{ "id": "sales", "title": "Sales", "open": { "screen": "orders" }, "node": { "viz": {
          "family": "bar", "kind": "column",
          "bind": [{ "to": "labels", "from": { "column": { "query": query, "field": "day", "as": "date" } } },
                   { "to": "values", "from": { "column": { "query": query, "field": "total" } } },
                   { "to": "value", "from": { "value": { "data": { "query": "open_count", "field": "count" } } } }]
        } } }]);
        ui
    };
    assert_eq!(report_errors(widget("home_sales_days")), vec![]);
    let d: Descriptor = serde_json::from_value(report(widget("home_sales_days"))).unwrap();
    let r = requirements(d.ui.as_ref().unwrap());
    assert_eq!(r["home"].compat(1), "degraded");
    // a screen's query is not in reach of the home
    let e = report_errors(widget("orders"));
    assert_eq!(e.len(), 2, "{e:?}");
    assert!(e.iter().all(|(c, _)| c == "E_UI_QUERY_UNKNOWN"), "{e:?}");
    // a home widget has no state to select into
    let mut ui = widget("home_sales_days");
    ui["home"][0]["node"]["viz"]["select"] = json!({ "state": "channel" });
    assert_eq!(
        report_errors(ui),
        vec![(
            "E_UI_STATE_UNKNOWN".to_string(),
            "ui.home[sales].node.viz.select".to_string()
        )]
    );
}

/// A board of the given items over the report screen's queries.
fn board(items: Value) -> Value {
    json!({ "board": { "children": items } })
}

fn metric_tile() -> Value {
    json!({ "viz": { "family": "figure", "kind": "metric",
                     "bind": [{ "to": "value", "from": { "value": { "data": { "query": "sales", "field": "revenue" } } } }] } })
}

#[test]
fn a_board_lays_cards_on_twelve_columns() {
    let ui = report_ui(board(json!([
        { "span": 3, "node": metric_tile() },
        { "span": 9, "node": viz() },
        { "node": { "text": { "text": "By channel", "style": "heading" } } },
        { "span": 12, "node": metric_tile() }
    ])));
    assert_eq!(report_errors(ui.clone()), vec![]);
    let d: Descriptor = serde_json::from_value(report(ui)).unwrap();
    assert_eq!(ui_warnings(&d), vec![]);
    // the full width is the default span: it is left out
    let v = serde_json::to_value(&d).unwrap();
    let items = &v["ui"]["screens"][2]["layout"]["board"]["children"];
    assert_eq!(items[0]["span"], 3);
    assert!(items[2].get("span").is_none() && items[3].get("span").is_none());
    assert_eq!(
        serde_json::from_str::<Descriptor>(&canonical_json(&v)).unwrap(),
        d
    );
    // a layout of catalog 2 with display children: an older client draws its fallback
    let r = requirements(d.ui.as_ref().unwrap());
    assert_eq!(r["report"].compat(1), "degraded");
    assert_eq!(r["report"].compat(2), "full");
    // an item has a node and a span, nothing else
    for bad in [
        json!([{ "span": 3 }]),
        json!([{ "span": 3, "node": metric_tile(), "rows": 2 }]),
        json!([metric_tile()]),
    ] {
        assert!(
            serde_json::from_value::<Descriptor>(report(report_ui(board(bad.clone())))).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn the_spans_items_and_nesting_of_a_board_are_checked() {
    let at = "ui.screens[report].layout.board";
    for span in [0, 13] {
        assert_eq!(
            report_errors(report_ui(board(json!([
                { "span": 6, "node": metric_tile() },
                { "span": span, "node": metric_tile() }
            ])))),
            vec![("E_UI_LIMIT".to_string(), format!("{at}.children[1].span"))]
        );
    }
    for span in [1, 12] {
        assert_eq!(
            report_errors(report_ui(board(
                json!([{ "span": span, "node": metric_tile() }])
            ))),
            vec![]
        );
    }
    let items = |n: usize| json!(vec![json!({ "span": 3, "node": { "divider": {} } }); n]);
    assert_eq!(
        report_errors(report_ui(board(items(ui::BOARD_MAX_ITEMS)))),
        vec![]
    );
    assert_eq!(
        report_errors(report_ui(board(items(ui::BOARD_MAX_ITEMS + 1)))),
        vec![("E_UI_LIMIT".to_string(), format!("{at}.children"))]
    );
    // a board is not inside another board, however deep; two boards side by side are fine
    let inner = board(json!([{ "span": 6, "node": metric_tile() }]));
    assert_eq!(
        report_errors(report_ui(board(json!([{ "node": inner.clone() }])))),
        vec![("E_UI_BOARD_NESTED".to_string(), format!("{at}[0].board"))]
    );
    assert_eq!(
        report_errors(report_ui(board(json!([
            { "node": { "stack": { "children": [{ "card": { "children": [inner.clone()] } }] } } }
        ])))),
        vec![(
            "E_UI_BOARD_NESTED".to_string(),
            format!("{at}[0].stack[0].card[0].board")
        )]
    );
    assert_eq!(
        report_errors(report_ui(
            json!({ "stack": { "children": [inner.clone(), inner] } })
        )),
        vec![]
    );
    // a child is checked as any node, with the screen's queries and state
    assert_eq!(
        report_errors(report_ui(board(json!([
            { "span": 4, "node": { "viz": { "family": "bar", "kind": "bar", "select": { "state": "ghost" },
                "bind": [{ "to": "values", "from": { "column": { "query": "sales", "field": "sum" } } }] } } }
        ])))),
        vec![
            (
                "E_UI_FIELD_UNKNOWN".to_string(),
                format!("{at}[0].viz.bind[0].from.column.field")
            ),
            (
                "E_UI_STATE_UNKNOWN".to_string(),
                format!("{at}[0].viz.select")
            ),
        ]
    );
}

#[test]
fn a_button_in_a_board_needs_a_client_that_draws_the_board() {
    let mut ui = report_ui(board(json!([
        { "span": 6, "node": metric_tile() },
        { "span": 6, "node": { "button": { "label": "Refresh", "action": "refresh" } } }
    ])));
    ui["screens"][2]["actions"] = json!([{ "name": "refresh", "steps": [{ "refresh": {} }] }]);
    assert_eq!(report_errors(ui.clone()), vec![]);
    let d: Descriptor = serde_json::from_value(report(ui)).unwrap();
    let r = requirements(d.ui.as_ref().unwrap());
    // an older client would draw a placeholder for the board and lose the button: the screen is not shown
    assert_eq!(r["report"].compat(1), "unsupported");
    assert_eq!(r["report"].compat(2), "full");
}

#[test]
fn a_viz_is_not_framed_twice() {
    let at = "ui.screens[report].layout";
    let text = json!({ "text": { "text": "Sales" } });
    // a viz draws its own card: directly on a board, in columns or in a stack
    for layout in [
        board(json!([{ "span": 6, "node": metric_tile() }])),
        json!({ "columns": { "children": [metric_tile(), text] } }),
        json!({ "stack": { "children": [text, metric_tile()] } }),
        // alone in a card or a section
        json!({ "card": { "children": [metric_tile()] } }),
        json!({ "section": { "title": "Sales", "actions": [{ "divider": {} }], "children": [metric_tile()] } }),
    ] {
        assert_eq!(report_errors(report_ui(layout.clone())), vec![], "{layout}");
    }
    assert_eq!(
        report_errors(report_ui(
            json!({ "card": { "title": "Sales", "children": [text, metric_tile()] } })
        )),
        vec![("E_UI_VIZ_FRAMED".to_string(), format!("{at}.card[1]"))]
    );
    assert_eq!(
        report_errors(report_ui(board(json!([{ "node": { "section": {
            "title": "Sales", "actions": [{ "divider": {} }], "children": [metric_tile(), metric_tile()]
        } } }])))),
        vec![
            (
                "E_UI_VIZ_FRAMED".to_string(),
                format!("{at}.board[0].section[1]")
            ),
            (
                "E_UI_VIZ_FRAMED".to_string(),
                format!("{at}.board[0].section[2]")
            ),
        ]
    );
}

/// The descriptor the three SDKs print for `sdk/conformance/shop` (its report screen has a param with a default, a
/// switch over it and a command with args) is a valid one.
#[test]
fn the_conformance_shop_validates() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../sdk/conformance/shop/descriptor.json"
    );
    let d = parse(&std::fs::read(path).expect("the fixture")).expect("parses");
    assert_eq!(validate(&d), vec![]);
    let ui = d.ui.as_ref().unwrap();
    let report = ui.screens.iter().find(|s| s.id == "report").unwrap();
    let range = report.params.iter().find(|p| p.name == "range").unwrap();
    assert_eq!(
        (range.required, &range.default),
        (false, &Some(json!("30")))
    );
    assert_eq!(ui.commands[1].target.args[0].field, "range");
    assert_eq!(requirements(ui)["report"].compat(1), "degraded");
}

/// A chat screen whose thread has a Following bell: `actions.follow` takes `{form: follow}` (a bool) and needs the
/// thread view's `following`.
#[test]
fn a_chat_threads_bell_follows_through_an_action() {
    let doc = |following: bool, form: &str| {
        let mut thread = vec![rep("replies", 1, "message:Msg")];
        if following {
            thread.push(f("following", 2, "bool"));
        }
        json!({
          "schema": 1, "id": "talk", "version": "1.0.0",
          "messages": [
            { "name": "Msg", "fields": [f("id", 1, "string"), f("author", 2, "string"), f("text", 3, "string"),
                                         f("time", 4, "string")] },
            { "name": "IdRequest", "fields": [f("id", 1, "string")] },
            { "name": "ChannelView", "fields": [f("id", 1, "string"), rep("messages", 2, "message:Msg")] },
            { "name": "ThreadView", "fields": thread },
            { "name": "PostRequest", "fields": [f("channel", 1, "string"), f("text", 2, "string")] },
            { "name": "FollowRequest", "fields": [f("id", 1, "string"), f("follow", 2, "bool")] }
          ],
          "endpoints": [
            { "name": "GetChannel", "input": "IdRequest", "output": "ChannelView", "read_only": true, "ui": true },
            { "name": "GetThread", "input": "IdRequest", "output": "ThreadView", "read_only": true, "ui": true },
            { "name": "Post", "input": "PostRequest", "output": "Msg", "ui": true, "idempotency": "optional" },
            { "name": "Follow", "input": "FollowRequest", "output": "FollowRequest", "ui": true, "idempotency": "optional" }
          ],
          "ui": { "catalog": 1, "screens": [{
            "id": "channel", "title": "Chat",
            "params": [{ "name": "c", "type": "string" }, { "name": "thread", "type": "string" }],
            "queries": [
              { "name": "channel", "endpoint": "GetChannel", "args": [{ "field": "id", "value": { "param": "c" } }] },
              { "name": "thread", "endpoint": "GetThread", "args": [{ "field": "id", "value": { "param": "thread" } }] }],
            "actions": [
              { "name": "send", "steps": [{ "call": { "endpoint": "Post", "args": [
                  { "field": "channel", "value": { "param": "c" } }, { "field": "text", "value": { "form": "text" } }] } }] },
              { "name": "follow", "steps": [{ "call": { "endpoint": "Follow", "args": [
                  { "field": "id", "value": { "param": "thread" } }, { "field": "follow", "value": { "form": form } }] } }] }],
            "layout": { "chat": { "channel": "channel", "thread": "thread", "actions": { "send": "send", "follow": "follow" } } }
          }] }
        })
    };
    let errors = |v: Value| -> Vec<(String, String)> {
        let d: Descriptor = serde_json::from_value(v).expect("parses");
        validate(&d)
            .into_iter()
            .map(|e| (e.code.to_string(), e.path))
            .collect()
    };
    assert_eq!(errors(doc(true, "follow")), vec![]);
    let codes = |v| errors(v).into_iter().map(|(c, _)| c).collect::<Vec<_>>();
    // a thread view without the bell's state
    assert_eq!(codes(doc(false, "follow")), vec!["E_UI_FIELD_UNKNOWN"]);
    // the chat's form has no such value
    assert_eq!(codes(doc(true, "folow")), vec!["E_UI_FIELD_UNKNOWN"]);
}

#[test]
fn the_landing_is_a_screen_that_opens_without_arguments() {
    let with = |landing: &str| {
        let mut ui = good_ui();
        ui["landing"] = json!(landing);
        errors(ui)
    };
    // a screen whose params are all optional
    assert_eq!(with("orders"), vec![]);
    // no such screen
    assert_eq!(
        with("ghost"),
        vec![("E_UI_LANDING".into(), "ui.landing".into())]
    );
    // a screen with a required param and no default
    assert_eq!(
        with("order"),
        vec![("E_UI_LANDING".into(), "ui.landing".into())]
    );
    // a default makes the required param never missing
    let mut ui = good_ui();
    ui["landing"] = json!("order");
    ui["screens"][1]["params"][0]["default"] = json!("00000000-0000-0000-0000-000000000000");
    assert_eq!(errors(ui), vec![]);
}

// ---------------------------------------------------------------------------------------------- catalog 3

/// A messenger's messages, its live runs and its people: the data of the catalog 3 primitives.
fn talk(ui: Value) -> Value {
    json!({
      "schema": 1, "id": "talk", "version": "1.0.0",
      "messages": [
        { "name": "Face", "fields": [f("id", 1, "string"), f("name", 2, "string"), f("avatar", 3, "string"),
                                      f("agent", 4, "bool"), f("icon", 5, "string"), f("engine", 6, "string"),
                                      f("detail", 7, "string"), f("section", 8, "string"), f("online", 9, "bool")] },
        { "name": "Reaction", "fields": [f("emoji", 1, "string"), f("count", 2, "int32"), f("mine", 3, "bool")] },
        { "name": "Call", "fields": [f("call", 1, "string"), f("detail", 2, "string"), f("state", 3, "string"),
                                      f("output", 4, "string"), f("truncated", 5, "bool")] },
        { "name": "WorkRow", "fields": [f("kind", 1, "string"), f("title", 2, "string"), f("detail", 3, "string"),
                                         rep("tools", 4, "message:Call")] },
        { "name": "Work", "fields": [f("state", 1, "string"), f("label", 2, "string"), f("meta", 3, "string"),
                                      f("started", 4, "int64"), rep("rows", 5, "message:WorkRow"),
                                      f("steps", 6, "int32"), f("saying", 7, "string"), f("retry_label", 8, "string")] },
        { "name": "Summary", "fields": [f("count", 1, "int32"), rep("faces", 2, "message:Face"), f("last", 3, "string"),
                                         f("working", 4, "string"), f("since", 5, "int64"), f("waiting", 6, "string")] },
        { "name": "Msg", "fields": [f("id", 1, "string"), f("by", 2, "string"), f("time", 3, "string"),
                                     f("text", 4, "string"), f("writing", 5, "bool"), f("kind", 6, "string"),
                                     f("edited", 7, "bool"), f("pinned", 8, "bool"), f("mine", 9, "bool"),
                                     f("receipt", 10, "string"), f("receipt_tone", 11, "string"),
                                     f("receipt_to", 12, "string"), f("why", 13, "string"), f("fix_label", 14, "string"),
                                     rep("reactions", 15, "message:Reaction"), f("thread", 16, "message:Summary"),
                                     f("work", 17, "message:Work"), f("run", 18, "string"), f("can_retry", 19, "bool"),
                                     f("source", 20, "string")] },
        { "name": "Run", "fields": [f("run", 1, "string"), f("agent", 2, "string"), f("icon", 3, "string"),
                                     f("engine", 4, "string"), f("state", 5, "string"), f("since", 6, "int64"),
                                     f("until", 7, "int64"), f("sub", 8, "string"), f("block", 9, "string")] },
        { "name": "ChannelView", "fields": [f("title", 1, "string"), f("icon", 2, "string"), f("description", 3, "string"),
                                             f("members", 4, "string"), rep("faces", 5, "message:Face"),
                                             rep("messages", 6, "message:Msg"), rep("people", 7, "message:Face"),
                                             f("older_page_token", 8, "string"), f("pinned_count", 9, "int32"),
                                             f("readonly", 10, "bool"), f("placeholder", 11, "string"),
                                             f("empty", 12, "string"), f("root", 13, "message:Msg"),
                                             f("count", 14, "int32"), f("following", 15, "bool")] },
        { "name": "LiveView", "fields": [rep("blocks", 1, "message:Msg"), rep("people", 2, "message:Face"),
                                          rep("runs", 3, "message:Run"), f("label", 4, "string"),
                                          f("can_stop", 5, "bool"), f("hint", 6, "string")] },
        { "name": "PeopleView", "fields": [rep("people", 1, "message:Face")] },
        { "name": "RunView", "fields": [f("title", 1, "string"), f("status", 2, "string"), f("note", 3, "string"),
                                         f("call", 4, "message:Call"), rep("steps", 5, "message:WorkRow")] },
        { "name": "ChannelRequest", "fields": [f("id", 1, "string"), f("page_token", 2, "string"), f("tab", 3, "string")] },
        { "name": "RunRequest", "fields": [f("id", 1, "string"), f("call", 2, "string")] },
        { "name": "HintRequest", "fields": [f("channel", 1, "string"), f("text", 2, "string")] },
        { "name": "PostRequest", "fields": [f("channel", 1, "string"), f("text", 2, "string"), f("reply", 3, "string"),
                                             f("also", 4, "bool"), f("audio", 5, "string")] },
        { "name": "MsgRequest", "fields": [f("message", 1, "string"), f("text", 2, "string"), f("emoji", 3, "string")] },
        { "name": "FollowRequest", "fields": [f("id", 1, "string"), f("follow", 2, "bool")] },
        { "name": "StartRequest", "fields": [f("to", 1, "string"), f("text", 2, "string")] },
        { "name": "Posted", "fields": [f("message", 1, "message:Msg")] },
        { "name": "Done", "fields": [f("ok", 1, "bool")] }
      ],
      "endpoints": [
        { "name": "GetChannel", "input": "ChannelRequest", "output": "ChannelView", "read_only": true, "ui": true },
        { "name": "GetLive", "input": "ChannelRequest", "output": "LiveView", "read_only": true, "ui": true },
        { "name": "GetPeople", "input": "ChannelRequest", "output": "PeopleView", "read_only": true, "ui": true },
        { "name": "GetRun", "input": "RunRequest", "output": "RunView", "read_only": true, "ui": true },
        { "name": "PreviewHint", "input": "HintRequest", "output": "LiveView", "read_only": true, "ui": true },
        { "name": "PostMessage", "input": "PostRequest", "output": "Posted", "ui": true, "idempotency": "optional" },
        { "name": "OnMessage", "input": "MsgRequest", "output": "Done", "ui": true, "idempotency": "optional" },
        { "name": "Follow", "input": "FollowRequest", "output": "Done", "ui": true, "idempotency": "optional" },
        { "name": "Start", "input": "StartRequest", "output": "Done", "ui": true, "idempotency": "optional" }
      ],
      "ui": ui
    })
}

fn on_msg(name: &str, extra: Value) -> Value {
    let mut args = vec![json!({ "field": "message", "value": { "row": "id" } })];
    if let Value::Array(a) = extra {
        args.extend(a);
    }
    json!({ "name": name, "steps": [{ "call": { "endpoint": "OnMessage", "args": args } }] })
}

fn talk_queries() -> Value {
    json!([
      { "name": "channel", "endpoint": "GetChannel", "rows": "messages",
        "args": [{ "field": "id", "value": { "param": "c" } }],
        "older": { "token": "older_page_token", "arg": "page_token" },
        "join": [{ "as": "who", "rows": "people", "key": "id", "by": "by" }] },
      { "name": "pinned", "endpoint": "GetChannel", "rows": "messages",
        "args": [{ "field": "id", "value": { "param": "c" } }, { "field": "tab", "value": { "lit": "pinned" } }],
        "join": [{ "as": "who", "rows": "people", "key": "id", "by": "by" }],
        "when": { "eq": [{ "state": "tab" }, { "lit": "pinned" }] } },
      { "name": "live", "endpoint": "GetLive", "rows": "blocks",
        "args": [{ "field": "id", "value": { "param": "c" } }],
        "join": [{ "as": "who", "rows": "people", "key": "id", "by": "by" }] },
      { "name": "people", "endpoint": "GetPeople", "rows": "people",
        "args": [{ "field": "id", "value": { "param": "c" } }] },
      { "name": "hint", "endpoint": "PreviewHint", "debounce_ms": 300,
        "args": [{ "field": "channel", "value": { "param": "c" } }, { "field": "text", "value": { "state": "draft" } }],
        "when": { "not": { "empty": { "state": "draft" } } } },
      { "name": "run", "endpoint": "GetRun", "args": [{ "field": "id", "value": { "param": "run" } }],
        "when": { "not": { "empty": { "param": "run" } } } }
    ])
}

fn talk_actions() -> Value {
    json!([
      { "name": "send", "steps": [
          { "call": { "endpoint": "PostMessage", "args": [
              { "field": "channel", "value": { "param": "c" } }, { "field": "text", "value": { "form": "text" } },
              { "field": "reply", "value": { "form": "reply" } }, { "field": "audio", "value": { "form": "audio" } },
              { "field": "also", "value": { "form": "also" } }] } },
          { "merge": { "query": "channel", "row": { "result": "message" } } }] },
      on_msg("react", json!([{ "field": "emoji", "value": { "form": "emoji" } }])),
      on_msg("edit", json!([{ "field": "text", "value": { "form": "text" } }])),
      on_msg("mark_read", json!([])),
      on_msg("retry", json!([])),
      on_msg("correct", json!([])),
      on_msg("open_thread", json!([])),
      { "name": "open_run", "steps": [{ "set": { "param": "run", "value": { "row": "run" } } }] },
      { "name": "stop_run", "steps": [{ "call": { "endpoint": "OnMessage", "args": [
          { "field": "message", "value": { "row": "run" } }] } }] },
      { "name": "stop_dm", "steps": [{ "call": { "endpoint": "OnMessage", "args": [] } }] },
      { "name": "ask", "steps": [{ "draft": { "text": { "key": "ask", "args": { "who": { "row": "who.name" } } } } }] },
      { "name": "reply", "steps": [{ "reply_to": { "name": { "row": "who.name" }, "key": { "row": "id" } } }] },
      { "name": "copy_link", "steps": [{ "copy": { "link": { "param": "c" } } }] },
      { "name": "follow", "steps": [{ "call": { "endpoint": "Follow", "args": [
          { "field": "id", "value": { "param": "c" } }, { "field": "follow", "value": { "form": "pressed" } }] } }] },
      { "name": "message_person", "steps": [{ "call": { "endpoint": "Start", "args": [
          { "field": "to", "value": { "form": "to" } }] } }] },
      { "name": "close_run", "steps": [{ "set": { "param": "run", "value": { "lit": "" } } }] }
    ])
}

fn talk_overlay() -> Value {
    json!({
      "id": "new_message", "kind": "dialog", "title": "New message",
      "queries": [{ "name": "directory", "endpoint": "GetPeople", "rows": "people" }],
      "actions": [{ "name": "start", "steps": [{ "call": { "endpoint": "Start", "form": true } }, { "close": {} }] }],
      "layout": { "form": { "endpoint": "Start", "submit": "start",
        "submit_label": { "key": "start_with", "args": { "first": { "form": "to.first" } } },
        "notes": [{ "text": "Agents act only on messages that mention them.",
                    "visible": { "gt": [{ "form": "to.agents" }, { "lit": 0 }] } }],
        "sections": [{ "inputs": [{ "name": "to", "control": "pick", "label": "To", "placeholder": "Find people",
          "pick": { "query": "directory", "value": "name", "label": "name", "avatar": "avatar", "agent": "agent",
                    "icon": "icon", "engine": "engine", "detail": "detail", "section": "section",
                    "empty": "Nobody matches" } }] }] } }
    })
}

fn talk_header() -> Value {
    json!({ "header": {
                "title": { "data": { "query": "channel", "field": "title" } },
                "title_icon": { "data": { "query": "channel", "field": "icon" } },
                "subtitle": [{ "text": { "data": { "query": "channel", "field": "description" } } },
                             { "dot": "waiting", "visible": { "eq": [{ "data": { "query": "live", "field": "can_stop" } }, { "lit": true }] } },
                             { "value": { "data": { "query": "live", "field": "label" } }, "format": { "countdown": {} },
                               "prefix": "ends in " }],
                "faces": { "value": { "data": { "query": "channel", "field": "faces" } },
                           "label": { "data": { "query": "channel", "field": "members" } } },
                "menu": { "items": [{ "label": "Copy link", "action": "copy_link" }] },
                "tabs": { "bind": { "state": "tab" }, "items": [
                  { "id": "messages", "title": "Messages" },
                  { "id": "pinned", "title": "Pinned", "badge": { "data": { "query": "channel", "field": "pinned_count" } } }] },
                "phone": { "sub": { "data": { "query": "channel", "field": "members" } }, "menu_tabs": true } } })
}

fn talk_post() -> Value {
    json!({
                  "author": "who.name", "time": "time", "source": "source", "mine": "mine", "edited": "edited",
                  "avatar": { "url": "who.avatar", "tile": { "icon": "who.icon", "engine": "who.engine",
                                                              "when": { "eq": [{ "row": "who.agent" }, { "lit": true }] } } },
                  "badge": { "text": "AGENT", "when": { "eq": [{ "row": "who.agent" }, { "lit": true }] } },
                  "text": { "field": "text", "writing": "writing" },
                  "pinned": { "field": "pinned", "mark": { "lit": true } },
                  "receipt": { "text": "receipt", "tone": { "field": "receipt_tone", "tones": { "done": "ok" } } },
                  "route": { "to": "receipt_to", "status": "receipt", "why": "why" },
                  "correction": { "label": "fix_label", "action": "correct", "from": "time" },
                  "reactions": { "field": "reactions", "emoji": "emoji", "count": "count", "mine": "mine",
                                 "action": "react", "quick": ["👍", "🔥"] },
                  "summary": { "count": "thread.count", "faces": "thread.faces", "last": "thread.last",
                               "working": "thread.working", "since": "thread.since", "waiting": "thread.waiting",
                               "action": "open_thread",
                               "label": { "key": "replies", "args": { "n": { "row": "thread.count" } } } },
                  "process": { "state": "work.state", "label": "work.label", "meta": "work.meta",
                               "started": "work.started", "rows": "work.rows", "steps": "work.steps",
                               "saying": "work.saying",
                               "retry": { "action": "retry", "label": "work.retry_label",
                                          "when": { "eq": [{ "row": "can_retry" }, { "lit": true }] } },
                               "log": { "action": "open_run" },
                               "fetch_call": { "query": "run", "args": [{ "field": "id", "value": { "row": "run" } }],
                                               "call": "call", "field": "call" } },
                  "hover": [{ "builtin": "react" }, { "builtin": "reply" },
                            { "action": "ask", "label": "Ask agent", "style": "text",
                              "visible": { "ne": [{ "row": "who.agent" }, { "lit": true }] } },
                            { "builtin": "more" }],
                  "menu": [{ "builtin": "copy_text" }, { "builtin": "copy_link" }, { "builtin": "correction" },
                           { "builtin": "edit" }, { "builtin": "separator" },
                           { "action": "reply", "label": "Reply", "danger": false }],
                  "sheet": [{ "builtin": "reply" }, { "builtin": "copy_text" }],
                  "edit": { "action": "edit", "when": { "eq": [{ "row": "mine" }, { "lit": true }] } },
                  "actions_when": { "ne": [{ "data": { "query": "channel", "field": "readonly" } }, { "lit": true }] },
                  "card": { "query": "people", "name": "name", "avatar": "avatar", "agent": "agent", "icon": "icon",
                            "engine": "engine", "detail": "detail", "action": "message_person",
                            "unknown": "Not in this workspace's directory" } })
}

fn talk_stream() -> Value {
    json!({ "stream": {
                "query": "channel", "key": "id", "id": "messages", "days": "time",
                "tail": { "query": "live", "hold": true },
                "kinds": { "field": "kind", "line": { "when": "event", "author": "who.name", "text": "text", "time": "time" } },
                "jump_working": { "data": { "query": "live", "field": "label" } },
                "empty": { "data": { "query": "channel", "field": "empty" } },
                "on_seen": { "action": "mark_read" },
                "item": { "post": talk_post() } },
              "visible": { "eq": [{ "state": "tab" }, { "lit": "messages" }] } })
}

fn talk_composer() -> Value {
    json!({ "composer": {
                "send": "send", "for": "messages", "primary": true, "reply": true, "tags": true,
                "placeholder": { "data": { "query": "channel", "field": "placeholder" } },
                "mentions": { "query": "people", "name": "name", "avatar": "avatar", "agent": "agent", "icon": "icon",
                              "engine": "engine", "detail": "detail", "section": "section",
                              "note": "A mention always reaches the agent" },
                "dictation": {}, "draft_state": "draft",
                "options": [{ "name": "also", "label": "Also send to the channel" }],
                "caption": [{ "value": { "data": { "query": "hint", "field": "hint" } }, "icon": "corner-down-right",
                              "style": "route" },
                            { "value": { "data": { "query": "live", "field": "hint" } }, "style": "dot" }],
                "stop": { "action": "stop_dm", "when": { "eq": [{ "data": { "query": "live", "field": "can_stop" } }, { "lit": true }] },
                          "label": "Stop" },
                "readonly": { "when": { "eq": [{ "data": { "query": "channel", "field": "readonly" } }, { "lit": true }] },
                              "reason": { "data": { "query": "channel", "field": "placeholder" } } } } })
}

fn talk_detail() -> Value {
    json!({ "stack": { "children": [
            { "header": { "title": "Run", "expand": true, "close": "close_run", "back": "Back",
                          "icons": [{ "icon_button": { "icon": "bell", "label": "Following", "action": "follow",
                                                        "icon_off": "bell-off", "label_off": "Follow",
                                                        "pressed": { "data": { "query": "channel", "field": "following" } } } }] } },
            { "strip": { "query": "live", "rows": "runs", "key": "run", "title": "agent",
                         "label": { "data": { "query": "live", "field": "label" } },
                         "tile": { "icon": "icon", "engine": "engine" },
                         "state": { "field": "state", "live": ["working", "starting"], "waiting": ["waiting"],
                                    "starting": ["starting"] },
                         "since": "since", "until": "until", "sub": "sub", "countdown_text": "ends in {left}",
                         "open": "open_run", "jump": { "field": "block", "label": "Jump to turn" },
                         "buttons": [{ "label": "Stop", "icon": "square", "style": "stop", "action": "stop_run",
                                       "visible": { "eq": [{ "row": "state" }, { "lit": "working" }] } }] } },
            { "process": { "query": "run", "state": "status", "label": "title", "rows": "steps" } },
            { "agent_run": { "query": "run", "title": "title", "status": { "field": "status" }, "steps": "steps",
                             "step_text": "title", "step_state": "kind", "log": true, "note": "note",
                             "step_detail": "detail" } },
            { "list": { "query": "people", "title": "name", "search": "Find people", "online": "online", "limit": 5,
                        "group_label": "People" } }
          ] }, "visible": { "not": { "empty": { "param": "run" } } } })
}

fn talk_ui() -> Value {
    json!({
      "catalog": 3,
      "screens": [{
        "id": "channel", "title": { "data": { "query": "channel", "field": "title" } },
        "params": [{ "name": "c", "type": "string" }, { "name": "run", "type": "string" }],
        "state": [{ "name": "tab", "type": "string", "default": "messages", "reset": ["c"] },
                  { "name": "wide", "type": "bool", "default": false, "reset": ["c"] },
                  { "name": "draft", "type": "string", "default": "" }],
        "queries": talk_queries(),
        "actions": talk_actions(),
        "overlays": [talk_overlay()],
        "layout": { "split": {
          "fill": true, "resize": { "min": 340, "max": 820, "default": 420, "keep": 400 }, "expand": { "state": "wide" },
          "list": { "stack": { "children": [
            talk_header(),
            talk_stream(),
            { "stream": { "query": "pinned", "key": "id", "anchor": "top", "older_load": "button",
                          "older_label": "Show more pinned messages",
                          "item": { "post": { "author": "who.name", "time": "time", "text": { "field": "text" } } } },
              "visible": { "eq": [{ "state": "tab" }, { "lit": "pinned" }] } },
            talk_composer()
          ] } },
          "detail": talk_detail()
        } }
      }],
      "texts": { "en": { "ask": "@{who} about this: ", "replies": "{n, plural, one {# reply} other {# replies}}",
                         "start_with": "Start with {first}" } }
    })
}

fn talk_errors(ui: Value) -> Vec<(String, String)> {
    let d: Descriptor = serde_json::from_value(talk(ui)).expect("parses");
    validate(&d)
        .into_iter()
        .map(|e| (e.code.to_string(), e.path))
        .collect()
}

fn talk_codes(ui: Value) -> Vec<String> {
    talk_errors(ui).into_iter().map(|(c, _)| c).collect()
}

/// Every catalog 3 primitive, option and step in one app-like screen: it validates, round-trips, and an older
/// client cannot show it.
#[test]
fn an_app_like_screen_of_catalog_3_validates() {
    assert_eq!(talk_errors(talk_ui()), vec![]);
    let d: Descriptor = serde_json::from_value(talk(talk_ui())).unwrap();
    let again: Descriptor =
        serde_json::from_str(&canonical_json(&serde_json::to_value(&d).unwrap())).unwrap();
    assert_eq!(again, d);
    assert_eq!(ui_warnings(&d), vec![]);
    let r = requirements(d.ui.as_ref().unwrap());
    assert_eq!(r["channel"].compat(3), "full");
    assert_eq!(r["channel"].compat(2), "unsupported");
    // the document as JSON keeps the spellings of the contract
    let v = serde_json::to_value(d.ui.as_ref().unwrap()).unwrap();
    let list = &v["screens"][0]["layout"]["split"]["list"]["stack"]["children"];
    assert_eq!(list[1]["stream"]["item"]["post"]["author"], "who.name");
    assert_eq!(list[3]["composer"]["for"], "messages");
    assert_eq!(v["screens"][0]["queries"][0]["join"][0]["as"], "who");
    assert_eq!(
        v["screens"][0]["actions"][12]["steps"][0],
        json!({ "copy": { "link": { "param": "c" } } })
    );
}

#[test]
fn catalog_3_mistakes_are_reported() {
    let at = |f: &dyn Fn(&mut Value), code: &str| {
        let mut ui = talk_ui();
        f(&mut ui);
        let c = talk_codes(ui);
        assert!(c.contains(&code.to_string()), "{code} not in {c:?}");
    };
    let list = |ui: &mut Value, i: usize| -> *mut Value {
        &mut ui["screens"][0]["layout"]["split"]["list"]["stack"]["children"][i] as *mut Value
    };
    let node = |ui: &mut Value, i: usize| unsafe { &mut *list(ui, i) };
    // a row field through a join that has no such field
    at(
        &|ui| node(ui, 1)["stream"]["item"]["post"]["author"] = json!("who.nick"),
        "E_UI_FIELD_UNKNOWN",
    );
    // a row field of no join
    at(
        &|ui| node(ui, 1)["stream"]["item"]["post"]["author"] = json!("whom.name"),
        "E_UI_FIELD_UNKNOWN",
    );
    // a key that is not a row field
    at(
        &|ui| node(ui, 1)["stream"]["key"] = json!("uid"),
        "E_UI_FIELD_UNKNOWN",
    );
    // a stream over a query without rows
    at(
        &|ui| node(ui, 1)["stream"]["query"] = json!("run"),
        "E_UI_ROWS",
    );
    // a tail of other rows
    at(
        &|ui| node(ui, 1)["stream"]["tail"]["query"] = json!("people"),
        "E_UI_ROWS",
    );
    // two bottom-anchored streams in one column
    at(
        &|ui| node(ui, 2)["stream"]["anchor"] = json!("bottom"),
        "E_UI_STREAM_TWICE",
    );
    // an unknown built-in, and one whose need is missing
    at(
        &|ui| node(ui, 1)["stream"]["item"]["post"]["hover"][0] = json!({ "builtin": "zap" }),
        "E_UI_BUILTIN",
    );
    at(
        &|ui| node(ui, 2)["stream"]["item"]["post"]["menu"] = json!([{ "builtin": "edit" }]),
        "E_UI_BUILTIN",
    );
    // an entry with both an action and a builtin
    at(
        &|ui| {
            node(ui, 1)["stream"]["item"]["post"]["menu"][0] =
                json!({ "builtin": "copy_text", "action": "ask" })
        },
        "E_UI_SHAPE",
    );
    // the post's form has no such value
    at(
        &|ui| {
            ui["screens"][0]["actions"][1]["steps"][0]["call"]["args"][1]["value"] =
                json!({ "form": "emojis" })
        },
        "E_UI_FIELD_UNKNOWN",
    );
    // the composer's form has text, …, reply and its options
    at(
        &|ui| {
            ui["screens"][0]["actions"][0]["steps"][0]["call"]["args"][4]["value"] =
                json!({ "form": "alsoo" })
        },
        "E_UI_FIELD_UNKNOWN",
    );
    // a composer for no stream; two primary composers
    at(
        &|ui| node(ui, 3)["composer"]["for"] = json!("thread"),
        "E_UI_STREAM_UNKNOWN",
    );
    at(
        &|ui| {
            let c = node(ui, 3).clone();
            node(ui, 0)["header"]["title"] = json!("x");
            ui["screens"][0]["layout"]["split"]["list"]["stack"]["children"]
                .as_array_mut()
                .unwrap()
                .push(c);
        },
        "E_UI_DUP",
    );
    // the draft is mirrored into a string state
    at(
        &|ui| node(ui, 3)["composer"]["draft_state"] = json!("wide"),
        "E_UI_TYPE",
    );
    // a merge of another message into the rows
    at(
        &|ui| ui["screens"][0]["actions"][0]["steps"][1]["merge"]["query"] = json!("people"),
        "E_UI_ARG_TYPE",
    );
    // copy takes one of value and link
    at(
        &|ui| ui["screens"][0]["actions"][12]["steps"][0] = json!({ "copy": {} }),
        "E_UI_SHAPE",
    );
    // the tabs' state starts on a tab
    at(
        &|ui| ui["screens"][0]["state"][0]["default"] = json!("files"),
        "E_UI_STATE_DEFAULT",
    );
    // too many tabs
    at(
        &|ui| {
            node(ui, 0)["header"]["tabs"]["items"] = json!(
                (0..9)
                    .map(|i| json!({ "id": format!("t{i}"), "title": "T" }))
                    .collect::<Vec<_>>()
            )
        },
        "E_UI_LIMIT",
    );
    // a segment of two kinds
    at(
        &|ui| node(ui, 0)["header"]["subtitle"][1]["text"] = json!("x"),
        "E_UI_SHAPE",
    );
    // a resize out of range, an expand of a string state
    at(
        &|ui| ui["screens"][0]["layout"]["split"]["resize"]["min"] = json!(100),
        "E_UI_LIMIT",
    );
    at(
        &|ui| ui["screens"][0]["layout"]["split"]["expand"]["state"] = json!("tab"),
        "E_UI_TYPE",
    );
    // a state reset by an unknown param
    at(
        &|ui| ui["screens"][0]["state"][0]["reset"] = json!(["channel"]),
        "E_UI_PARAM_UNKNOWN",
    );
    // query options
    at(
        &|ui| ui["screens"][0]["queries"][0]["older"]["arg"] = json!("token"),
        "E_UI_FIELD_UNKNOWN",
    );
    at(
        &|ui| ui["screens"][0]["queries"][4]["debounce_ms"] = json!(5000),
        "E_UI_LIMIT",
    );
    at(
        &|ui| ui["screens"][0]["queries"][0]["join"][0]["rows"] = json!("title"),
        "E_UI_ROWS",
    );
    at(
        &|ui| ui["screens"][0]["queries"][0]["join"][0]["as"] = json!("text"),
        "E_UI_NAME",
    );
    // a fetch of a call names the request field of the call
    at(
        &|ui| {
            node(ui, 1)["stream"]["item"]["post"]["process"]["fetch_call"]["call"] = json!("calls")
        },
        "E_UI_FIELD_UNKNOWN",
    );
    // a pick over a directory that has no such field; the pick's derived values
    at(
        &|ui| {
            ui["screens"][0]["overlays"][0]["layout"]["form"]["sections"][0]["inputs"][0]["pick"]
                ["label"] = json!("title")
        },
        "E_UI_FIELD_UNKNOWN",
    );
    at(
        &|ui| {
            ui["screens"][0]["overlays"][0]["layout"]["form"]["notes"][0]["visible"] =
                json!({ "gt": [{ "form": "to.people" }, { "lit": 0 }] })
        },
        "E_UI_FIELD_UNKNOWN",
    );
    // a toggle's action reads form.pressed
    at(
        &|ui| {
            ui["screens"][0]["actions"][13]["steps"][0]["call"]["args"][1]["value"] =
                json!({ "form": "follow" })
        },
        "E_UI_FIELD_UNKNOWN",
    );
    // a header's icons are icon buttons
    at(
        &|ui| {
            ui["screens"][0]["layout"]["split"]["detail"]["stack"]["children"][0]["header"]["icons"]
                [0] = json!({ "text": { "text": "x" } })
        },
        "E_UI_SHAPE",
    );
    // a strip's row field
    at(
        &|ui| {
            ui["screens"][0]["layout"]["split"]["detail"]["stack"]["children"][1]["strip"]["jump"]
                ["field"] = json!("blok")
        },
        "E_UI_FIELD_UNKNOWN",
    );
    // the log's step fields are fields of a step
    at(
        &|ui| {
            ui["screens"][0]["layout"]["split"]["detail"]["stack"]["children"][3]["agent_run"]["step_detail"] =
                json!("note")
        },
        "E_UI_FIELD_UNKNOWN",
    );
    // a list's phone limit
    at(
        &|ui| {
            ui["screens"][0]["layout"]["split"]["detail"]["stack"]["children"][4]["list"]["limit"] =
                json!(0)
        },
        "E_UI_LIMIT",
    );
}

/// The options of catalog 3 on older primitives need a client of catalog 3: a display option degrades, an
/// interactive one is unsupported.
#[test]
fn catalog_3_options_need_a_catalog_3_client() {
    let mut ui = good_ui();
    let fields = ui["screens"][1]["layout"].clone();
    ui["screens"][1]["layout"] = json!({ "stack": { "children": [fields,
        { "text": { "text": { "data": { "query": "order", "field": "number" } }, "format": { "elapsed": {} } } }] } });
    let d: Descriptor = serde_json::from_value(shop(ui)).unwrap();
    assert_eq!(validate(&d), vec![]);
    let r = requirements(d.ui.as_ref().unwrap());
    assert_eq!(r["order"].compat(2), "degraded");
    let mut ui = good_ui();
    ui["screens"][1]["actions"][0]["steps"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "scroll_to": { "key": { "param": "id" } } }));
    let d: Descriptor = serde_json::from_value(shop(ui)).unwrap();
    assert_eq!(validate(&d), vec![]);
    assert_eq!(
        requirements(d.ui.as_ref().unwrap())["order"].compat(2),
        "unsupported"
    );
}

/// The deprecated chat node: a warning, and no new version may start using it.
#[test]
fn the_chat_node_is_deprecated() {
    let chat = |with: bool| {
        let layout = if with {
            json!({ "chat": { "channel": "channel", "actions": { "send": "send" } } })
        } else {
            json!({ "stream": { "query": "channel", "key": "id",
                                "item": { "post": { "author": "by", "time": "time" } } } })
        };
        let mut v = talk(json!({ "catalog": 3, "screens": [{
            "id": "channel", "title": "Chat",
            "queries": [{ "name": "channel", "endpoint": "GetChannel", "rows": "messages" }],
            "actions": [{ "name": "send", "steps": [{ "call": { "endpoint": "PostMessage", "args": [
                { "field": "text", "value": { "form": "text" } }] } }] }],
            "layout": layout }] }));
        // the chat node reads a message's author
        v["messages"][6]["fields"]
            .as_array_mut()
            .unwrap()
            .push(f("author", 30, "string"));
        let d: Descriptor = serde_json::from_value(v).unwrap();
        d
    };
    let old = chat(true);
    assert_eq!(validate(&old), vec![]);
    let w = ui_warnings(&old);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].code, "W_UI_DEPRECATED");
    assert_eq!(w[0].path, "ui.screens[channel].layout.chat");
    // keeping it is fine; adding it is refused; replacing it is not breaking
    let new = chat(false);
    let codes = |o: &Descriptor, n: &Descriptor| -> Vec<&'static str> {
        crate::ui_check::ui_changes(o, n)
            .into_iter()
            .map(|c| c.code)
            .collect()
    };
    assert!(codes(&old, &old).is_empty());
    assert_eq!(codes(&new, &old), vec!["E_UI_DEPRECATED_ADDED"]);
    assert!(codes(&old, &new).is_empty());
}

/// D10: an absent value equals the protobuf default in `eq`, `ne` and `in` (a view omits empty fields).
#[test]
fn an_absent_value_equals_the_default() {
    use crate::ui::{Cond, Value as V, holds};
    let row = json!({ "name": "Ada", "count": "3", "agent": true });
    let get = |v: &V| -> Option<Value> {
        match v {
            V::Lit(x) => Some(x.clone()),
            V::Row(f) => row.get(f).cloned(),
            _ => None,
        }
    };
    let eq = |f: &str, lit: Value| Cond::Eq(V::Row(f.into()), V::Lit(lit));
    // absent equals "", 0, "0", false
    for d in [json!(""), json!(0), json!("0"), json!(false), Value::Null] {
        assert!(holds(&eq("missing", d.clone()), &get), "{d}");
        assert!(!holds(&Cond::Ne(V::Row("missing".into()), V::Lit(d)), &get));
    }
    // but not a value
    assert!(!holds(&eq("missing", json!("x")), &get));
    assert!(!holds(&eq("missing", json!(true)), &get));
    assert!(holds(
        &Cond::In {
            value: V::Row("missing".into()),
            set: vec![json!("a"), json!(false)]
        },
        &get
    ));
    assert!(!holds(
        &Cond::In {
            value: V::Row("missing".into()),
            set: vec![json!("a")]
        },
        &get
    ));
    // present values compare as before: int64 strings as numbers, text case-insensitively
    assert!(holds(&eq("count", json!(3)), &get));
    assert!(holds(&eq("name", json!("ada")), &get));
    assert!(holds(&eq("agent", json!(true)), &get));
    assert!(!holds(&eq("agent", json!(false)), &get));
    // gt and lt are false with an absent side; empty holds for absent
    assert!(!holds(
        &Cond::Gt(V::Row("missing".into()), V::Lit(json!(-1))),
        &get
    ));
    assert!(holds(
        &Cond::Gt(V::Row("count".into()), V::Lit(json!(2))),
        &get
    ));
    assert!(holds(&Cond::Empty(V::Row("missing".into())), &get));
}

/// `talk` with files: `Msg.files` and `PostRequest.files` (lists of files), the composer's attach tool and the
/// post's files bound to them, then changed by `f`.
fn talk_files(f: &dyn Fn(&mut Value, &mut Value)) -> Vec<String> {
    let mut ui = talk_ui();
    let mut d = talk(Value::Null);
    for (m, n) in [("Msg", 21), ("PostRequest", 6)] {
        let msgs = d["messages"].as_array_mut().unwrap();
        let msg = msgs.iter_mut().find(|x| x["name"] == m).unwrap();
        msg["fields"]
            .as_array_mut()
            .unwrap()
            .push(rep("files", n, "file"));
    }
    let list = &mut ui["screens"][0]["layout"]["split"]["list"]["stack"]["children"];
    list[3]["composer"]["tools"] = json!(["attach"]);
    list[3]["composer"]["files"] = json!("files");
    list[1]["stream"]["item"]["post"]["files"] = json!("files");
    f(&mut d, &mut ui);
    d["ui"] = ui;
    let d: Descriptor = serde_json::from_value(d).expect("parses");
    validate(&d)
        .into_iter()
        .map(|e| e.code.to_string())
        .collect()
}

#[test]
fn files_in_a_composer_and_a_post() {
    assert_eq!(talk_files(&|_, _| {}), Vec::<String>::new());
    fn composer(ui: &mut Value) -> &mut Value {
        &mut ui["screens"][0]["layout"]["split"]["list"]["stack"]["children"][3]["composer"]
    }
    let at = |f: &dyn Fn(&mut Value, &mut Value), code: &str| {
        let c = talk_files(f);
        assert!(c.contains(&code.to_string()), "{code} not in {c:?}");
    };
    // the attach tool names its field; the field belongs to the attach tool
    at(&|_, ui| composer(ui)["files"] = Value::Null, "E_UI_SHAPE");
    at(
        &|_, ui| composer(ui)["tools"] = json!(["emoji"]),
        "E_UI_SHAPE",
    );
    // a field the send action's request does not have, or one that holds no files
    at(
        &|_, ui| composer(ui)["files"] = json!("attachments"),
        "E_UI_FIELD_UNKNOWN",
    );
    at(
        &|_, ui| composer(ui)["files"] = json!("text"),
        "E_UI_ARG_TYPE",
    );
    // a post's files are a file field of the row
    at(
        &|_, ui| {
            ui["screens"][0]["layout"]["split"]["list"]["stack"]["children"][1]["stream"]["item"]
                ["post"]["files"] = json!("text")
        },
        "E_UI_ARG_TYPE",
    );
    at(
        &|_, ui| {
            ui["screens"][0]["layout"]["split"]["list"]["stack"]["children"][1]["stream"]["item"]
                ["post"]["files"] = json!("scans")
        },
        "E_UI_FIELD_UNKNOWN",
    );
    // a file field takes files, never text, and text takes no files
    let push_arg = |ui: &mut Value, action: usize, arg: Value| {
        ui["screens"][0]["actions"][action]["steps"][0]["call"]["args"]
            .as_array_mut()
            .unwrap()
            .push(arg)
    };
    at(
        &|_, ui| {
            push_arg(
                ui,
                0,
                json!({ "field": "files", "value": { "form": "text" } }),
            )
        },
        "E_UI_ARG_TYPE",
    );
    at(
        &|_, ui| {
            push_arg(
                ui,
                0,
                json!({ "field": "files", "value": { "lit": "0193" } }),
            )
        },
        "E_UI_ARG_TYPE",
    );
    at(
        &|_, ui| {
            push_arg(
                ui,
                3,
                json!({ "field": "text", "value": { "row": "files" } }),
            )
        },
        "E_UI_ARG_TYPE",
    );
    // the JSON keeps its spelling
    let d: Descriptor = {
        let mut d = talk(Value::Null);
        let mut ui = talk_ui();
        ui["screens"][0]["layout"]["split"]["list"]["stack"]["children"][3]["composer"]["tools"] =
            json!(["attach"]);
        ui["screens"][0]["layout"]["split"]["list"]["stack"]["children"][3]["composer"]["files"] =
            json!("files");
        d["ui"] = ui;
        serde_json::from_value(d).unwrap()
    };
    let v = serde_json::to_value(d.ui.as_ref().unwrap()).unwrap();
    let c = &v["screens"][0]["layout"]["split"]["list"]["stack"]["children"][3]["composer"];
    assert_eq!(
        (&c["tools"], &c["files"]),
        (&json!(["attach"]), &json!("files"))
    );
}

/// `shop` with files: `CancelOrderRequest.scans` (a list) and `Order.receipt`, then changed by `f`.
fn shop_files(f: &dyn Fn(&mut Value)) -> Vec<(String, String)> {
    let mut ui = good_ui();
    ui["screens"][1]["overlays"][0]["layout"]["form"]["sections"][0]["inputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "scans", "control": "upload", "label": "Scans", "multiple": true }));
    ui["screens"][0]["layout"]["stack"]["children"][1]["table"]["columns"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "field": "receipt", "title": "Receipt", "format": { "file": {} } }));
    let mut d = shop(ui);
    for (m, field) in [
        ("CancelOrderRequest", rep("scans", 3, "file")),
        (
            "Order",
            json!({ "name": "receipt", "number": 6, "type": "file", "optional": true }),
        ),
    ] {
        let msgs = d["messages"].as_array_mut().unwrap();
        let msg = msgs.iter_mut().find(|x| x["name"] == m).unwrap();
        msg["fields"].as_array_mut().unwrap().push(field);
    }
    f(&mut d);
    let d: Descriptor = serde_json::from_value(d).expect("parses");
    validate(&d)
        .into_iter()
        .map(|e| (e.code.to_string(), e.path))
        .collect()
}

#[test]
fn an_upload_input_and_a_file_column() {
    assert_eq!(shop_files(&|_| {}), vec![]);
    fn inputs(d: &mut Value) -> &mut Value {
        &mut d["ui"]["screens"][1]["overlays"][0]["layout"]["form"]["sections"][0]["inputs"]
    }
    let has = |errs: Vec<(String, String)>, code: &str| {
        assert!(
            errs.iter().any(|(c, _)| c == code),
            "{code} not in {errs:?}"
        );
    };
    // an upload fills a file field, and a file field is filled by an upload
    has(
        shop_files(&|d| inputs(d)[0]["control"] = json!("upload")),
        "E_UI_ARG_TYPE",
    );
    has(
        shop_files(&|d| inputs(d)[1]["control"] = json!("text")),
        "E_UI_ARG_TYPE",
    );
    // an upload carries its field's options; other inputs have none
    has(
        shop_files(&|d| inputs(d)[1]["multiple"] = json!(false)),
        "E_UI_ARG_TYPE",
    );
    has(
        shop_files(&|d| inputs(d)[1]["max_size"] = json!(10)),
        "E_UI_ARG_TYPE",
    );
    has(
        shop_files(&|d| inputs(d)[0]["accept"] = json!(["text/csv"])),
        "E_UI_SHAPE",
    );
    // an upload is no filter
    has(
        shop_files(&|d| {
            d["ui"]["screens"][0]["layout"]["stack"]["children"][0]["filter_bar"]["inputs"][0]["control"] =
                json!("upload")
        }),
        "E_UI_SHAPE",
    );
    // the file format shows a file field
    has(
        shop_files(&|d| {
            d["ui"]["screens"][0]["layout"]["stack"]["children"][1]["table"]["columns"][0]["format"] =
                json!({ "file": {} })
        }),
        "E_UI_ARG_TYPE",
    );
}

#[test]
fn a_download_step_takes_one_file_of_the_answer_before_it() {
    fn steps(d: &mut Value) -> &mut Vec<Value> {
        d["ui"]["screens"][0]["actions"][1]["steps"]
            .as_array_mut()
            .unwrap()
    }
    let with = |at: usize, field: &str| {
        let field = field.to_string();
        move |d: &mut Value| steps(d).insert(at, json!({ "download": field }))
    };
    // after the call: `receipt` of its answer (Order)
    assert_eq!(shop_files(&with(2, "receipt")), vec![]);
    let only =
        |errs: Vec<(String, String)>| -> Vec<String> { errs.into_iter().map(|(c, _)| c).collect() };
    // before any call, and in on_error, there is no answer to read
    assert_eq!(
        only(shop_files(&with(0, "receipt"))),
        vec!["E_UI_REF_CONTEXT"]
    );
    assert_eq!(
        only(shop_files(&|d| {
            d["ui"]["screens"][0]["actions"][1]["on_error"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "download": "receipt" }))
        })),
        vec!["E_UI_REF_CONTEXT"]
    );
    // a field the answer has not, a field that is not a file, a list of files
    assert_eq!(
        only(shop_files(&with(2, "nope"))),
        vec!["E_UI_FIELD_UNKNOWN"]
    );
    assert_eq!(only(shop_files(&with(2, "number"))), vec!["E_UI_ARG_TYPE"]);
    assert_eq!(
        only(shop_files(&|d| {
            let msgs = d["messages"].as_array_mut().unwrap();
            let order = msgs.iter_mut().find(|x| x["name"] == "Order").unwrap();
            order["fields"]
                .as_array_mut()
                .unwrap()
                .push(rep("copies", 7, "file"));
            steps(d).insert(2, json!({ "download": "copies" }));
        })),
        vec!["E_UI_ARG_TYPE"]
    );
    // the JSON shape: the step's value is the path
    let s: crate::ui::Step = serde_json::from_value(json!({ "download": "receipt" })).unwrap();
    assert_eq!(s, crate::ui::Step::Download("receipt".into()));
    assert_eq!(s.name(), "download");
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        json!({ "download": "receipt" })
    );
}

// ------------------------------------------------------------------------------------------ catalog 4

/// The report shop with quarters, sales at a quarter, places on a map, a maps connection and a dependency.
fn travel(ui: Value) -> Value {
    let mut v = report(ui);
    v["messages"].as_array_mut().unwrap().extend([
        json!({ "name": "Quarter", "fields": [f("key", 1, "string"), f("label", 2, "string"), f("ends", 3, "string"),
                                              f("total", 4, "decimal"), f("running", 5, "bool")] }),
        json!({ "name": "QuartersResponse", "fields": [rep("quarters", 1, "message:Quarter")] }),
        json!({ "name": "SalesAtRequest", "fields": [f("quarter", 1, "string"), f("window", 2, "int32"),
                                                     f("area", 3, "string"), f("upto", 4, "int32")] }),
        json!({ "name": "Place", "fields": [f("lat", 1, "double"), f("lon", 2, "double"), f("value", 3, "decimal"),
                                            f("name", 4, "string"), f("id", 5, "string"), f("shape", 6, "json"),
                                            f("state", 7, "string"), f("ours", 8, "bool")] }),
        json!({ "name": "PlacesResponse", "fields": [rep("places", 1, "message:Place")] }),
    ]);
    let place = v["messages"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|m| m["name"] == "Place")
        .unwrap();
    place["fields"]
        .as_array_mut()
        .unwrap()
        .push(rep("labels", 10, "double"));
    v["endpoints"].as_array_mut().unwrap().extend([
        json!({ "name": "Quarters", "input": "Empty", "output": "QuartersResponse", "scopes": ["read"], "read_only": true, "ui": true }),
        json!({ "name": "SalesAt", "input": "SalesAtRequest", "output": "SalesResponse", "scopes": ["read"], "read_only": true, "ui": true }),
        json!({ "name": "Places", "input": "SalesAtRequest", "output": "PlacesResponse", "scopes": ["read"], "read_only": true, "ui": true }),
    ]);
    v["connections"] =
        json!([{ "name": "basemap", "kind": "maps" }, { "name": "model", "kind": "llm" }]);
    v["dependencies"] = json!([{ "plugin": "geo", "calls": [] }]);
    v
}

/// `report_ui` with a screen `travel` whose layout is `node`: params quarter, window, then and area; queries
/// over them.
fn travel_ui(node: Value) -> Value {
    let mut ui = report_ui(json!({ "text": { "text": "Report" } }));
    ui["catalog"] = json!(4);
    ui["screens"].as_array_mut().unwrap().push(json!({
      "id": "travel", "title": "Travel",
      "params": [{ "name": "quarter", "type": "string" }, { "name": "window", "type": "int32" },
                 { "name": "then", "type": "string" }, { "name": "area", "type": "string" },
                 { "name": "upto", "type": "int32" }],
      "state": [{ "name": "picked", "type": "string" }],
      "queries": [
        { "name": "quarters", "endpoint": "Quarters", "rows": "quarters" },
        { "name": "sales", "endpoint": "SalesAt", "rows": "days",
          "args": [{ "field": "quarter", "value": { "param": "quarter" } }, { "field": "window", "value": { "param": "window" } },
                   { "field": "upto", "value": { "param": "upto" } }] },
        { "name": "places", "endpoint": "Places", "rows": "places",
          "args": [{ "field": "quarter", "value": { "param": "quarter" } }, { "field": "area", "value": { "param": "area" } }] },
        { "name": "whole", "endpoint": "SalesAt", "rows": "days",
          "args": [{ "field": "quarter", "value": { "param": "quarter" } }] },
        { "name": "still", "endpoint": "Places", "rows": "places" },
        { "name": "all_sales", "endpoint": "SalesAt", "rows": "days" }
      ],
      "layout": node
    }));
    ui
}

fn travel_errors(node: Value) -> Vec<(String, String)> {
    let d: Descriptor = serde_json::from_value(travel(travel_ui(node))).expect("parses");
    validate(&d)
        .into_iter()
        .map(|e| (e.code.to_string(), e.path))
        .collect()
}

/// Bars by channel with time travel over the quarters, compared with a year before.
fn timed_bars() -> Value {
    json!({ "viz": {
      "family": "bar", "kind": "bar", "title": "Revenue by channel",
      "bind": [{ "to": "rows", "from": { "rows": { "query": "sales", "map": { "name": "channel", "value": "total" }, "text": ["name"] } } }],
      "time": {
        "read": "flow", "control": "brush",
        "periods": { "query": "quarters", "value": "key", "label": "label", "track": "total", "partial": "running" },
        "at": { "param": "quarter" }, "window": { "param": "window" }, "then": { "param": "then" }, "grain": "quarter"
      }
    } })
}

/// A map over the places: points sized by value, hexagons, regions, an area the member draws.
fn map() -> Value {
    json!({ "viz": {
      "family": "geo", "kind": "map", "title": "Deals", "connection": "basemap",
      "options": { "view": { "center": [-118.28, 33.97], "zoom": 10 } },
      "layers": [
        { "type": "points", "query": "places", "title": "Deals",
          "fields": { "lat": "lat", "lon": "lon", "value": "value", "name": "name", "id": "id" },
          "options": { "size": true, "fmt": "money" }, "select": { "state": "picked" } },
        { "type": "hex", "query": "places", "fields": { "lat": "lat", "lon": "lon", "value": "value" }, "options": { "radius": 14 } },
        { "type": "regions", "query": "still", "fields": { "shape": "shape", "value": "value", "name": "name" },
          "options": { "mode": "diverging" } },
        { "type": "change", "query": "places", "fields": { "lat": "lat", "lon": "lon", "state": "state" } },
        { "type": "share", "query": "places", "fields": { "lat": "lat", "lon": "lon", "ours": "ours" } },
        { "type": "area", "title": "Drawn area", "select": { "param": "area" } }
      ],
      "time": { "read": "flow", "periods": { "query": "quarters", "value": "key", "label": "label" },
                "at": { "param": "quarter" }, "then": { "param": "then" } }
    } })
}

#[test]
fn time_travel_validates_round_trips_and_needs_catalog_4() {
    assert_eq!(travel_errors(timed_bars()), vec![]);
    let d: Descriptor = serde_json::from_value(travel(travel_ui(timed_bars()))).unwrap();
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["ui"]["screens"][3]["layout"], timed_bars());
    let again: Descriptor = serde_json::from_str(&canonical_json(&v)).unwrap();
    assert_eq!(again, d);
    let r = requirements(d.ui.as_ref().unwrap());
    // a display part: an older client draws the fallback (the card without time travel is not drawn)
    assert_eq!(r["travel"].compat(3), "degraded");
    assert_eq!(r["travel"].compat(4), "full");
    assert_eq!(r["report"].compat(3), "full");
    // the smallest: a read, the periods' value and label, and `at`
    let mut small = timed_bars();
    small["viz"]["time"] = json!({ "read": "state", "periods": { "query": "quarters", "value": "key", "label": "label" },
                                   "at": { "param": "quarter" } });
    assert_eq!(travel_errors(small), vec![]);
    // unknown reads, controls and fields do not parse
    for bad in [
        json!({ "read": "sum", "periods": { "query": "quarters", "value": "key", "label": "label" }, "at": { "param": "quarter" } }),
        json!({ "read": "flow", "control": "dial", "periods": { "query": "quarters", "value": "key", "label": "label" }, "at": { "param": "quarter" } }),
        json!({ "read": "flow", "periods": { "query": "quarters", "value": "key", "label": "label" }, "at": { "param": "quarter" }, "today": 3 }),
    ] {
        let mut n = timed_bars();
        n["viz"]["time"] = bad.clone();
        assert!(
            serde_json::from_value::<Descriptor>(travel(travel_ui(n))).is_err(),
            "{bad}"
        );
    }
}

/// `timed_bars` that compares with the period before first and cuts a running quarter at the same day.
fn cut_bars() -> Value {
    let mut n = timed_bars();
    n["viz"]["time"]["compare"] = json!("previous");
    n["viz"]["time"]["upto"] = json!({ "param": "upto" });
    n
}

#[test]
fn time_compare_and_cut_validate_and_round_trip() {
    assert_eq!(travel_errors(cut_bars()), vec![]);
    let d: Descriptor = serde_json::from_value(travel(travel_ui(cut_bars()))).unwrap();
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["ui"]["screens"][3]["layout"], cut_bars());
    assert_eq!(
        v["ui"]["screens"][3]["layout"]["viz"]["time"]["upto"],
        json!({ "param": "upto" })
    );
    // still catalog 4: both are optional, a document without them is as before
    let r = requirements(d.ui.as_ref().unwrap());
    assert_eq!(r["travel"].compat(4), "full");
    let mut year = cut_bars();
    year["viz"]["time"]["compare"] = json!("year");
    assert_eq!(travel_errors(year), vec![]);
    // only year and previous; the person picks a period in the menu, never the plugin
    let mut picked = cut_bars();
    picked["viz"]["time"]["compare"] = json!("picked");
    assert!(serde_json::from_value::<Descriptor>(travel(travel_ui(picked))).is_err());
}

#[test]
fn time_compare_and_cut_are_checked() {
    let at = "ui.screens[travel].layout.viz.time";
    let one = |change: &dyn Fn(&mut Value), want: &[(&str, &str)]| {
        let mut n = cut_bars();
        change(&mut n["viz"]);
        let want: Vec<(String, String)> = want
            .iter()
            .map(|(c, p)| (c.to_string(), format!("{at}{p}")))
            .collect();
        assert_eq!(travel_errors(n), want);
    };
    one(
        &|v| v["time"]["upto"] = json!({ "param": "ghost" }),
        &[("E_UI_PARAM_UNKNOWN", ".upto")],
    );
    one(
        &|v| v["time"]["upto"] = json!({ "param": "area" }),
        // a string, and the sales do not pass it either
        &[("E_UI_ARG_TYPE", ".upto"), ("E_UI_VIZ_TIME", ".upto")],
    );
    one(
        &|v| v["time"]["upto"] = json!({ "param": "window" }),
        &[("E_UI_VIZ_TIME", "")],
    );
    // the cut must reach the card's data: these sales do not pass it
    one(
        &|v| v["bind"][0]["from"]["rows"]["query"] = json!("whole"),
        &[("E_UI_VIZ_TIME", ".upto")],
    );
    one(
        &|v| {
            v["time"]["periods"]
                .as_object_mut()
                .unwrap()
                .remove("partial");
        },
        &[("E_UI_VIZ_TIME", ".periods.partial")],
    );
    one(
        &|v| {
            v["time"].as_object_mut().unwrap().remove("grain");
        },
        &[("E_UI_VIZ_TIME", ".grain")],
    );
    one(
        &|v| {
            v["time"]["read"] = json!("state");
            v["time"].as_object_mut().unwrap().remove("window");
        },
        &[("E_UI_VIZ_TIME", ".upto")],
    );
    one(
        &|v| {
            v["time"].as_object_mut().unwrap().remove("then");
        },
        &[("E_UI_VIZ_TIME", ".compare")],
    );
    one(
        &|v| {
            v["time"]["compare"] = json!("year");
            v["time"].as_object_mut().unwrap().remove("grain");
            v["time"].as_object_mut().unwrap().remove("upto");
        },
        &[("E_UI_VIZ_TIME", ".compare")],
    );
    // the period before needs no grain; a cut without a comparison is fine
    let mut n = cut_bars();
    n["viz"]["time"].as_object_mut().unwrap().remove("grain");
    n["viz"]["time"].as_object_mut().unwrap().remove("upto");
    assert_eq!(travel_errors(n), vec![]);
    let mut n = cut_bars();
    n["viz"]["time"].as_object_mut().unwrap().remove("then");
    n["viz"]["time"].as_object_mut().unwrap().remove("compare");
    assert_eq!(travel_errors(n), vec![]);
}

#[test]
fn a_line_compares_with_the_earlier_periods_line() {
    let mut n = timed_bars();
    n["viz"]["family"] = json!("line");
    n["viz"]["kind"] = json!("line");
    n["viz"]["time"]["control"] = json!("stepper");
    n["viz"]["time"].as_object_mut().unwrap().remove("window");
    assert_eq!(travel_errors(n), vec![]);
}

#[test]
fn time_travel_is_checked() {
    let at = "ui.screens[travel].layout.viz.time";
    let one = |change: &dyn Fn(&mut Value), code: &str, path: &str| {
        let mut n = timed_bars();
        change(&mut n["viz"]);
        assert_eq!(
            travel_errors(n),
            vec![(code.to_string(), format!("{at}{path}"))],
            "{code} at {path}"
        );
    };
    one(
        &|v| v["time"]["periods"]["query"] = json!("ghost"),
        "E_UI_QUERY_UNKNOWN",
        ".periods.query",
    );
    one(
        &|v| v["time"]["periods"]["label"] = json!("name"),
        "E_UI_FIELD_UNKNOWN",
        ".periods.label",
    );
    one(
        &|v| v["time"]["periods"]["track"] = json!("label"),
        "E_UI_ARG_TYPE",
        ".periods.track",
    );
    one(
        &|v| v["time"]["periods"]["partial"] = json!("total"),
        "E_UI_ARG_TYPE",
        ".periods.partial",
    );
    one(
        &|v| v["time"]["at"] = json!({ "param": "ghost" }),
        "E_UI_PARAM_UNKNOWN",
        ".at",
    );
    one(
        &|v| v["time"]["then"] = json!({ "state": "ghost" }),
        "E_UI_STATE_UNKNOWN",
        ".then",
    );
    one(
        &|v| v["time"]["window"] = json!({ "param": "area" }),
        "E_UI_ARG_TYPE",
        ".window",
    );
    one(
        &|v| v["time"]["then"] = json!({ "param": "quarter" }),
        "E_UI_VIZ_TIME",
        "",
    );
    // the period must reach the card's data
    one(
        &|v| v["bind"][0]["from"]["rows"]["query"] = json!("all_sales"),
        "E_UI_VIZ_TIME",
        ".at",
    );
    // what the kind can draw
    one(
        &|v| {
            v["kind"] = json!("diverging");
        },
        "E_UI_VIZ_TIME",
        ".then",
    );
    one(
        &|v| {
            v["time"]["control"] = json!("axis");
            v["time"].as_object_mut().unwrap().remove("then");
        },
        "E_UI_VIZ_TIME",
        ".control",
    );
    one(
        &|v| {
            v["time"]["control"] = json!("screen");
        },
        "E_UI_VIZ_TIME",
        ".then",
    );
    one(
        &|v| {
            v["time"]["read"] = json!("state");
        },
        "E_UI_VIZ_TIME",
        ".window",
    );
    one(
        &|v| {
            v["time"]["read"] = json!("scheduled");
            v["time"]["periods"]
                .as_object_mut()
                .unwrap()
                .remove("partial");
        },
        "E_UI_VIZ_TIME",
        ".periods.partial",
    );
    one(
        &|v| {
            v["family"] = json!("table");
            v["kind"] = json!("grid");
        },
        "E_UI_VIZ_TIME",
        ".key",
    );
    // the axis control on a kind with its own time axis, and a grid that names its compared column, are fine
    let mut n = timed_bars();
    n["viz"]["kind"] = json!("column");
    n["viz"]["time"]["control"] = json!("axis");
    n["viz"]["time"].as_object_mut().unwrap().remove("then");
    assert_eq!(travel_errors(n), vec![]);
    let mut n = timed_bars();
    n["viz"]["family"] = json!("table");
    n["viz"]["kind"] = json!("grid");
    n["viz"]["time"]["key"] = json!(1);
    assert_eq!(travel_errors(n), vec![]);
    // a home widget has no params or state to hold the period
    let mut ui = travel_ui(json!({ "text": { "text": "x" } }));
    ui["queries"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "q", "endpoint": "Quarters", "rows": "quarters" }));
    ui["home"] = json!([{ "id": "w", "title": "W", "node": { "viz": {
        "family": "figure", "kind": "metric",
        "time": { "read": "flow", "periods": { "query": "q", "value": "key", "label": "label" }, "at": { "param": "quarter" } } } } }]);
    let d: Descriptor = serde_json::from_value(travel(ui)).unwrap();
    let codes: Vec<&str> = validate(&d).iter().map(|e| e.code).collect();
    assert!(codes.contains(&"E_UI_PARAM_UNKNOWN"), "{codes:?}");
}

#[test]
fn a_map_validates_and_needs_catalog_4() {
    assert_eq!(travel_errors(map()), vec![]);
    let d: Descriptor = serde_json::from_value(travel(travel_ui(map()))).unwrap();
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["ui"]["screens"][3]["layout"], map());
    let r = requirements(d.ui.as_ref().unwrap());
    assert_eq!(r["travel"].compat(3), "degraded");
    assert_eq!(r["travel"].compat(4), "full");
    // a dependency's maps connection
    let mut n = map();
    n["viz"]["connection"] = json!("geo.osm");
    assert_eq!(travel_errors(n), vec![]);
    // unknown layer types do not parse
    let mut n = map();
    n["viz"]["layers"][0]["type"] = json!("pins");
    assert!(serde_json::from_value::<Descriptor>(travel(travel_ui(n))).is_err());
}

#[test]
fn a_map_is_checked() {
    let at = "ui.screens[travel].layout.viz";
    let one = |change: &dyn Fn(&mut Value), code: &str, path: &str| {
        let mut n = map();
        change(&mut n["viz"]);
        assert_eq!(
            travel_errors(n),
            vec![(code.to_string(), format!("{at}{path}"))],
            "{code} at {path}"
        );
    };
    // the connection: a maps connection of this plugin or of a dependency
    one(
        &|v| {
            v.as_object_mut().unwrap().remove("connection");
        },
        "E_UI_VIZ_CONNECTION",
        ".connection",
    );
    one(
        &|v| v["connection"] = json!("ghost"),
        "E_UI_VIZ_CONNECTION",
        ".connection",
    );
    one(
        &|v| v["connection"] = json!("model"),
        "E_UI_VIZ_CONNECTION",
        ".connection",
    );
    one(
        &|v| v["connection"] = json!("crm.osm"),
        "E_UI_VIZ_CONNECTION",
        ".connection",
    );
    one(
        &|v| v["connection"] = json!("geo.Osm"),
        "E_UI_VIZ_CONNECTION",
        ".connection",
    );
    // layers: at least one, at most eight
    one(&|v| v["layers"] = json!([]), "E_UI_VIZ_LAYER", ".layers");
    one(
        &|v| {
            let l = v["layers"][1].clone();
            v["layers"] = json!(vec![l; ui::MAP_MAX_LAYERS + 1]);
        },
        "E_UI_LIMIT",
        ".layers",
    );
    // a layer's roles, their fields and their types
    one(
        &|v| v["layers"][0]["fields"]["size"] = json!("value"),
        "E_UI_VIZ_LAYER",
        ".layers[0].fields.size",
    );
    one(
        &|v| {
            v["layers"][0]["fields"]
                .as_object_mut()
                .unwrap()
                .remove("lat");
        },
        "E_UI_VIZ_LAYER",
        ".layers[0].fields",
    );
    one(
        &|v| v["layers"][0]["fields"]["lon"] = json!("lng"),
        "E_UI_FIELD_UNKNOWN",
        ".layers[0].fields.lon",
    );
    one(
        &|v| v["layers"][0]["fields"]["lat"] = json!("name"),
        "E_UI_ARG_TYPE",
        ".layers[0].fields.lat",
    );
    one(
        &|v| v["layers"][0]["fields"]["lat"] = json!("labels"),
        "E_UI_ARG_TYPE",
        ".layers[0].fields.lat",
    );
    one(
        &|v| v["layers"][2]["fields"]["shape"] = json!("value"),
        "E_UI_ARG_TYPE",
        ".layers[2].fields.shape",
    );
    one(
        &|v| v["layers"][4]["fields"]["ours"] = json!("ghost"),
        "E_UI_FIELD_UNKNOWN",
        ".layers[4].fields.ours",
    );
    one(
        &|v| {
            v["layers"][1].as_object_mut().unwrap().remove("query");
        },
        "E_UI_VIZ_LAYER",
        ".layers[1].query",
    );
    one(
        &|v| v["layers"][1]["query"] = json!("ghost"),
        "E_UI_QUERY_UNKNOWN",
        ".layers[1].query",
    );
    // selections: declared, and not on layers where nothing is picked
    one(
        &|v| v["layers"][0]["select"] = json!({ "state": "ghost" }),
        "E_UI_STATE_UNKNOWN",
        ".layers[0].select",
    );
    one(
        &|v| v["layers"][1]["select"] = json!({ "state": "picked" }),
        "E_UI_VIZ_LAYER",
        ".layers[1].select",
    );
    // an area is drawn: no query, and somewhere to put it
    one(
        &|v| v["layers"][5]["query"] = json!("places"),
        "E_UI_VIZ_LAYER",
        ".layers[5].query",
    );
    one(
        &|v| {
            v["layers"][5].as_object_mut().unwrap().remove("select");
        },
        "E_UI_VIZ_LAYER",
        ".layers[5].select",
    );
    one(
        &|v| v["layers"][0]["title"] = json!(" "),
        "E_UI_TEXT",
        ".layers[0].title",
    );
    // only a map has a connection and layers
    let mut n = timed_bars();
    n["viz"]["connection"] = json!("basemap");
    n["viz"]["layers"] = map()["viz"]["layers"].clone();
    assert_eq!(
        travel_errors(n),
        vec![
            ("E_UI_VIZ_CONNECTION".into(), format!("{at}.connection")),
            ("E_UI_VIZ_LAYER".into(), format!("{at}.layers")),
        ]
    );
}
