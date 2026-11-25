use serde_json::{Value, json};

use super::*;

fn field(name: &str, n: u32, ty: &str) -> Value {
    json!({ "name": name, "number": n, "type": ty, "repeated": false, "optional": false, "description": "" })
}

fn col(source: &str, field: &str) -> Value {
    json!({ "column": { "source": source, "field": field } })
}

/// `retail`: a raw lake table of texts, a cleaned one computed from it (`parse`), a monthly one computed from the
/// cleaned one, the entity `product` (a masked cost, an encrypted note) with a view over it, and lake tables computed
/// from the entity and from the view; the plugin bypasses read of `product`.
fn retail() -> Value {
    json!({
      "schema": 1, "id": "retail", "version": "1.0.0",
      "roles": [{ "name": "analyst", "title": "Analyst" }, { "name": "buyer", "title": "Buyer" }],
      "capabilities": [{ "name": "authz:bypass", "reason": "models", "on": [{ "entity": "product", "ops": ["read"] }] }],
      "dependencies": [{ "plugin": "erp", "calls": [], "lake": ["stock"] }],
      "messages": [
        { "name": "StockFileRow", "kind": "message", "description": "",
          "fields": [field("loaded_on", 1, "date"), field("sku", 2, "string"), field("on_hand", 3, "string")] },
        { "name": "Stock", "kind": "message", "description": "",
          "fields": [field("day", 1, "date"), field("sku", 2, "string"), field("on_hand", 3, "decimal")] },
        { "name": "Monthly", "kind": "message", "description": "",
          "fields": [field("month", 1, "date"), field("skus", 2, "int64"), field("on_hand", 3, "decimal")] },
        { "name": "ProductDim", "kind": "message", "description": "",
          "fields": [field("id", 1, "ref:retail.product"), field("sku", 2, "string"), field("category", 3, "string")] },
        { "name": "ByCategory", "kind": "message", "description": "",
          "fields": [field("category", 1, "string"), field("products", 2, "int64")] }
      ],
      "entities": [
        { "name": "product", "message": "Product", "key": "uuid", "label": "sku", "soft_delete": false,
          "version": false, "append_only": false,
          "fields": [field("id", 1, "ref:retail.product"), field("sku", 2, "string"), field("category", 3, "string"),
                     { "name": "cost", "number": 4, "type": "decimal", "repeated": false, "optional": true,
                       "description": "" },
                     { "name": "note", "number": 5, "type": "string", "repeated": false, "optional": true,
                       "description": "", "encrypted": true }],
          "access": { "read": "authenticated", "masks": [{ "field": "cost", "roles": ["buyer"] }] } }
      ],
      "views": [
        { "name": "by_category", "message": "ByCategory", "description": "",
          "sources": [{ "entity": "product", "alias": "p" }],
          "group_by": [{ "source": "p", "field": "category" }],
          "columns": [
            { "name": "category", "value": col("p", "category") },
            { "name": "products", "value": { "aggregate": { "fn": "count" } } }
          ] }
      ],
      "layers": [{ "name": "raw", "title": "" }, { "name": "core", "title": "" }, { "name": "mart", "title": "" }],
      "lake_tables": [
        { "name": "stock_file", "layer": "raw", "message": "StockFileRow", "partition": ["loaded_on"],
          "system_read": true },
        { "name": "stock", "layer": "core", "message": "Stock", "partition": ["day"], "system_read": true },
        { "name": "monthly", "layer": "mart", "message": "Monthly", "partition": ["month"] },
        { "name": "product_dim", "layer": "core", "message": "ProductDim" },
        { "name": "category_count", "layer": "mart", "message": "ByCategory" }
      ],
      "lake_models": [
        { "table": "stock",
          "sources": [{ "entity": "stock_file", "alias": "f" }],
          "filter": { "not": { "compare": { "left": col("f", "sku"), "op": "eq", "values": [""] } } },
          "columns": [
            { "name": "day", "value": col("f", "loaded_on") },
            { "name": "sku", "value": col("f", "sku") },
            { "name": "on_hand", "value": { "column": { "source": "f", "field": "on_hand", "parse": "decimal" } } }
          ] },
        { "table": "monthly",
          "sources": [{ "entity": "stock", "alias": "s" }],
          "group_by": [{ "source": "s", "field": "day", "bucket": "month" }],
          "columns": [
            { "name": "month", "value": { "column": { "source": "s", "field": "day", "bucket": "month" } } },
            { "name": "skus", "value": { "aggregate": { "fn": "count_distinct", "column": { "source": "s", "field": "sku" } } } },
            { "name": "on_hand", "value": { "aggregate": { "fn": "sum", "column": { "source": "s", "field": "on_hand" } } } }
          ] },
        { "table": "product_dim",
          "sources": [{ "entity": "product", "alias": "p" }],
          "columns": [
            { "name": "id", "value": col("p", "id") },
            { "name": "sku", "value": col("p", "sku") },
            { "name": "category", "value": col("p", "category") }
          ] },
        { "table": "category_count",
          "sources": [{ "entity": "by_category", "alias": "b" }],
          "columns": [
            { "name": "category", "value": col("b", "category") },
            { "name": "products", "value": col("b", "products") }
          ] }
      ]
    })
}

