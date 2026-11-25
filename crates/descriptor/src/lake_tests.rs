use serde_json::{Value, json};

use super::*;

fn field(name: &str, n: u32, ty: &str, optional: bool) -> Value {
    json!({ "name": name, "number": n, "type": ty, "repeated": false, "optional": optional, "description": "" })
}

/// `retail`: the layers raw and core, a raw table written by a job and a cleaned one with a key.
fn retail() -> Value {
    json!({
      "schema": 1, "id": "retail", "version": "1.0.0",
      "roles": [{ "name": "analyst", "title": "Analyst" }],
      "enums": [{ "name": "Unit", "values": ["piece", "kg"] }],
      "messages": [
        { "name": "StockFileRow", "kind": "message", "description": "",
          "fields": [field("loaded_on", 1, "date", false), field("sku", 2, "string", false),
                     field("on_hand", 3, "string", false)] },
        { "name": "Stock", "kind": "message", "description": "",
          "fields": [field("day", 1, "date", false), field("sku", 2, "string", false),
                     field("warehouse", 3, "string", false), field("on_hand", 4, "decimal", false),
                     field("unit", 5, "enum:Unit", false), field("seen_at", 6, "timestamp", true)] }
      ],
      "layers": [{ "name": "raw", "title": "As it arrived" }, { "name": "core", "title": "Cleaned" }],
      "lake_tables": [
        { "name": "stock_file", "layer": "raw", "message": "StockFileRow", "partition": ["loaded_on"] },
        { "name": "stock", "layer": "core", "message": "Stock", "partition": ["day"],
          "key": ["day", "sku", "warehouse"], "read": ["analyst"], "system_read": true, "export": true }
      ]
    })
}

fn errors(v: &Value) -> Vec<(&'static str, String)> {
    let d = parse(v.to_string().as_bytes()).unwrap();
    validate(&d).into_iter().map(|e| (e.code, e.path)).collect()
}

#[test]
fn a_plugin_with_layers_and_lake_tables_is_valid_and_round_trips() {
    let v = retail();
    assert_eq!(errors(&v), vec![]);
    let checked = check(v.to_string().as_bytes()).unwrap();
    assert_eq!(checked.descriptor.lake_tables.len(), 2);
    assert_eq!(checked.descriptor.layers[1].name, "core");
    let cols = lake::columns(&checked.descriptor, &checked.descriptor.lake_tables[1]).unwrap();
    assert_eq!(cols.len(), 6);
    assert_eq!(
        cols[4].ty,
        lake::ColumnType::Enum(vec!["piece".into(), "kg".into()])
    );
    assert!(cols[5].optional);
    // a plugin without any: nothing is written
    let plain = json!({ "schema": 1, "id": "plain", "version": "1.0.0" });
    let d = parse(plain.to_string().as_bytes()).unwrap();
    let out = serde_json::to_string(&d).unwrap();
    assert!(!out.contains("layers") && !out.contains("lake"), "{out}");
}

