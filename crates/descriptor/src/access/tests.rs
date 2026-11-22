use serde_json::{Value, json};

use crate::{Error, check};

/// `notes`: a notebook its owner and its group read, a note that inherits the notebook's read, a manager role.
fn notes() -> Value {
    json!({
      "schema": 1, "id": "notes", "version": "1.0.0",
      "roles": [{ "name": "manager", "strict": false }, { "name": "auditor", "strict": true }],
      "enums": [{ "name": "State", "values": ["draft", "published", "archived"] }],
      "capabilities": [{ "name": "authz:bypass", "reason": "Cleans up.", "on": [{ "entity": "note", "ops": ["read", "delete"] }] }],
      "entities": [
        { "name": "notebook", "message": "Notebook",
          "fields": [
            { "name": "id", "number": 1, "type": "ref:notes.notebook" },
            { "name": "owner_id", "number": 2, "type": "ref:platform.user" },
            { "name": "group_id", "number": 3, "type": "ref:platform.group", "optional": true },
            { "name": "readers", "number": 4, "type": "ref:platform.user", "repeated": true }
          ],
          "access": {
            "relations": [{ "name": "owner", "kind": "column", "field": "owner_id" },
                          { "name": "team", "kind": "group", "field": "group_id" }],
            "read": { "any": [{ "rel": "owner" }, { "rel": "team" }, { "role": "manager" }] },
            "create": { "rel": "owner" },
            "update": { "rel": "owner" },
            "delete": null,
            "permits": [], "guards": [], "masks": [], "export": ["read"], "admin": true
          } },
        { "name": "note", "message": "Note",
          "fields": [
            { "name": "id", "number": 1, "type": "ref:notes.note" },
            { "name": "notebook_id", "number": 2, "type": "ref:notes.notebook" },
            { "name": "author_id", "number": 3, "type": "ref:platform.user" },
            { "name": "state", "number": 4, "type": "enum:State" },
            { "name": "secret", "number": 5, "type": "string", "optional": true }
          ],
          "access": {
            "relations": [{ "name": "author", "kind": "column", "field": "author_id" },
                          { "name": "book", "kind": "column", "field": "notebook_id" }],
            "read": { "any": [{ "rel": "author" }, { "via": { "rel": "book", "perm": "read" } }] },
            "create": { "rel": "author" },
            "update": { "any": [{ "when": { "grant": { "rel": "author" },
                                              "filter": [{ "field": "state", "op": "eq", "value": "draft" }] } },
                                { "role": "manager" }] },
            "delete": { "all": [{ "rel": "author" }, { "not": { "role": "auditor" } }] },
            "permits": [{ "action": "publish", "grant": { "role": "manager" } }],
            "guards": [{ "fields": ["state"],
                         "rules": [{ "value": "published", "grant": { "holds": "publish" } },
                                   { "value": null, "grant": { "role": "workspace.admin" } }] }],
            "masks": [{ "field": "secret", "roles": ["manager"] }],
            "export": [], "admin": true
          } }
      ]
    })
}

fn errors(v: &Value) -> Vec<Error> {
    match check(v.to_string().as_bytes()) {
        Ok(_) => vec![],
        Err(e) => e,
    }
}

fn codes_after(f: impl FnOnce(&mut Value)) -> Vec<(&'static str, String)> {
    let mut v = notes();
    f(&mut v);
    errors(&v).into_iter().map(|e| (e.code, e.path)).collect()
}

fn note(v: &mut Value) -> &mut Value {
    &mut v["entities"][1]["access"]
}

#[test]
fn a_full_declaration_is_valid_and_round_trips() {
    let v = notes();
    assert_eq!(errors(&v), vec![]);
    let d = crate::parse(v.to_string().as_bytes()).unwrap();
    let a = d.entities[1].access.as_ref().unwrap();
    assert_eq!(a.relations.len(), 2);
    assert!(matches!(a.grant("publish"), Some(crate::access::Grant::Role(r)) if r == "manager"));
    assert_eq!(d.capabilities[0].on[0].ops, ["read", "delete"]);
    // every member is written back, the absent grants as null
    let back = serde_json::to_value(&d).unwrap();
    assert_eq!(
        back["entities"][0]["access"],
        notes()["entities"][0]["access"]
    );
    assert_eq!(
        back["entities"][1]["access"],
        notes()["entities"][1]["access"]
    );
}