fn codes(v: &Value) -> Vec<(&'static str, String)> {
    let d = parse(v.to_string().as_bytes()).unwrap();
    validate(&d).into_iter().map(|e| (e.code, e.path)).collect()
}

fn after(f: impl FnOnce(&mut Value)) -> Vec<(&'static str, String)> {
    let mut v = retail();
    f(&mut v);
    codes(&v)
}

fn model<'a>(v: &'a mut Value, table: &str) -> &'a mut Value {
    v["lake_models"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|m| m["table"] == table)
        .unwrap()
}

#[test]
fn models_over_lake_tables_an_entity_and_a_view_check_and_round_trip() {
    assert_eq!(codes(&retail()), vec![]);
    let d = parse(retail().to_string().as_bytes()).unwrap();
    assert_eq!(
        serde_json::to_value(&d).unwrap()["lake_models"],
        retail()["lake_models"]
    );
    let m = &d.lake_models[2];
    assert_eq!(lake_model::kind(&d, "product"), lake_model::Kind::Entity);
    assert_eq!(lake_model::kind(&d, "by_category"), lake_model::Kind::View);
    assert_eq!(lake_model::kind(&d, "stock"), lake_model::Kind::Lake);
    assert_eq!(lake_model::kind(&d, "erp.stock"), lake_model::Kind::Lake);
    assert_eq!(lake_model::kind(&d, "erp.other"), lake_model::Kind::Unknown);
    assert_eq!(lake_model::entities(&d, m), vec!["product".to_string()]);
    assert_eq!(
        lake_model::entities(&d, &d.lake_models[3]),
        vec!["product".to_string()]
    );
    // the hash follows the model and the table's columns
    let h = m.hash(&d);
    assert_eq!(h.len(), 64);
    let mut v = retail();
    model(&mut v, "product_dim")["filter"] =
        json!({ "compare": { "left": col("p", "sku"), "op": "ne", "values": [""] } });
    let d2 = parse(v.to_string().as_bytes()).unwrap();
    assert_ne!(d2.lake_models[2].hash(&d2), h);
    assert_eq!(d.lake_models[2].hash(&d), h);
    // a view under a model is part of its declaration
    let h = d.lake_models[3].hash(&d);
    let mut v = retail();
    v["views"][0]["filter"] =
        json!({ "compare": { "left": col("p", "sku"), "op": "ne", "values": [""] } });
    let d3 = parse(v.to_string().as_bytes()).unwrap();
    assert_ne!(d3.lake_models[3].hash(&d3), h);
    assert_eq!(d3.lake_models[0].hash(&d3), d.lake_models[0].hash(&d));
    // the owner's consent to other plugins' models, apart from export, written only when given
    let mut v = retail();
    v["lake_tables"][1]["export_models"] = json!(true);
    let d4 = parse(v.to_string().as_bytes()).unwrap();
    assert!(d4.lake_tables[1].export_models && !d4.lake_tables[1].export);
    assert_eq!(
        serde_json::to_value(&d4).unwrap()["lake_tables"][1]["export_models"],
        true
    );
    assert!(
        serde_json::to_value(&d).unwrap()["lake_tables"][1]
            .get("export_models")
            .is_none()
    );
    // a plugin without models writes none
    let plain = json!({ "schema": 1, "id": "plain", "version": "1.0.0" });
    let out = serde_json::to_string(&parse(plain.to_string().as_bytes()).unwrap()).unwrap();
    assert!(
        !out.contains("lake_models") && !out.contains("parse"),
        "{out}"
    );
}