#[test]
fn every_rule_of_a_lake_declaration_has_its_code() {
    let mut cases: Vec<(Value, &str, &str)> = vec![];
    let mut v = retail();
    v["layers"][1]["name"] = json!("Core");
    cases.push((v, "E_LAKE_LAYER_NAME", "layers[Core].name"));
    let mut v = retail();
    v["layers"][1]["name"] = json!("raw");
    cases.push((v, "E_LAKE_LAYER_DUP", "layers[raw]"));
    let mut v = retail();
    v["lake_tables"][1]["name"] = json!("stock-x");
    cases.push((v, "E_LAKE_TABLE_NAME", "lake_tables[stock-x].name"));
    let mut v = retail();
    v["lake_tables"][1]["name"] = json!("stock_file");
    cases.push((v, "E_LAKE_TABLE_DUP", "lake_tables[stock_file]"));
    let mut v = retail();
    v["lake_tables"][0]["layer"] = json!("mart");
    cases.push((v, "E_LAKE_TABLE_LAYER", "lake_tables[stock_file].layer"));
    let mut v = retail();
    v["lake_tables"][0]["message"] = json!("Nope");
    cases.push((v, "E_LAKE_TABLE_MESSAGE", "lake_tables[stock_file].message"));
    let mut v = retail();
    v["messages"][0]["fields"][2]["repeated"] = json!(true);
    cases.push((v, "E_LAKE_FIELD_TYPE", "lake_tables[stock_file].message"));
    let mut v = retail();
    v["messages"][0]["fields"][2]["encrypted"] = json!(true);
    cases.push((v, "E_LAKE_ENCRYPTED", "lake_tables[stock_file].message"));
    let mut v = retail();
    v["lake_tables"][1]["partition"] = json!(["on_hand"]);
    cases.push((v, "E_LAKE_PARTITION", "lake_tables[stock].partition[0]"));
    let mut v = retail();
    v["lake_tables"][1]["partition"] = json!(["nope"]);
    cases.push((v, "E_LAKE_PARTITION", "lake_tables[stock].partition[0]"));
    let mut v = retail();
    v["lake_tables"][1]["key"] = json!(["day", "seen_at"]);
    cases.push((v, "E_LAKE_KEY", "lake_tables[stock].key[1]"));
    let mut v = retail();
    v["lake_tables"][1]["key"] = json!(["on_hand"]);
    cases.push((v, "E_LAKE_KEY", "lake_tables[stock].key[0]"));
    let mut v = retail();
    v["lake_tables"][1]["read"] = json!(["boss"]);
    cases.push((v, "E_LAKE_ROLE", "lake_tables[stock].read[0]"));
    let mut v = retail();
    v["entities"] = json!([{ "name": "stock", "message": "Stock", "key": "uuid", "fields": [
        field("id", 1, "ref:retail.stock", false)] }]);
    v["messages"][1]["name"] = json!("StockRow");
    v["lake_tables"][1]["message"] = json!("StockRow");
    cases.push((v, "E_LAKE_TABLE_CLASH", "lake_tables[stock]"));
    let mut v = retail();
    v["dependencies"] = json!([{ "plugin": "erp", "calls": [], "lake": ["Bad Name"] }]);
    cases.push((v, "E_LAKE_DEPENDENCY", "dependencies[0].lake[0]"));
    for (v, code, path) in cases {
        let e = errors(&v);
        assert!(
            e.iter().any(|(c, p)| *c == code && p == path),
            "{code} at {path}: {e:?}"
        );
    }
}

#[test]
fn a_lake_table_only_grows_between_versions() {
    let old = parse(retail().to_string().as_bytes()).unwrap();
    let prev = lock::lock_of(&old);
    assert_eq!(prev.lake_tables["stock"].columns["on_hand"], "decimal");
    // an optional column added, a table added, a table no longer declared: no change
    let mut v = retail();
    v["messages"][1]["fields"]
        .as_array_mut()
        .unwrap()
        .push(field("reserved", 7, "decimal", true));
    v["lake_tables"].as_array_mut().unwrap().remove(0);
    v["messages"].as_array_mut().unwrap().push(
        json!({ "name": "Extra", "kind": "message", "description": "",
        "fields": [field("n", 1, "int64", false)] }),
    );
    v["lake_tables"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "extra", "layer": "core", "message": "Extra" }));
    let up = lock::update(Some(&prev), &parse(v.to_string().as_bytes()).unwrap());
    assert_eq!(up.changes, vec![]);
    assert!(
        up.lock.lake_tables.contains_key("stock_file"),
        "a table no longer declared stays locked"
    );
    // every change that is not growth
    let mut v = retail();
    v["lake_tables"][1]["layer"] = json!("raw");
    v["lake_tables"][1]["partition"] = json!(["sku"]);
    v["lake_tables"][1]["key"] = json!(["day", "sku"]);
    v["messages"][1]["fields"][3]["type"] = json!("double");
    v["messages"][1]["fields"].as_array_mut().unwrap().remove(4);
    v["messages"][1]["fields"]
        .as_array_mut()
        .unwrap()
        .push(field("required", 8, "string", false));
    let new = parse(v.to_string().as_bytes()).unwrap();
    let codes: Vec<&str> = lock::update(Some(&prev), &new)
        .changes
        .iter()
        .filter(|c| c.breaking)
        .map(|c| c.code)
        .filter(|c| c.starts_with("E_LAKE_"))
        .collect();
    for want in [
        "E_LAKE_TABLE_LAYER_CHANGED",
        "E_LAKE_PARTITION_CHANGED",
        "E_LAKE_KEY_CHANGED",
        "E_LAKE_COLUMN_TYPE",
        "E_LAKE_COLUMN_REMOVED",
        "E_LAKE_COLUMN_REQUIRED",
    ] {
        assert!(codes.contains(&want), "{want}: {codes:?}");
    }
    // the publish check sees the same
    assert!(
        lock::compare(&old, &new)
            .iter()
            .any(|c| c.code == "E_LAKE_COLUMN_TYPE")
    );
    // an optional column that becomes required keeps its type
    let mut v = retail();
    v["messages"][1]["fields"][5]["optional"] = json!(false);
    let up = lock::update(Some(&prev), &parse(v.to_string().as_bytes()).unwrap());
    assert!(
        up.changes.iter().all(|c| !c.code.starts_with("E_LAKE_")),
        "{:?}",
        up.changes
    );
}
