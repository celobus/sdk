use serde_json::{Value, json};

use super::*;

fn field(name: &str, n: u32, ty: &str, optional: bool) -> Value {
    json!({ "name": name, "number": n, "type": ty, "repeated": false, "optional": optional, "description": "" })
}

fn kept(name: &str, n: u32, ty: &str, optional: bool) -> Value {
    let mut f = field(name, n, ty, optional);
    f["history"] = json!(true);
    f
}

/// `helpdesk` of scenario L-1: tickets whose status and assignee are kept in the layer `audit`, a model of open
/// tickets per day over the history per day.
fn helpdesk() -> Value {
    json!({
      "schema": 1, "id": "helpdesk", "version": "1.0.0",
      "roles": [{ "name": "analyst", "title": "Analyst" }, { "name": "agent", "title": "Agent" }],
      "enums": [{ "name": "Status", "values": ["open", "waiting", "closed"] }],
      "messages": [
        { "name": "Ticket", "kind": "message", "description": "",
          "fields": [field("id", 1, "ref:helpdesk.ticket", false), field("title", 2, "string", false),
                     field("status", 3, "enum:Status", false), field("assignee", 4, "ref:platform.user", true),
                     field("body", 5, "string", false)] },
        { "name": "OpenByDay", "kind": "message", "description": "",
          "fields": [field("day", 1, "date", false), field("assignee", 2, "ref:platform.user", true),
                     field("open", 3, "int64", false)] }
      ],
      "entities": [
        { "name": "ticket", "message": "Ticket", "key": "uuid", "label": "title", "soft_delete": true,
          "version": false, "append_only": false,
          "fields": [field("id", 1, "ref:helpdesk.ticket", false), field("title", 2, "string", false),
                     kept("status", 3, "enum:Status", false), kept("assignee", 4, "ref:platform.user", true),
                     field("body", 5, "string", false)],
          "history": { "layer": "audit", "read": ["analyst"], "system_read": true } }
      ],
      "layers": [{ "name": "audit", "title": "History of records" }, { "name": "mart", "title": "For reports" }],
      "lake_tables": [
        { "name": "open_by_day", "layer": "mart", "message": "OpenByDay", "partition": ["day"], "read": ["analyst"] }
      ],
      "lake_models": [
        { "table": "open_by_day",
          "sources": [{ "entity": "ticket", "alias": "h", "history": true, "each_day": true }],
          "filter": { "not": { "compare": { "left": { "column": { "source": "h", "field": "status" } },
                                            "op": "eq", "values": ["closed"] } } },
          "group_by": [{ "source": "h", "field": "day" }, { "source": "h", "field": "assignee" }],
          "columns": [
            { "name": "day", "value": { "column": { "source": "h", "field": "day" } } },
            { "name": "assignee", "value": { "column": { "source": "h", "field": "assignee" } } },
            { "name": "open", "value": { "aggregate": { "fn": "count" } } }
          ] }
      ]
    })
}

fn codes(v: &Value) -> Vec<(&'static str, String)> {
    let d = parse(v.to_string().as_bytes()).unwrap();
    validate(&d).into_iter().map(|e| (e.code, e.path)).collect()
}

fn after(f: impl FnOnce(&mut Value)) -> Vec<&'static str> {
    let mut v = helpdesk();
    f(&mut v);
    codes(&v).into_iter().map(|(c, _)| c).collect()
}

#[test]
fn a_history_with_a_model_over_it_per_day_checks_and_round_trips() {
    assert_eq!(codes(&helpdesk()), vec![]);
    let d = parse(helpdesk().to_string().as_bytes()).unwrap();
    let back = serde_json::to_value(&d).unwrap();
    assert_eq!(
        back["entities"][0]["history"],
        json!({ "layer": "audit", "read": ["analyst"], "system_read": true })
    );
    assert_eq!(back["entities"][0]["fields"][2]["history"], json!(true));
    // a field not kept says nothing of history
    assert!(back["entities"][0]["fields"][1].get("history").is_none());
    assert_eq!(
        back["lake_models"][0]["sources"][0],
        json!({ "entity": "ticket", "alias": "h", "history": true, "each_day": true })
    );
    let e = &d.entities[0];
    assert_eq!(
        history::columns(&d, e)
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["status", "assignee"]
    );
    // a model over the history reads the lake
    assert_eq!(
        lake_model::kind(&d, "ticket#history_days"),
        lake_model::Kind::Lake
    );
    assert_eq!(
        lake_model::kind(&d, "nope#history"),
        lake_model::Kind::Unknown
    );
}