#[test]
fn every_rule_of_a_model_has_its_code() {
    let has = |got: Vec<(&'static str, String)>, code: &str, path: &str| {
        assert!(
            got.iter().any(|(c, p)| *c == code && p.starts_with(path)),
            "{code} at {path}: {got:?}"
        );
    };
    // a table the plugin does not declare, and two models of one table
    has(
        after(|v| model(v, "stock")["table"] = json!("nope")),
        "E_LAKE_MODEL_TABLE",
        "lake_models[nope].table",
    );
    has(
        after(|v| model(v, "monthly")["table"] = json!("stock")),
        "E_LAKE_MODEL_TABLE",
        "lake_models[stock]",
    );
    // unknown sources: a name of nothing, another plugin's table not declared, the directory, a dependency's entity
    for source in ["nope", "erp.other", "platform.user"] {
        has(
            after(|v| model(v, "stock")["sources"][0]["entity"] = json!(source)),
            "E_LAKE_MODEL_SOURCE",
            "lake_models[stock].sources",
        );
    }
    // an unknown column, and a column of another type than the table's field
    has(
        after(|v| model(v, "product_dim")["columns"][1]["value"] = col("p", "nope")),
        "E_LAKE_MODEL_COLUMN",
        "lake_models[product_dim].columns",
    );
    has(
        after(|v| model(v, "stock")["columns"][2]["value"] = col("f", "on_hand")),
        "E_LAKE_MODEL_TYPE",
        "lake_models[stock].columns[on_hand]",
    );
    // a field of the table left without a column
    has(
        after(|v| {
            model(v, "product_dim")["columns"]
                .as_array_mut()
                .unwrap()
                .pop();
        }),
        "E_LAKE_MODEL_COLUMN",
        "lake_models[product_dim].columns",
    );
    // an encrypted column
    has(
        after(|v| {
            model(v, "product_dim")["columns"][2]["value"] = col("p", "note");
        }),
        "E_LAKE_MODEL_ENCRYPTED",
        "lake_models[product_dim].columns",
    );
    // a masked column, unless the model says unmasked; through a view naming one too
    let masked = |v: &mut Value| {
        model(v, "product_dim")["filter"] =
            json!({ "compare": { "left": col("p", "cost"), "op": "gt", "values": ["1"] } });
    };
    has(
        after(masked),
        "E_LAKE_MODEL_MASKED",
        "lake_models[product_dim]",
    );
    assert_eq!(
        after(|v| {
            masked(v);
            model(v, "product_dim")["unmasked"] = json!(true);
        }),
        vec![]
    );
    has(
        after(|v| {
            v["views"][0]["filter"] =
                json!({ "compare": { "left": col("p", "cost"), "op": "is_not_null" } });
        }),
        "E_LAKE_MODEL_MASKED",
        "lake_models[category_count]",
    );
    has(
        after(|v| model(v, "stock")["unmasked"] = json!(true)),
        "E_LAKE_MODEL_MASKED",
        "lake_models[stock].unmasked",
    );
    // the bypass of read missing, for the entity and for the view over it
    let got = after(|v| v["capabilities"] = json!([]));
    has(
        got.clone(),
        "E_LAKE_MODEL_BYPASS",
        "lake_models[product_dim]",
    );
    has(got, "E_LAKE_MODEL_BYPASS", "lake_models[category_count]");
    // an entity and a lake table in one model
    has(
        after(|v| {
            model(v, "product_dim")["sources"] = json!([
                { "entity": "product", "alias": "p" },
                { "entity": "stock", "alias": "s", "join": "inner",
                  "left": { "source": "p", "field": "sku" }, "right": { "source": "s", "field": "sku" } }
            ]);
        }),
        "E_LAKE_MODEL_MIXED",
        "lake_models[product_dim].sources",
    );
    // parse: of an entity's column, of a non-text column, to an unknown type; never in a view
    has(
        after(|v| {
            model(v, "product_dim")["columns"][1]["value"] =
                json!({ "column": { "source": "p", "field": "sku", "parse": "int64" } });
        }),
        "E_LAKE_MODEL_PARSE",
        "lake_models[product_dim].columns",
    );
    has(
        after(|v| {
            model(v, "stock")["columns"][0]["value"] =
                json!({ "column": { "source": "f", "field": "loaded_on", "parse": "date" } });
        }),
        "E_LAKE_MODEL_PARSE",
        "lake_models[stock].columns",
    );
    has(
        after(|v| {
            model(v, "stock")["columns"][2]["value"] =
                json!({ "column": { "source": "f", "field": "on_hand", "parse": "money" } });
        }),
        "E_LAKE_MODEL_PARSE",
        "lake_models[stock].columns",
    );
    has(
        after(|v| {
            v["views"][0]["columns"][0]["value"] =
                json!({ "column": { "source": "p", "field": "category", "parse": "int64" } });
        }),
        "E_VIEW_PARSE",
        "views[by_category]",
    );
    // the plugin's own lake table read without being declared read by the system context
    has(
        after(|v| v["lake_tables"][0]["system_read"] = json!(false)),
        "E_LAKE_MODEL_SOURCE",
        "lake_models[stock].sources",
    );
    // a model that reads its own table, and two that read each other's
    has(
        after(|v| model(v, "monthly")["sources"][0]["entity"] = json!("monthly")),
        "E_LAKE_MODEL_CYCLE",
        "lake_models[monthly]",
    );
    has(
        after(|v| {
            v["messages"][0]["fields"] = json!([
                field("loaded_on", 1, "date"),
                field("sku", 2, "string"),
                field("on_hand", 3, "decimal")
            ]);
            v["lake_models"].as_array_mut().unwrap().push(json!(
                { "table": "stock_file", "sources": [{ "entity": "stock", "alias": "s" }],
                  "columns": [{ "name": "loaded_on", "value": col("s", "day") },
                              { "name": "sku", "value": col("s", "sku") },
                              { "name": "on_hand", "value": col("s", "on_hand") }] }));
        }),
        "E_LAKE_MODEL_CYCLE",
        "lake_models[stock]",
    );
    // a model larger than a view may be
    has(
        after(|v| {
            let items: Vec<Value> = (0..600)
                .map(|_| json!({ "compare": { "left": col("f", "sku"), "op": "ne", "values": ["x"] } }))
                .collect();
            model(v, "stock")["filter"] = json!({ "and": items });
        }),
        "E_LAKE_MODEL_SIZE",
        "lake_models[stock]",
    );
}