#[test]
fn relations_name_fields_of_the_right_type() {
    let c = codes_after(|v| {
        note(v)["relations"][0]["field"] = json!("state");
    });
    assert!(
        c.iter().any(|(code, _)| *code == "E_ACCESS_REL_COLUMN"),
        "{c:?}"
    );
    let c = codes_after(|v| {
        v["entities"][0]["access"]["relations"][1]["field"] = json!("owner_id");
    });
    assert!(c.iter().any(|(code, _)| *code == "E_ACCESS_GROUP"), "{c:?}");
    // a list of people in a column is no relation: a junction entity is
    let c = codes_after(|v| {
        v["entities"][0]["access"]["relations"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "shared", "kind": "users", "field": "readers" }));
    });
    assert!(c.iter().any(|(code, _)| *code == "E_ACCESS_USERS"), "{c:?}");
    // a relation to a row is not a grant by itself
    let c = codes_after(|v| {
        note(v)["read"] = json!({ "rel": "book" });
    });
    assert!(
        c.iter().any(|(code, _)| *code == "E_ACCESS_REL_COLUMN"),
        "{c:?}"
    );
    let c = codes_after(|v| {
        note(v)["read"] = json!({ "rel": "nobody" });
    });
    assert_eq!(c[0].0, "E_ACCESS_REL_UNKNOWN");
}

#[test]
fn roles_actions_and_via_are_known() {
    let c = codes_after(|v| {
        note(v)["update"] = json!({ "role": "boss" });
    });
    assert_eq!(
        c[0],
        (
            "E_ACCESS_ROLE_UNKNOWN",
            "entities[note].access.update".into()
        )
    );
    let c = codes_after(|v| {
        note(v)["update"] = json!({ "role": "workspace.member" });
    });
    assert!(c.is_empty(), "{c:?}");
    let c = codes_after(|v| {
        note(v)["update"] = json!({ "holds": "archive" });
    });
    assert_eq!(c[0].0, "E_ACCESS_ACTION_UNKNOWN");
    let c = codes_after(|v| {
        note(v)["permits"][0]["grant"] = json!({ "holds": "publish" });
    });
    assert_eq!(c[0].0, "E_ACCESS_CYCLE");
    let c = codes_after(|v| {
        note(v)["read"] = json!({ "via": { "rel": "author", "perm": "read" } });
    });
    assert_eq!(c[0].0, "E_ACCESS_VIA_TARGET");
    let c = codes_after(|v| {
        note(v)["read"] = json!({ "via": { "rel": "book", "perm": "delete" } });
    });
    assert_eq!(
        c[0].0, "E_ACCESS_VIA_TARGET",
        "the notebook grants no delete"
    );
    let c = codes_after(|v| {
        v["entities"][0]["access"]["read"] = json!({ "when": { "grant": { "rel": "owner" }, "filter": [{ "field": "owner_id", "op": "is_not_null" }] } });
    });
    assert!(c.is_empty(), "via inherits a grant with when: {c:?}");
}

#[test]
fn via_does_not_go_round_or_too_deep() {
    let c = codes_after(|v| {
        // the notebook reads through a note, which reads through the notebook
        v["entities"][0]["fields"].as_array_mut().unwrap().push(
            json!({ "name": "cover_id", "number": 5, "type": "ref:notes.note", "optional": true }),
        );
        let a = &mut v["entities"][0]["access"];
        a["relations"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "cover", "kind": "column", "field": "cover_id" }));
        a["read"] = json!({ "via": { "rel": "cover", "perm": "read" } });
    });
    assert!(c.iter().any(|(code, _)| *code == "E_ACCESS_CYCLE"), "{c:?}");
}

#[test]
fn guards_masks_and_exports_are_checked() {
    let c = codes_after(|v| {
        note(v)["guards"][0]["rules"][0]["value"] = json!("deleted");
    });
    assert_eq!(c[0].0, "E_ACCESS_GUARD_VALUE");
    let c = codes_after(|v| {
        note(v)["guards"][0]["fields"] = json!(["id"]);
    });
    assert!(
        c.iter().any(|(code, _)| *code == "E_ACCESS_GUARD_FIELD"),
        "{c:?}"
    );
    let c = codes_after(|v| {
        note(v)["masks"][0]["field"] = json!("id");
    });
    assert_eq!(c[0].0, "E_ACCESS_MASK_KEY");
    let c = codes_after(|v| {
        note(v)["masks"][0]["field"] = json!("state");
    });
    assert_eq!(c[0].0, "E_ACCESS_MASK_REQUIRED");
    let c = codes_after(|v| {
        note(v)["masks"][0]["roles"] = json!(["boss"]);
    });
    assert_eq!(c[0].0, "E_ACCESS_ROLE_UNKNOWN");
    let c = codes_after(|v| {
        note(v)["export"] = json!(["read"]);
    });
    assert_eq!(c[0].0, "E_ACCESS_EXPORT_NOT_ROW_LOCAL");
    let c = codes_after(|v| {
        note(v)["export"] = json!(["archive"]);
    });
    assert_eq!(c[0].0, "E_ACCESS_EXPORT");
    let c = codes_after(|v| {
        note(v)["update"]["any"][0]["when"]["filter"][0]["value"] = json!("lost");
    });
    assert_eq!(c[0].0, "E_ACCESS_WHEN_FIELD");
}