#[test]
fn what_a_history_cannot_keep_is_refused_by_its_code() {
    // a name whose history's tables could take another lake table's names
    let errs = after(|v| {
        v["entities"][0]["name"] = json!("ti__cket");
        v["messages"][0]["fields"][0]["type"] = json!("ref:helpdesk.ti__cket");
        v["entities"][0]["fields"][0]["type"] = json!("ref:helpdesk.ti__cket");
        v["lake_models"][0]["sources"][0]["entity"] = json!("ti__cket");
    });
    assert!(errs.contains(&"E_HISTORY_NAME"), "{errs:?}");
    // the layer is not declared
    assert_eq!(
        after(|v| v["entities"][0]["history"]["layer"] = json!("raw")),
        ["E_HISTORY_LAYER"]
    );
    // a column kept on an entity without a history
    assert_eq!(
        after(|v| {
            v["entities"][0].as_object_mut().unwrap().remove("history");
            v["lake_models"] = json!([]);
            v["lake_tables"] = json!([]);
        }),
        ["E_HISTORY_LAYER"]
    );
    // an encrypted column
    assert_eq!(
        after(|v| v["entities"][0]["fields"][4] = {
            let mut f = kept("body", 5, "string", false);
            f["encrypted"] = json!(true);
            f
        }),
        ["E_HISTORY_ENCRYPTED"]
    );
    // a masked column without `unmasked` (the model naming it no longer finds it), then with it
    let errs = after(|v| {
        v["entities"][0]["access"] = json!({ "read": "authenticated", "masks": [{ "field": "assignee", "roles": ["agent"] }] })
    });
    assert!(errs.contains(&"E_HISTORY_MASKED"), "{errs:?}");
    assert_eq!(
        after(|v| {
            v["entities"][0]["access"] = json!({ "read": "authenticated", "masks": [{ "field": "assignee", "roles": ["agent"] }] });
            v["entities"][0]["history"]["unmasked"] = json!(true);
        }),
        Vec::<&str>::new()
    );
    // a masked column marked without `unmasked` is never among the kept ones, whatever the check says
    let mut v = helpdesk();
    v["entities"][0]["access"] =
        json!({ "read": "authenticated", "masks": [{ "field": "assignee", "roles": ["agent"] }] });
    let d = parse(v.to_string().as_bytes()).unwrap();
    assert_eq!(
        history::columns(&d, &d.entities[0])
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["status"]
    );
    // a list, a column named as a history column, a role that is not the plugin's
    assert_eq!(
        after(|v| {
            v["messages"][0]["fields"][4]["repeated"] = json!(true);
            v["entities"][0]["fields"][4] = {
                let mut f = kept("body", 5, "string", false);
                f["repeated"] = json!(true);
                f
            };
        }),
        ["E_HISTORY_TYPE"]
    );
    assert_eq!(
        after(|v| {
            v["messages"][0]["fields"][4] = field("change", 5, "string", false);
            v["entities"][0]["fields"][4] = kept("change", 5, "string", false);
        }),
        ["E_HISTORY_COLUMN"]
    );
    assert_eq!(
        after(|v| v["entities"][0]["history"]["read"] = json!(["boss"])),
        ["E_HISTORY_ROLE"]
    );
    // `all` with a column that does not carry its flag
    assert_eq!(
        after(|v| v["entities"][0]["history"]["all"] = json!(true)),
        ["E_HISTORY_ALL", "E_HISTORY_ALL"]
    );
    // a view reads no history
    let errs = after(|v| {
        v["views"] = json!([{ "name": "trail", "message": "OpenByDay", "description": "",
            "sources": [{ "entity": "ticket", "alias": "h", "history": true }],
            "columns": [] }]);
    });
    assert!(errs.contains(&"E_VIEW_SOURCE"), "{errs:?}");
    // a model over its own history read without system_read, or over another plugin's history
    let errs = after(|v| v["entities"][0]["history"]["system_read"] = json!(false));
    assert!(errs.contains(&"E_LAKE_MODEL_SOURCE"), "{errs:?}");
    let errs = after(|v| {
        v["dependencies"] = json!([{ "plugin": "crm", "calls": [], "lake": ["ticket"] }]);
        v["lake_models"][0]["sources"][0]["entity"] = json!("crm.ticket");
    });
    assert!(errs.contains(&"E_LAKE_MODEL_SOURCE"), "{errs:?}");
    // a model naming a column the history does not keep
    let errs = after(|v| {
        v["lake_models"][0]["group_by"][1] = json!({ "source": "h", "field": "title" });
    });
    assert!(errs.contains(&"E_LAKE_MODEL_COLUMN"), "{errs:?}");
}