#[test]
fn another_plugin_s_table_is_checked_by_its_fields_when_they_are_known() {
    let mut v = retail();
    v["lake_models"].as_array_mut().unwrap().push(json!(
        { "table": "stock_file", "sources": [{ "entity": "erp.stock", "alias": "e" }],
          "columns": [{ "name": "loaded_on", "value": col("e", "day") },
                      { "name": "sku", "value": col("e", "sku") },
                      { "name": "on_hand", "value": col("e", "qty") }] }));
    // by its declaration only: valid
    assert_eq!(codes(&v), vec![]);
    let d = parse(v.to_string().as_bytes()).unwrap();
    let fields = |qty: &str| {
        let f: Vec<Field> = serde_json::from_value(json!([
            field("day", 1, "date"),
            field("sku", 2, "string"),
            field("qty", 3, qty)
        ]))
        .unwrap();
        f
    };
    let ok = lake_model::validate_with(&d, &|p, t| {
        (p == "erp" && t == "stock").then(|| fields("string"))
    });
    assert!(ok.is_empty(), "{ok:?}");
    let wrong = lake_model::validate_with(&d, &|p, t| {
        (p == "erp" && t == "stock").then(|| fields("int64"))
    });
    assert_eq!(wrong.len(), 1, "{wrong:?}");
    assert_eq!(wrong[0].code, "E_LAKE_MODEL_TYPE");
}