#[test]
fn append_only_entities_have_no_update_grants() {
    let c = codes_after(|v| {
        v["entities"][1]["append_only"] = json!(true);
    });
    let codes: Vec<&str> = c.iter().map(|x| x.0).collect();
    assert!(codes.contains(&"E_ENT_APPEND_ONLY_UPDATE"), "{c:?}");
}

#[test]
fn the_bypass_capability_is_scoped() {
    let c = codes_after(|v| {
        v["capabilities"][0]["on"] = json!([]);
    });
    assert_eq!(c[0].0, "E_CAPABILITY_SCOPE");
    let c = codes_after(|v| {
        v["capabilities"][0]["on"][0]["ops"] = json!(["drop"]);
    });
    assert_eq!(c[0].0, "E_CAPABILITY_SCOPE");
    let c = codes_after(|v| {
        v["capabilities"][0]["on"][0]["entity"] = json!("nope");
    });
    assert_eq!(c[0].0, "E_CAPABILITY_SCOPE");
}

#[test]
fn removing_an_export_is_breaking() {
    let old = crate::parse(notes().to_string().as_bytes()).unwrap();
    let mut v = notes();
    v["entities"][0]["access"]["export"] = json!([]);
    let new = crate::parse(v.to_string().as_bytes()).unwrap();
    let changes = crate::lock::compare(&old, &new);
    assert!(
        changes
            .iter()
            .any(|c| c.code == "E_CONTRACT_EXPORT_REMOVED" && c.breaking),
        "{changes:?}"
    );
    let mut v = notes();
    v["entities"][0]["access"]["read"] = json!({ "rel": "owner" });
    let new = crate::parse(v.to_string().as_bytes()).unwrap();
    let changes = crate::lock::compare(&old, &new);
    assert!(
        changes
            .iter()
            .any(|c| c.code == "W_CONTRACT_EXPORT_CHANGED" && !c.breaking),
        "{changes:?}"
    );
}

/// `notes` with a junction: `member(notebook_id, user_id, team_id, admin)`, the notebook read by its members (its
/// rows), changed by its admins, the member read through the notebook.
fn with_members() -> Value {
    let mut v = notes();
    v["entities"].as_array_mut().unwrap().push(json!(
        { "name": "member", "message": "Member", "soft_delete": true,
          "fields": [
            { "name": "id", "number": 1, "type": "ref:notes.member" },
            { "name": "notebook_id", "number": 2, "type": "ref:notes.notebook" },
            { "name": "user_id", "number": 3, "type": "ref:platform.user", "optional": true },
            { "name": "team_id", "number": 4, "type": "ref:platform.group", "optional": true },
            { "name": "admin", "number": 5, "type": "bool" }
          ],
          "access": {
            "relations": [{ "name": "notebook", "kind": "column", "field": "notebook_id" }],
            "read": { "via": { "rel": "notebook", "perm": "read" } },
            "create": { "via": { "rel": "notebook", "perm": "update" } },
            "delete": { "via": { "rel": "notebook", "perm": "update" } },
            "permits": [], "guards": [], "masks": [], "export": [], "admin": true
          } }
    ));
    let a = &mut v["entities"][0]["access"];
    let rels = a["relations"].as_array_mut().unwrap();
    rels.push(
        json!({ "name": "member", "kind": "rows", "field": "user_id",
                      "rows": { "entity": "member", "target": "notebook_id", "filter": [] } }),
    );
    rels.push(
        json!({ "name": "member_team", "kind": "rows", "field": "team_id",
                      "rows": { "entity": "member", "target": "notebook_id", "filter": [] } }),
    );
    rels.push(json!({ "name": "admin", "kind": "rows", "field": "user_id",
                      "rows": { "entity": "member", "target": "notebook_id",
                                "filter": [{ "field": "admin", "op": "eq", "value": true }] } }));
    a["read"] =
        json!({ "any": [{ "rel": "owner" }, { "rel": "member" }, { "rel": "member_team" }] });
    a["update"] = json!({ "any": [{ "rel": "owner" }, { "rel": "admin" }] });
    a["export"] = json!([]);
    v
}

fn codes_with_members(f: impl FnOnce(&mut Value)) -> Vec<(&'static str, String)> {
    let mut v = with_members();
    f(&mut v);
    errors(&v).into_iter().map(|e| (e.code, e.path)).collect()
}