#[test]
fn all_covers_every_column_it_can_and_says_what_it_left_out() {
    let mut v = helpdesk();
    v["lake_models"] = json!([]);
    v["entities"][0]["history"]["all"] = json!(true);
    v["entities"][0]["fields"][1] = kept("title", 2, "string", false);
    v["entities"][0]["fields"][3] = field("assignee", 4, "ref:platform.user", true);
    v["entities"][0]["access"] =
        json!({ "read": "authenticated", "masks": [{ "field": "assignee", "roles": ["agent"] }] });
    v["entities"][0]["fields"][4]["encrypted"] = json!(true);
    let d = parse(v.to_string().as_bytes()).unwrap();
    let e = &d.entities[0];
    assert_eq!(
        history::left_out(&d, e)
            .into_iter()
            .map(|(f, why)| (f, why.split(':').next().unwrap().to_string()))
            .collect::<Vec<_>>(),
        [
            ("assignee".to_string(), "it is masked".to_string()),
            ("body".to_string(), "it is encrypted".to_string())
        ]
    );
    assert_eq!(history::covered(&d, e), ["title", "status"]);
    assert_eq!(codes(&v), vec![]);
    // with unmasked, the masked one is covered, and must carry its flag
    v["entities"][0]["history"]["unmasked"] = json!(true);
    let d = parse(v.to_string().as_bytes()).unwrap();
    assert_eq!(
        history::covered(&d, &d.entities[0]),
        ["title", "status", "assignee"]
    );
    assert_eq!(
        codes(&v),
        vec![(
            "E_HISTORY_ALL",
            "entities[ticket].fields[assignee].history".to_string()
        )]
    );
}

#[test]
fn a_history_only_grows_between_versions() {
    let old = parse(helpdesk().to_string().as_bytes()).unwrap();
    let prev = lock::lock_of(&old);
    assert_eq!(prev.histories["ticket"].columns["status"], "enum:Status");
    // a column added to the kept ones, one no longer kept: no change, both stay locked
    let mut v = helpdesk();
    v["entities"][0]["fields"][1] = kept("title", 2, "string", false);
    v["entities"][0]["fields"][3] = field("assignee", 4, "ref:platform.user", true);
    let up = lock::update(Some(&prev), &parse(v.to_string().as_bytes()).unwrap());
    assert_eq!(up.changes, vec![]);
    assert_eq!(
        up.lock.histories["ticket"]
            .columns
            .keys()
            .collect::<Vec<_>>(),
        ["assignee", "status", "title"]
    );
    // another layer, a kept column of another type
    let mut v = helpdesk();
    v["entities"][0]["history"]["layer"] = json!("mart");
    v["messages"][0]["fields"][2]["type"] = json!("string");
    v["entities"][0]["fields"][2]["type"] = json!("string");
    let codes: Vec<&str> = lock::update(Some(&prev), &parse(v.to_string().as_bytes()).unwrap())
        .changes
        .iter()
        .filter(|c| c.breaking && c.code.starts_with("E_HISTORY_"))
        .map(|c| c.code)
        .collect();
    assert_eq!(codes, ["E_HISTORY_LAYER_CHANGED", "E_HISTORY_COLUMN_TYPE"]);
}

#[test]
fn check_notes_a_history_read_without_the_entity_s_rules() {
    // members read tickets: no note
    assert!(history::notes(&parse(helpdesk().to_string().as_bytes()).unwrap()).is_empty());
    // a ticket read by its assignee only: its history is read by analysts without that rule
    let mut v = helpdesk();
    v["entities"][0]["access"] = json!({
        "relations": [{ "name": "assignee", "kind": "column", "field": "assignee" }],
        "read": { "rel": "assignee" }
    });
    let d = parse(v.to_string().as_bytes()).unwrap();
    let notes = history::notes(&d);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0]
            .1
            .contains("read by analyst without the entity's rules"),
        "{notes:?}"
    );
}
