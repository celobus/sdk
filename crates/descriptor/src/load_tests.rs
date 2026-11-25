use serde_json::{Value, json};

use super::*;

fn field(name: &str, n: u32, ty: &str) -> Value {
    json!({ "name": name, "number": n, "type": ty, "repeated": false, "optional": false, "description": "" })
}

fn col(source: &str, field: &str) -> Value {
    json!({ "column": { "source": source, "field": field } })
}

/// `retail`: the lake table `metrics` (a mart, read as the system context) and `erp.stock` (another plugin's), the
/// entity `store`, and `store_metric`, stored in OLAP, keyed by (day, store, metric), read by analysts and by whoever
/// reads its store, its `value` masked for buyers; loaded from `metrics`.
fn retail() -> Value {
    json!({
      "schema": 1, "id": "retail", "version": "1.0.0",
      "roles": [{ "name": "analyst", "title": "Analyst" }, { "name": "buyer", "title": "Buyer" }],
      "dependencies": [{ "plugin": "erp", "calls": [], "lake": ["stock"] }],
      "messages": [
        { "name": "MetricRow", "kind": "message", "description": "",
          "fields": [field("day", 1, "date"), field("store_id", 2, "ref:retail.store"), field("metric", 3, "string"),
                     field("value", 4, "string")] }
      ],
      "entities": [
        { "name": "store", "message": "Store", "key": "uuid", "label": "name",
          "fields": [field("id", 1, "ref:retail.store"), field("name", 2, "string")],
          "access": { "read": "authenticated" } },
        { "name": "store_metric", "message": "StoreMetric", "key": "uuid", "storage": "olap",
          "olap_key": ["day", "store", "metric"],
          "fields": [field("id", 1, "ref:retail.store_metric"), field("day", 2, "date"),
                     field("store", 3, "ref:retail.store"), field("metric", 4, "string"),
                     { "name": "value", "number": 5, "type": "decimal", "repeated": false, "optional": true,
                       "description": "" },
                     { "name": "note", "number": 6, "type": "string", "repeated": false, "optional": true,
                       "description": "" }],
          "access": {
            "relations": [{ "name": "store", "kind": "column", "field": "store" }],
            "read": { "any": [{ "via": { "rel": "store", "perm": "read" } }, { "role": "analyst" }] },
            "masks": [{ "field": "value", "roles": ["analyst"] }] } }
      ],
      "layers": [{ "name": "mart", "title": "" }],
      "lake_tables": [
        { "name": "metrics", "layer": "mart", "message": "MetricRow", "partition": ["day"], "system_read": true }
      ],
      "loads": [
        { "entity": "store_metric",
          "sources": [{ "entity": "metrics", "alias": "m" }],
          "filter": { "compare": { "left": col("m", "day"), "op": "ge", "right": { "days_ago": 400 } } },
          "columns": [
            { "name": "day", "value": col("m", "day") },
            { "name": "store", "value": col("m", "store_id") },
            { "name": "metric", "value": col("m", "metric") },
            { "name": "value", "value": { "column": { "source": "m", "field": "value", "parse": "decimal" } } }
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

fn metric(v: &mut Value) -> &mut Value {
    &mut v["entities"][1]
}

fn has(errors: &[(&'static str, String)], code: &str) -> bool {
    errors.iter().any(|(c, _)| *c == code)
}

#[test]
fn an_entity_stored_in_olap_and_its_load_check_and_round_trip() {
    assert_eq!(codes(&retail()), vec![]);
    let d = parse(retail().to_string().as_bytes()).unwrap();
    let back = serde_json::to_value(&d).unwrap();
    assert_eq!(back["loads"], retail()["loads"]);
    assert_eq!(back["entities"][1]["storage"], "olap");
    assert_eq!(
        back["entities"][1]["olap_key"],
        json!(["day", "store", "metric"])
    );
    // an ordinary entity writes neither key
    assert!(back["entities"][0].get("storage").is_none());
    assert!(back["entities"][0].get("olap_key").is_none());
    assert!(d.entities[1].olap() && !d.entities[0].olap());
    let cols = load::columns(&d, &d.entities[1]).unwrap();
    assert_eq!(
        cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["day", "store", "metric", "value", "note"]
    );
    // the hash follows the load and the entity's fields
    let h = d.loads[0].hash(&d);
    assert_eq!(h.len(), 64);
    let mut v = retail();
    metric(&mut v)["fields"][5]["type"] = json!("int64");
    let d2 = parse(v.to_string().as_bytes()).unwrap();
    assert_ne!(d2.loads[0].hash(&d2), h);
}

#[test]
fn a_descriptor_without_such_entities_is_unchanged_by_the_feature() {
    let mut v = retail();
    v["entities"].as_array_mut().unwrap().pop();
    v.as_object_mut().unwrap().remove("loads");
    let d = parse(v.to_string().as_bytes()).unwrap();
    let back = serde_json::to_value(&d).unwrap();
    assert!(back.get("loads").is_none());
    assert!(back["entities"][0].get("storage").is_none());
}

#[test]
fn the_entity_s_shape_is_checked() {
    assert!(has(
        &after(|v| metric(v)["storage"] = json!("disk")),
        "E_OLAP_STORAGE"
    ));
    for (what, on) in [
        ("soft_delete", true),
        ("version", true),
        ("append_only", true),
    ] {
        assert!(
            has(&after(|v| metric(v)[what] = json!(on)), "E_OLAP_ENTITY"),
            "{what}"
        );
    }
    assert!(has(
        &after(|v| metric(v)["key"] = json!("i64")),
        "E_OLAP_KEY"
    ));
    assert!(has(
        &after(|v| metric(v)["olap_key"] = json!([])),
        "E_OLAP_KEY"
    ));
    assert!(has(
        &after(|v| metric(v)["olap_key"] = json!(["day", "nope"])),
        "E_OLAP_KEY"
    ));
    assert!(has(
        &after(|v| metric(v)["olap_key"] = json!(["day", "day"])),
        "E_OLAP_KEY"
    ));
    // an optional field keys nothing
    assert!(has(
        &after(|v| metric(v)["olap_key"] = json!(["day", "note"])),
        "E_OLAP_KEY"
    ));
    // an ordinary entity declares no key of fields
    assert!(has(
        &after(|v| v["entities"][0]["olap_key"] = json!(["name"])),
        "E_OLAP_KEY"
    ));
    assert!(has(
        &after(|v| metric(v)["fields"][5]["encrypted"] = json!(true)),
        "E_OLAP_ENCRYPTED"
    ));
    assert!(has(
        &after(|v| metric(v)["fields"][5]["repeated"] = json!(true)),
        "E_OLAP_FIELD"
    ));
    assert!(has(
        &after(|v| metric(v)["fields"][4]["history"] = json!(true)),
        "E_OLAP_HISTORY"
    ));
    // no load, or two
    assert!(has(
        &after(|v| {
            v.as_object_mut().unwrap().remove("loads");
        }),
        "E_OLAP_LOAD"
    ));
    assert!(has(
        &after(|v| {
            let l = v["loads"][0].clone();
            v["loads"].as_array_mut().unwrap().push(l);
        }),
        "E_OLAP_LOAD"
    ));
}

#[test]
fn its_access_needs_no_grants_kept_per_row_and_grants_read_only() {
    assert!(has(
        &after(|v| metric(v)["access"]["create"] = json!("authenticated")),
        "E_OLAP_ACCESS"
    ));
    assert!(has(
        &after(|v| metric(v)["access"]["delete"] = json!({ "role": "analyst" })),
        "E_OLAP_ACCESS"
    ));
    assert!(has(
        &after(|v| {
            metric(v)["access"]["relations"] = json!([{ "name": "store", "kind": "column", "field": "store" },
                       { "name": "watchers", "kind": "users", "field": "metric" }]);
        }),
        "E_OLAP_ACCESS"
    ));
    assert!(has(
        &after(|v| metric(v)["access"]["read"] = json!({ "rel": "store" })),
        "E_OLAP_ACCESS"
    ));
    assert!(has(
        &after(|v| {
            metric(v)["access"]["permits"] =
                json!([{ "action": "flag", "grant": { "role": "analyst" } }])
        }),
        "E_OLAP_ACCESS"
    ));
    // a role, a condition on its own columns and via are fine
    assert_eq!(
        after(|v| {
            metric(v)["access"]["read"] = json!({ "any": [
                { "when": { "grant": "authenticated", "filter": [{ "field": "metric", "op": "eq", "value": "footfall" }] } },
                { "via": { "rel": "store", "perm": "read" } }] })
        }),
        vec![]
    );
}

#[test]
fn an_ordinary_entity_never_refers_to_one_stored_in_olap() {
    let e = after(|v| {
        v["entities"][0]["fields"]
            .as_array_mut()
            .unwrap()
            .push(field("best", 3, "ref:retail.store_metric"));
    });
    assert!(has(&e, "E_OLAP_REF"), "{e:?}");
}

#[test]
fn a_load_reads_lake_relations_only_and_fills_every_required_field() {
    // an entity or a view is no source
    let e = after(|v| v["loads"][0]["sources"][0]["entity"] = json!("store"));
    assert!(has(&e, "E_LOAD_SOURCE"), "{e:?}");
    // another plugin's lake table it does not declare
    let e = after(|v| v["loads"][0]["sources"][0]["entity"] = json!("erp.other"));
    assert!(has(&e, "E_LOAD_SOURCE"), "{e:?}");
    // its own table read as the system context must say so
    let e = after(|v| v["lake_tables"][0]["system_read"] = json!(false));
    assert!(has(&e, "E_LOAD_SOURCE"), "{e:?}");
    // a required field missing, a field of another type, the id given
    let e = after(|v| {
        v["loads"][0]["columns"].as_array_mut().unwrap().remove(2);
    });
    assert!(has(&e, "E_LOAD_COLUMN"), "{e:?}");
    let e = after(|v| v["loads"][0]["columns"][3]["value"] = col("m", "value"));
    assert!(has(&e, "E_LOAD_TYPE"), "{e:?}");
    let e = after(|v| {
        v["loads"][0]["columns"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "id", "value": col("m", "store_id") }))
    });
    assert!(has(&e, "E_LOAD_COLUMN"), "{e:?}");
    // an optional field may be given
    assert_eq!(
        after(|v| {
            v["loads"][0]["columns"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "name": "note", "value": col("m", "metric") }))
        }),
        vec![]
    );
    // a load of an ordinary entity, of no entity
    let e = after(|v| v["loads"][0]["entity"] = json!("store"));
    assert!(has(&e, "E_LOAD_ENTITY"), "{e:?}");
    let e = after(|v| v["loads"][0]["entity"] = json!("nope"));
    assert!(has(&e, "E_LOAD_ENTITY"), "{e:?}");
    // a dependency's lake table: checked by its declaration only here
    assert_eq!(
        after(|v| v["loads"][0]["sources"][0]["entity"] = json!("erp.stock")),
        vec![]
    );
    // and by its fields when they are known
    let mut v = retail();
    v["loads"][0]["sources"][0]["entity"] = json!("erp.stock");
    let d = parse(v.to_string().as_bytes()).unwrap();
    let foreign = |_: &str, _: &str| Some(vec![]);
    let e: Vec<_> = load::validate_with(&d, &foreign)
        .into_iter()
        .map(|e| e.code)
        .collect();
    assert!(e.contains(&"E_LOAD_COLUMN"), "{e:?}");
}