#[test]
fn a_rows_relation_is_valid_round_trips_and_is_no_cycle() {
    let v = with_members();
    // the member reads through the notebook, which reads through the member's rows: a lookup, not a cycle
    assert_eq!(errors(&v), vec![]);
    let d = crate::parse(v.to_string().as_bytes()).unwrap();
    let a = d.entities[0].access.as_ref().unwrap();
    let r = a.relation("admin").unwrap();
    assert_eq!(r.kind, crate::access::RelationKind::Rows);
    let rows = r.rows.as_ref().unwrap();
    assert_eq!(
        (rows.entity.as_str(), rows.target.as_str()),
        ("member", "notebook_id")
    );
    assert_eq!(rows.filter[0].field, "admin");
    let back = serde_json::to_value(&d).unwrap();
    assert_eq!(back["entities"][0]["access"], v["entities"][0]["access"]);
    // a relation of another kind is written without `rows`
    assert!(
        back["entities"][0]["access"]["relations"][0]
            .get("rows")
            .is_none()
    );
}

#[test]
fn a_rows_relation_names_a_junction_of_the_plugin_pointing_here_and_a_subject() {
    fn rel(v: &mut Value) -> &mut Value {
        &mut v["entities"][0]["access"]["relations"][2]
    }
    let c = codes_with_members(|v| rel(v)["rows"]["entity"] = json!("greeter.member"));
    assert_eq!(
        c[0],
        (
            "E_ACCESS_ROWS_PLUGIN",
            "entities[notebook].access.relations[2].rows.entity".into()
        )
    );
    let c = codes_with_members(|v| {
        rel(v).as_object_mut().unwrap().remove("rows");
    });
    assert_eq!(c[0].0, "E_ACCESS_ROWS_PLUGIN");
    // the target column: a ref to this entity (the note is not the member's target)
    let c = codes_with_members(|v| rel(v)["rows"]["target"] = json!("user_id"));
    assert_eq!(
        c[0],
        (
            "E_ACCESS_ROWS_TARGET",
            "entities[notebook].access.relations[2].rows.target".into()
        )
    );
    let c = codes_with_members(|v| {
        let n = &mut v["entities"][1]["access"]["relations"];
        n.as_array_mut().unwrap().push(
            json!({ "name": "member", "kind": "rows", "field": "user_id",
            "rows": { "entity": "member", "target": "notebook_id", "filter": [] } }),
        );
    });
    assert!(
        c.iter()
            .any(|x| x.0 == "E_ACCESS_ROWS_TARGET" && x.1.starts_with("entities[note]")),
        "{c:?}"
    );
    // the subject column: a person or a group
    let c = codes_with_members(|v| rel(v)["field"] = json!("admin"));
    assert_eq!(
        c[0],
        (
            "E_ACCESS_ROWS_SUBJECT",
            "entities[notebook].access.relations[2].field".into()
        )
    );
    let c = codes_with_members(|v| rel(v)["field"] = json!("notebook_id"));
    assert_eq!(c[0].0, "E_ACCESS_ROWS_SUBJECT");
    // the filter tests the junction's columns
    let c = codes_with_members(|v| {
        v["entities"][0]["access"]["relations"][4]["rows"]["filter"][0]["field"] = json!("owner_id")
    });
    assert_eq!(
        c[0],
        (
            "E_ACCESS_WHEN_FIELD",
            "entities[notebook].access.relations[4].rows.filter[0]".into()
        )
    );
    let c = codes_with_members(|v| {
        v["entities"][0]["access"]["relations"][4]["rows"]["filter"][0]["value"] = json!("yes")
    });
    assert_eq!(c[0].0, "E_ACCESS_WHEN_FIELD");
    // only a rows relation has a junction
    let c = codes_with_members(|v| {
        v["entities"][0]["access"]["relations"][0]["rows"] =
            json!({ "entity": "member", "target": "notebook_id", "filter": [] })
    });
    assert_eq!(c[0].0, "E_ACCESS_ROWS_TARGET");
}

#[test]
fn a_rows_relation_is_not_followed_by_via_nor_exported() {
    let c = codes_with_members(|v| {
        v["entities"][0]["access"]["read"] = json!({ "via": { "rel": "member", "perm": "read" } })
    });
    assert_eq!(c[0].0, "E_ACCESS_VIA_TARGET");
    let c = codes_with_members(|v| v["entities"][0]["access"]["export"] = json!(["update"]));
    assert_eq!(
        c[0],
        (
            "E_ACCESS_EXPORT_NOT_ROW_LOCAL",
            "entities[notebook].access.export[0]".into()
        )
    );
    // the subject column's own name on the target is no relation to a row (the notebook has no user_id)
    let c = codes_with_members(|v| {
        v["entities"][0]["access"]["read"] = json!({ "when": { "grant": { "rel": "member" },
            "filter": [{ "field": "owner_id", "op": "is_not_null" }] } })
    });
    assert!(
        c.is_empty(),
        "when on a rows relation tests the target's row: {c:?}"
    );
}