/// A name of the plugin's own entity or view written qualified (`retail.product`) is the same source to every check.
#[test]
fn the_plugin_s_own_qualified_names_are_its_own_sources() {
    let has = |got: Vec<(&'static str, String)>, code: &str, path: &str| {
        assert!(
            got.iter().any(|(c, p)| *c == code && p.starts_with(path)),
            "{code} at {path}: {got:?}"
        );
    };
    // a view naming its entity qualified, a masked column in it: the model over it needs unmasked
    let qualified_view = |v: &mut Value| {
        v["views"][0]["sources"][0]["entity"] = json!("retail.product");
        v["views"][0]["filter"] =
            json!({ "compare": { "left": col("p", "cost"), "op": "is_not_null" } });
    };
    has(
        after(qualified_view),
        "E_LAKE_MODEL_MASKED",
        "lake_models[category_count]",
    );
    // the model names the entity qualified: a masked column, the bypass, the audit's entity
    let qualified_model = |v: &mut Value| {
        model(v, "product_dim")["sources"][0]["entity"] = json!("retail.product");
        model(v, "product_dim")["filter"] =
            json!({ "compare": { "left": col("p", "cost"), "op": "gt", "values": ["1"] } });
    };
    has(
        after(qualified_model),
        "E_LAKE_MODEL_MASKED",
        "lake_models[product_dim]",
    );
    has(
        after(|v| {
            qualified_model(v);
            v["capabilities"] = json!([]);
        }),
        "E_LAKE_MODEL_BYPASS",
        "lake_models[product_dim]",
    );
    let mut v = retail();
    qualified_view(&mut v);
    let d = parse(v.to_string().as_bytes()).unwrap();
    assert_eq!(
        lake_model::entities(&d, &d.lake_models[3]),
        vec!["product".to_string()]
    );
    // the hash follows the view named qualified too
    let mut w = v.clone();
    w["views"][0]["filter"] =
        json!({ "compare": { "left": col("p", "sku"), "op": "ne", "values": [""] } });
    let d2 = parse(w.to_string().as_bytes()).unwrap();
    assert_ne!(d.lake_models[3].hash(&d), d2.lake_models[3].hash(&d2));
    // a model over the plugin's own lake table named qualified is checked as the plain name
    has(
        after(|v| {
            model(v, "stock")["sources"][0]["entity"] = json!("retail.stock_file");
            model(v, "stock")["columns"][2]["value"] = col("f", "on_hand");
        }),
        "E_LAKE_MODEL_TYPE",
        "lake_models[stock].columns",
    );
}

/// An alias given twice in a tree (two `exists` side by side), and anything under a model that is another plugin's
/// entity or the directory, are refused.
#[test]
fn an_alias_twice_and_another_plugin_s_rows_under_a_model_are_refused() {
    let has = |got: Vec<(&'static str, String)>, code: &str, path: &str, text: &str| {
        assert!(
            got.iter().any(|(c, p)| *c == code && p.starts_with(path)),
            "{code} at {path}: {got:?}"
        );
        let _ = text;
    };
    let ex = |alias: &str| {
        json!({ "exists": { "entity": "product", "alias": alias,
                            "filter": { "compare": { "left": col(alias, "cost"), "op": "is_not_null" } } } })
    };
    has(
        after(|v| model(v, "product_dim")["filter"] = json!({ "and": [ex("x"), ex("x")] })),
        "E_LAKE_MODEL_SOURCE",
        "lake_models[product_dim].sources",
        "twice",
    );
    has(
        after(|v| v["views"][0]["filter"] = json!({ "and": [ex("x"), ex("x")] })),
        "E_LAKE_MODEL_SOURCE",
        "lake_models[category_count].sources",
        "twice",
    );
    // a view under the model that joins a dependency's entity, or the directory
    let crm = |v: &mut Value| {
        v["dependencies"] = json!([{ "plugin": "crm", "calls": [], "lake": ["stock"],
            "reads": [{ "entity": "customer", "fields": [{ "name": "id", "type": "ref:crm.customer" },
                                                          { "name": "segment", "type": "string" }] }] }]);
    };
    for (entity, alias, field) in [
        ("crm.customer", "c", "segment"),
        ("platform.user", "u", "display_name"),
    ] {
        has(
            after(|v| {
                crm(v);
                v["views"][0]["sources"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({ "entity": entity, "alias": alias, "join": "left",
                                  "left": { "source": "p", "field": "sku" }, "right": { "source": alias, "field": field } }));
            }),
            "E_LAKE_MODEL_SOURCE",
            "lake_models[category_count].sources",
            entity,
        );
    }
}
